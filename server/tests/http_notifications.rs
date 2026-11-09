//! Integration: notification channels and rules, end to end against small fake servers (webhook, ntfy, Gotify,
//! SMTP) on localhost.
mod support;

use actix_web::test::TestRequest;
use actix_web::web;
use croustypatch_server::audit::{self, Event};
use croustypatch_server::notify::{self, Channel, Message};
use croustypatch_server::schema::{host_group_members, host_groups, hosts, notifications};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

/// One captured HTTP request.
#[derive(Debug, Clone)]
struct Captured {
    head: String,
    body: Vec<u8>,
}
impl Captured {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()))
    }
}

/// HTTP server answering `status` to every request and handing each request to the test.
async fn http_sink(status: u16) -> (String, tokio::sync::mpsc::UnboundedReceiver<Captured>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut buf = vec![];
                let mut tmp = [0u8; 4096];
                let (head, mut body) = loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 { return; }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break (String::from_utf8_lossy(&buf[..p]).to_string(), buf[p + 4..].to_vec());
                    }
                };
                let len: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
                while body.len() < len {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 { break; }
                    body.extend_from_slice(&tmp[..n]);
                }
                let _ = tx.send(Captured { head, body });
                let _ = s.write_all(format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await;
            });
        }
    });
    (url, rx)
}

/// Minimal SMTP server (no TLS, no auth): records MAIL FROM, RCPT TO and the DATA of each message.
async fn smtp_sink() -> (u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((s, _)) = l.accept().await else { return };
            let tx = tx.clone();
            tokio::spawn(async move {
                let (r, mut w) = s.into_split();
                let mut r = BufReader::new(r);
                let mut log = String::new();
                w.write_all(b"220 sink ESMTP\r\n").await.unwrap();
                let mut line = String::new();
                let mut in_data = false;
                loop {
                    line.clear();
                    if r.read_line(&mut line).await.unwrap_or(0) == 0 { break; }
                    if in_data {
                        if line == ".\r\n" { in_data = false; w.write_all(b"250 queued\r\n").await.unwrap(); } else { log.push_str(&line); }
                        continue;
                    }
                    let up = line.to_ascii_uppercase();
                    if up.starts_with("EHLO") || up.starts_with("HELO") { w.write_all(b"250-sink\r\n250 8BITMIME\r\n").await.unwrap(); }
                    else if up.starts_with("MAIL") || up.starts_with("RCPT") { log.push_str(&line); w.write_all(b"250 ok\r\n").await.unwrap(); }
                    else if up.starts_with("DATA") { in_data = true; w.write_all(b"354 go\r\n").await.unwrap(); }
                    else if up.starts_with("QUIT") { w.write_all(b"221 bye\r\n").await.unwrap(); break; }
                    else { w.write_all(b"250 ok\r\n").await.unwrap(); }
                }
                let _ = tx.send(log);
            });
        }
    });
    (port, rx)
}

async fn recv<T>(rx: &mut tokio::sync::mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.expect("nothing received in time").unwrap()
}

fn add_host(c: &mut PgConnection, name: &str) -> Uuid {
    diesel::insert_into(hosts::table).values((hosts::hostname.eq(name), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(c).unwrap()
}

#[actix_web::test]
async fn channels_validate_their_settings_and_never_return_secrets() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let post = |b: Value| TestRequest::post().uri("/api/v1/notifications/channels").cookie(ck.clone()).set_json(b);
    for bad in [
        json!({"name": "a", "kind": "webhook", "config": {"url": "javascript:alert(1)"}}),
        json!({"name": "b", "kind": "ntfy", "config": {"url": "https://ntfy.sh", "topic": "../admin"}}),
        json!({"name": "c", "kind": "smtp", "config": {"host": "mail", "from": "not-an-address"}}),
        json!({"name": "d", "kind": "sms", "config": {}}),
    ] {
        let (st, body) = call(&app, post(bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {body}");
    }
    let (st, ch) = call(&app, post(json!({"name": "ops hook", "kind": "webhook", "config": {"url": "https://hooks.example.org/x", "extra": "dropped"}, "secret": "s3cret"}))).await;
    assert_eq!(st, 201, "{ch}");
    assert_eq!(ch["has_secret"], json!(true));
    assert!(ch.get("secret").is_none() && !ch.to_string().contains("s3cret"), "the secret never comes back: {ch}");
    assert_eq!(ch["config"], json!({"url": "https://hooks.example.org/x"}), "unknown settings are dropped");
    let stored: String = croustypatch_server::schema::notification_channels::table.select(croustypatch_server::schema::notification_channels::secret).first(&mut db.conn()).unwrap();
    assert!(!stored.contains("s3cret") && !stored.is_empty(), "stored sealed, not in clear");

    // updating without "secret" keeps it
    let id = ch["id"].as_str().unwrap();
    let (st, ch2) = call(&app, TestRequest::put().uri(&format!("/api/v1/notifications/channels/{id}")).cookie(ck.clone()).set_json(json!({"name": "ops hook", "kind": "webhook", "config": {"url": "https://hooks.example.org/y"}}))).await;
    assert_eq!((st, ch2["has_secret"].clone()), (actix_web::http::StatusCode::OK, json!(true)));
}

#[actix_web::test]
async fn a_quarantine_reaches_a_signed_webhook_and_failures_are_retried() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let (url, mut rx) = http_sink(200).await;
    let (_, ch) = call(&app, TestRequest::post().uri("/api/v1/notifications/channels").cookie(ck.clone()).set_json(json!({"name": "siem", "kind": "webhook", "config": {"url": format!("{url}/hook")}, "secret": "k3y"}))).await;
    let (_, dead) = call(&app, TestRequest::post().uri("/api/v1/notifications/channels").cookie(ck.clone()).set_json(json!({"name": "dead", "kind": "webhook", "config": {"url": "http://127.0.0.1:9/hook"}}))).await;
    let (st, rule) = call(&app, TestRequest::post().uri("/api/v1/notifications/rules").cookie(ck.clone()).set_json(json!({"name": "critical", "events": ["host.quarantined", "task.failed"], "min_severity": "critical", "channel_ids": [ch["id"], dead["id"]]}))).await;
    assert_eq!(st, 201, "{rule}");

    let host = add_host(&mut db.conn(), "web-01");
    {
        let mut c = db.conn();
        audit::record(&mut c, Event::new("agent", "host.quarantined").host(host).details(json!({"message": "rollback failed"}))).unwrap();
        // high < critical: filtered out by the rule
        audit::record(&mut c, Event::new("agent", "task.failed").host(host).details(json!({"kind": "patch"}))).unwrap();
    }
    let queued: i64 = notifications::table.count().get_result(&mut db.conn()).unwrap();
    assert_eq!(queued, 2, "one event x two channels; the high-severity failure is below the rule's threshold");

    notify::run_once(&state).await.unwrap();
    let got = recv(&mut rx).await;
    let body: Value = serde_json::from_slice(&got.body).unwrap();
    assert_eq!(body["event"], json!("host.quarantined"));
    assert!(body["title"].as_str().unwrap().contains("web-01"));
    let ts: i64 = got.header("x-croustypatch-timestamp").unwrap().parse().unwrap();
    assert_eq!(got.header("x-croustypatch-signature").unwrap(), format!("sha256={}", notify::webhook_signature(b"k3y", ts, &got.body)), "HMAC-SHA256 over timestamp.body");

    let rows: Vec<(String, i32, String)> = notifications::table.order(notifications::status.asc()).select((notifications::status, notifications::attempts, notifications::last_error)).load(&mut db.conn()).unwrap();
    assert_eq!(rows[0].0, "pending", "the unreachable channel is retried later: {rows:?}");
    assert_eq!(rows[0].1, 1);
    assert!(!rows[0].2.is_empty());
    assert_eq!(rows[1].0, "sent");
}

#[actix_web::test]
async fn rules_scoped_to_a_group_ignore_other_hosts_and_digests_bundle_events() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let (url, mut rx) = http_sink(200).await;
    let (inside, outside, group) = {
        let mut c = db.conn();
        let g: Uuid = diesel::insert_into(host_groups::table).values(host_groups::name.eq("prod")).returning(host_groups::id).get_result(&mut c).unwrap();
        let a = add_host(&mut c, "in-1");
        let b = add_host(&mut c, "out-1");
        diesel::insert_into(host_group_members::table).values((host_group_members::host_id.eq(a), host_group_members::group_id.eq(g))).execute(&mut c).unwrap();
        (a, b, g)
    };
    let (_, ch) = call(&app, TestRequest::post().uri("/api/v1/notifications/channels").cookie(ck.clone()).set_json(json!({"name": "hook", "kind": "webhook", "config": {"url": url}}))).await;
    let (_, rule) = call(&app, TestRequest::post().uri("/api/v1/notifications/rules").cookie(ck.clone()).set_json(json!({"name": "daily", "events": ["task.succeeded", "task.failed"], "min_severity": "info", "frequency": "digest", "digest_hour": 8, "group_ids": [group], "channel_ids": [ch["id"]]}))).await;
    {
        let mut c = db.conn();
        audit::record(&mut c, Event::new("agent", "task.succeeded").host(inside).details(json!({"kind": "patch"}))).unwrap();
        audit::record(&mut c, Event::new("agent", "task.failed").host(inside).details(json!({"kind": "patch"}))).unwrap();
        audit::record(&mut c, Event::new("agent", "task.failed").host(outside).details(json!({"kind": "patch"}))).unwrap();
        audit::record(&mut c, Event::new("agent", "task.succeeded").host(inside).details(json!({"kind": "scan"}))).unwrap();
    }
    let digest: i64 = notifications::table.filter(notifications::status.eq("digest")).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(digest, 2, "only the in-scope patch events wait for the digest (scans are routine)");
    assert_eq!(notify::run_once(&state).await.unwrap() > 0, false, "nothing is sent before the digest hour");

    let (st, r) = call(&app, TestRequest::post().uri(&format!("/api/v1/notifications/rules/{}/digest", rule["id"].as_str().unwrap())).cookie(ck.clone())).await;
    assert_eq!((st.as_u16(), r["sent"].clone()), (200, json!(1)));
    let got: Value = serde_json::from_slice(&recv(&mut rx).await.body).unwrap();
    assert!(got["title"].as_str().unwrap().contains("2 event(s)"), "{got}");
    assert_eq!(got["severity"], json!("high"), "a digest carries its worst severity");
    let left: i64 = notifications::table.filter(notifications::status.eq("digest")).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn ntfy_gotify_and_smtp_messages_are_formed_correctly() {
    let m = Message { id: Uuid::new_v4(), event: "reboot.overdue".into(), severity: "critical".into(), title: "[CroustyPatch] Critical · Hôte · web-01".into(), body: "line 1\nline 2".into(), payload: json!({}), recipients: vec![] };

    let (url, mut rx) = http_sink(200).await;
    let ntfy = Channel { kind: "ntfy".into(), config: json!({"url": url, "topic": "ops"}), secret: zeroize::Zeroizing::new("tk".into()) };
    notify::send(&ntfy, &m).await.unwrap();
    let r = recv(&mut rx).await;
    assert!(r.head.starts_with("POST /ops "), "{}", r.head);
    assert_eq!(r.header("priority").as_deref(), Some("5"));
    assert_eq!(r.header("authorization").as_deref(), Some("Bearer tk"));
    assert!(r.header("title").unwrap().is_ascii());
    assert_eq!(r.body, b"line 1\nline 2");

    let gotify = Channel { kind: "gotify".into(), config: json!({"url": url}), secret: zeroize::Zeroizing::new("apptoken".into()) };
    notify::send(&gotify, &m).await.unwrap();
    let r = recv(&mut rx).await;
    assert!(r.head.starts_with("POST /message "));
    assert_eq!(r.header("x-gotify-key").as_deref(), Some("apptoken"));
    let v: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["priority"], json!(10));

    let (port, mut mails) = smtp_sink().await;
    let smtp = Channel { kind: "smtp".into(), config: json!({"host": "127.0.0.1", "port": port, "tls": "none", "from": "patch@example.org", "to": ["ops@example.org"]}), secret: zeroize::Zeroizing::new(String::new()) };
    notify::send(&smtp, &m).await.unwrap();
    let log = recv(&mut mails).await;
    assert!(log.contains("RCPT TO:<ops@example.org>"), "{log}");
    assert!(log.contains("line 1"), "{log}");

    // an HTTP error is a delivery failure
    let (url, _rx) = http_sink(500).await;
    let broken = Channel { kind: "gotify".into(), config: json!({"url": url}), secret: zeroize::Zeroizing::new("x".into()) };
    assert!(notify::send(&broken, &m).await.is_err());
}
