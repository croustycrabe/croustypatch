//! Integration: reports (scheduled summaries), mail recipients and the profile e-mail.
mod support;

use actix_web::test::TestRequest;
use actix_web::web;
use chrono::{DateTime, Duration, Utc};
use croustypatch_server::reports::{self, Report};
use croustypatch_server::schema::{host_packages, hosts, report_runs, reports as reports_t, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
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


fn report(frequency: &str, tz: &str) -> Report {
    Report {
        id: Uuid::nil(), name: "r".into(), enabled: true, frequency: frequency.into(), weekday: 0, day_of_month: 1, send_at: "08:00".into(), timezone: tz.into(), language: "fr".into(),
        sections: json!({}), group_ids: json!([]), emails: json!([]), channel_ids: json!([]), skip_if_empty: false, last_run_at: None, last_slot_at: None, attempts: 0,
        last_status: String::new(), last_error: String::new(), created_at: at("2026-01-01T00:00:00Z"), updated_at: at("2026-01-01T00:00:00Z"),
    }
}

#[test]
fn schedules_follow_the_local_time_zone() {
    // 08:00 in Paris is 07:00 UTC in winter and 06:00 UTC in summer
    let d = report("daily", "Europe/Paris");
    assert_eq!(reports::latest_slot(&d, at("2026-11-11T12:00:00Z")), Some(at("2026-11-11T07:00:00Z")));
    assert_eq!(reports::latest_slot(&d, at("2026-11-11T06:59:00Z")), Some(at("2026-11-10T07:00:00Z")), "not yet today");
    assert_eq!(reports::latest_slot(&d, at("2026-07-01T12:00:00Z")), Some(at("2026-07-01T06:00:00Z")));
    assert_eq!(reports::next_slot(&d, at("2026-11-11T12:00:00Z")), Some(at("2026-11-12T07:00:00Z")));
    // weekly on Monday (2026-11-09 is a Monday), monthly on the 1st
    let w = report("weekly", "UTC");
    assert_eq!(reports::latest_slot(&w, at("2026-11-11T12:00:00Z")), Some(at("2026-11-09T08:00:00Z")));
    assert_eq!(reports::next_slot(&w, at("2026-11-11T12:00:00Z")), Some(at("2026-11-16T08:00:00Z")));
    let m = report("monthly", "UTC");
    assert_eq!(reports::latest_slot(&m, at("2026-11-11T12:00:00Z")), Some(at("2026-11-01T08:00:00Z")));
    assert_eq!(reports::next_slot(&m, at("2026-11-11T12:00:00Z")), Some(at("2026-12-01T08:00:00Z")));
    // the clocks go forward on 2027-03-28 at 02:00 in Paris: a 02:30 report is sent just after the gap
    let mut gap = report("daily", "Europe/Paris");
    gap.send_at = "02:30".into();
    assert_eq!(reports::latest_slot(&gap, at("2027-03-28T12:00:00Z")), Some(at("2027-03-28T01:30:00Z")));
}

#[test]
fn a_report_is_due_once_per_slot_and_a_failed_send_is_retried_a_few_times() {
    let now = at("2026-11-11T12:00:00Z");
    let mut r = report("daily", "UTC");
    assert!(reports::is_due(&r, now), "a new slot");
    // a report created after today's slot does not fire for it
    r.created_at = at("2026-11-11T09:00:00Z");
    assert!(!reports::is_due(&r, now), "created at 09:00: this morning's 08:00 is not for it");
    assert!(reports::is_due(&r, at("2026-11-12T08:01:00Z")), "tomorrow's is");
    // handled
    r.created_at = at("2026-01-01T00:00:00Z");
    r.last_slot_at = Some(at("2026-11-11T08:00:00Z"));
    r.last_status = "sent".into();
    r.last_run_at = Some(at("2026-11-11T08:00:05Z"));
    r.attempts = 1;
    assert!(!reports::is_due(&r, now));
    // failed: tried again after 10 minutes, then 20, and then given up
    r.last_status = "failed".into();
    r.last_run_at = Some(now - Duration::minutes(5));
    assert!(!reports::is_due(&r, now), "too soon");
    r.last_run_at = Some(now - Duration::minutes(11));
    assert!(reports::is_due(&r, now));
    r.attempts = 3;
    assert!(!reports::is_due(&r, now), "three tries are enough: the failure stays visible");
    // a new day starts afresh
    assert!(reports::is_due(&r, at("2026-11-12T08:01:00Z")));
    r.enabled = false;
    assert!(!reports::is_due(&r, at("2026-11-12T08:01:00Z")));
}

async fn smtp_channel(app: &impl actix_web::dev::Service<actix_http::Request, Response = actix_web::dev::ServiceResponse, Error = actix_web::Error>, ck: &actix_web::cookie::Cookie<'static>, port: u16) -> String {
    let (st, ch) = call(app, TestRequest::post().uri("/api/v1/notifications/channels").cookie(ck.clone()).set_json(json!({"name": "mail", "kind": "smtp", "config": {"host": "127.0.0.1", "port": port, "tls": "none", "from": "patch@example.org"}}))).await;
    assert_eq!(st, 201, "{ch}");
    assert!(ch["config"]["to"].as_array().unwrap().is_empty(), "no default recipient is asked for");
    ch["id"].as_str().unwrap().to_string()
}

#[actix_web::test]
async fn a_report_is_sent_even_when_nothing_happened_and_says_so() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (port, mut mails) = smtp_sink().await;
    let ch = smtp_channel(&app, &ck, port).await;
    diesel::insert_into(hosts::table).values((hosts::hostname.eq("web-01"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).execute(&mut db.conn()).unwrap();
    let (st, rep) = call(&app, TestRequest::post().uri("/api/v1/reports").cookie(ck.clone()).set_json(json!({"name": "Weekly", "frequency": "weekly", "channel_ids": [ch], "emails": ["boss@example.org"], "timezone": "Europe/Paris"}))).await;
    assert_eq!(st, 201, "{rep}");
    assert!(rep["next_run"].is_string());
    let id = rep["id"].as_str().unwrap();

    let (st, r) = call(&app, TestRequest::post().uri(&format!("/api/v1/reports/{id}/send")).cookie(ck.clone())).await;
    assert_eq!((st.as_u16(), r["status"].clone()), (200, json!("sent")), "{r}");
    let mail = recv(&mut mails).await;
    assert!(mail.contains("RCPT TO:<boss@example.org>"), "{mail}");
    assert!(mail.contains("Tout est à jour") || mail.contains("Tout est =C3=A0 jour") || mail.contains("jour."), "the report says everything is current: {mail}");
    assert!(mail.to_lowercase().contains("text/html") && mail.to_lowercase().contains("text/plain"), "both a plain and a formatted version");
    assert!(mail.contains("Subject: [CroustyPatch] Weekly"), "{mail}");

    // skip_if_empty only applies to the schedule, never to "send now"
    let (_, _) = call(&app, TestRequest::put().uri(&format!("/api/v1/reports/{id}")).cookie(ck.clone()).set_json(json!({"name": "Weekly", "frequency": "weekly", "channel_ids": [ch], "emails": ["boss@example.org"], "skip_if_empty": true}))).await;
    let state = web::Data::new(db.state());
    let o = reports::send(&state, id.parse().unwrap(), "schedule", None).await.unwrap();
    assert_eq!(o.status, "skipped", "nothing to report and the report says to skip");
    let (_, r) = call(&app, TestRequest::post().uri(&format!("/api/v1/reports/{id}/send")).cookie(ck.clone())).await;
    assert_eq!(r["status"], json!("sent"));
    let (_, runs) = call(&app, TestRequest::get().uri(&format!("/api/v1/reports/{id}/runs")).cookie(ck.clone())).await;
    let st: Vec<&str> = runs.as_array().unwrap().iter().map(|r| r["status"].as_str().unwrap()).collect();
    assert_eq!(st, vec!["sent", "skipped", "sent"], "every attempt is logged, newest first");
}

#[actix_web::test]
async fn the_report_lists_what_is_waiting_and_what_was_patched() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (a, b) = {
        let mut c = db.conn();
        let mk = |c: &mut PgConnection, n: &str| -> Uuid { diesel::insert_into(hosts::table).values((hosts::hostname.eq(n), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(c).unwrap() };
        (mk(&mut c, "docker-service"), mk(&mut c, "docker-data"))
    };
    {
        let mut c = db.conn();
        for (h, n, sec) in [(a, "snapd", false), (a, "openssl", true), (b, "snapd", false)] {
            diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(h), host_packages::name.eq(n), host_packages::version.eq("1"), host_packages::candidate.eq(Some("2")), host_packages::is_security.eq(sec))).execute(&mut c).unwrap();
        }
        diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(a), host_packages::name.eq("docker:hoppscotch-db"), host_packages::source.eq("docker"), host_packages::version.eq("postgres:17 @3540"), host_packages::candidate.eq(Some("postgres:17 @e5d2")))).execute(&mut c).unwrap();
        let now = Utc::now();
        for (i, st) in ["succeeded", "succeeded", "failed", "succeeded"].iter().enumerate() {
            diesel::insert_into(tasks::table)
                .values((tasks::host_id.eq(a), tasks::kind.eq("patch"), tasks::status.eq(*st), tasks::message.eq(format!("run {i}")), tasks::finished_at.eq(Some(now - Duration::hours(i as i64 + 1))), tasks::packages.eq(json!([]))))
                .execute(&mut c)
                .unwrap();
        }
        diesel::update(hosts::table.find(b)).set(hosts::reboot_required.eq(true)).execute(&mut c).unwrap();
    }
    let body = |history_mode: &str, value: u32| json!({"name": "R", "language": "en", "sections": {"summary": true, "pending": true, "history": true, "history_mode": history_mode, "history_value": value, "failures": true, "reboots": true, "patch_days": false, "docker": true}});
    let (st, p) = call(&app, TestRequest::post().uri("/api/v1/reports/preview").cookie(ck.clone()).set_json(body("count", 2))).await;
    assert_eq!(st, 200, "{p}");
    let text = p["text"].as_str().unwrap();
    assert!(text.contains("docker-service") && text.contains("docker-data"), "{text}");
    assert!(text.contains("openssl"), "the security update is named");
    assert!(text.contains("Updates waiting"), "{text}");
    assert!(text.contains("Machines to reboot") && text.contains("docker-data"), "{text}");
    assert!(text.contains("hoppscotch-db") && text.contains("e5d2"), "container images are listed when asked for: {text}");
    assert_eq!(p["empty"], json!(false));
    let history = text.split("Patch history").nth(1).unwrap().split("Failures and problems").next().unwrap();
    assert_eq!(history.matches("docker-service").count(), 2, "the last 2 patches only: {history}");
    let (_, p) = call(&app, TestRequest::post().uri("/api/v1/reports/preview").cookie(ck.clone()).set_json(body("days", 30))).await;
    let history = p["text"].as_str().unwrap().split("Patch history").nth(1).unwrap().split("Failures and problems").next().unwrap().to_string();
    assert_eq!(history.matches("docker-service").count(), 4, "the last 30 days: {history}");
    assert!(p["text"].as_str().unwrap().split("Failures and problems").nth(1).unwrap().contains("failed"), "the failure is listed");
    assert!(!p["html"].as_str().unwrap().contains("<script"), "the formatted version is plain markup");

    // one group only
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/reports/preview").cookie(ck.clone()).set_json(json!({"name": "R", "frequency": "yearly"}))).await;
    assert_eq!(st, 400);
    for bad in [json!({"name": "R", "send_at": "25:00"}), json!({"name": "R", "timezone": "Mars/Olympus"}), json!({"name": "R", "day_of_month": 31}), json!({"name": "R", "emails": ["nope"]}), json!({"name": "R", "sections": {"history_mode": "weeks"}}), json!({"name": "R", "sections": {"history_value": 0}})] {
        let (st, b) = call(&app, TestRequest::post().uri("/api/v1/reports").cookie(ck.clone()).set_json(bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
}

#[actix_web::test]
async fn the_worker_sends_due_reports_once_and_retries_failures() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let (port, mut mails) = smtp_sink().await;
    let good = smtp_channel(&app, &ck, port).await;
    let (st, rep) = call(&app, TestRequest::post().uri("/api/v1/reports").cookie(ck.clone()).set_json(json!({"name": "Daily", "frequency": "daily", "send_at": "00:00", "timezone": "UTC", "channel_ids": [good], "emails": ["ops@example.org"]}))).await;
    assert_eq!(st, 201, "{rep}");
    let id: Uuid = rep["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(reports::run_due(&state).await.unwrap(), 0, "created today, after today's 00:00: nothing is due yet");
    diesel::update(reports_t::table.find(id)).set(reports_t::created_at.eq(Utc::now() - Duration::days(2))).execute(&mut db.conn()).unwrap();
    assert_eq!(reports::run_due(&state).await.unwrap(), 1);
    assert!(recv(&mut mails).await.contains("RCPT TO:<ops@example.org>"));
    assert_eq!(reports::run_due(&state).await.unwrap(), 0, "the slot is handled: not sent twice");

    // a dead provider: the failure is recorded with its reason, retried later, and finally left visible
    let (st, dead) = call(&app, TestRequest::post().uri("/api/v1/notifications/channels").cookie(ck.clone()).set_json(json!({"name": "dead", "kind": "smtp", "config": {"host": "127.0.0.1", "port": 9, "tls": "none", "from": "patch@example.org"}}))).await;
    assert_eq!(st, 201);
    let (_, rep2) = call(&app, TestRequest::post().uri("/api/v1/reports").cookie(ck.clone()).set_json(json!({"name": "Broken", "frequency": "daily", "send_at": "00:00", "timezone": "UTC", "channel_ids": [dead["id"]], "emails": ["ops@example.org"]}))).await;
    let id2: Uuid = rep2["id"].as_str().unwrap().parse().unwrap();
    diesel::update(reports_t::table.find(id2)).set(reports_t::created_at.eq(Utc::now() - Duration::days(2))).execute(&mut db.conn()).unwrap();
    assert_eq!(reports::run_due(&state).await.unwrap(), 1);
    let (status, err, attempts): (String, String, i32) = reports_t::table.find(id2).select((reports_t::last_status, reports_t::last_error, reports_t::attempts)).first(&mut db.conn()).unwrap();
    assert_eq!((status.as_str(), attempts), ("failed", 1));
    assert!(err.contains("smtp"), "the error says what failed: {err}");
    assert_eq!(reports::run_due(&state).await.unwrap(), 0, "retried after a pause, not at once");
    for expected in [2, 3] {
        diesel::update(reports_t::table.find(id2)).set(reports_t::last_run_at.eq(Some(Utc::now() - Duration::minutes(45)))).execute(&mut db.conn()).unwrap();
        assert_eq!(reports::run_due(&state).await.unwrap(), 1);
        let a: i32 = reports_t::table.find(id2).select(reports_t::attempts).first(&mut db.conn()).unwrap();
        assert_eq!(a, expected);
    }
    diesel::update(reports_t::table.find(id2)).set(reports_t::last_run_at.eq(Some(Utc::now() - Duration::minutes(90)))).execute(&mut db.conn()).unwrap();
    assert_eq!(reports::run_due(&state).await.unwrap(), 0, "three tries, then it stays failed");
    let runs: i64 = report_runs::table.filter(report_runs::report_id.eq(id2)).filter(report_runs::status.eq("failed")).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(runs, 3);
    let audited: i64 = croustypatch_server::schema::audit_log::table.filter(croustypatch_server::schema::audit_log::action.eq("report.failed")).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(audited, 3, "each failure can reach a notification rule");
}

#[actix_web::test]
async fn a_report_with_no_recipient_or_channel_fails_with_a_clear_reason() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (port, _mails) = smtp_sink().await;
    let ch = smtp_channel(&app, &ck, port).await;
    let (_, none) = call(&app, TestRequest::post().uri("/api/v1/reports").cookie(ck.clone()).set_json(json!({"name": "A"}))).await;
    let (_, r) = call(&app, TestRequest::post().uri(&format!("/api/v1/reports/{}/send", none["id"].as_str().unwrap())).cookie(ck.clone())).await;
    assert_eq!(r["status"], json!("failed"));
    assert!(r["error"].as_str().unwrap().contains("no channel"), "{r}");
    let (_, norcpt) = call(&app, TestRequest::post().uri("/api/v1/reports").cookie(ck.clone()).set_json(json!({"name": "B", "channel_ids": [ch]}))).await;
    let (_, r) = call(&app, TestRequest::post().uri(&format!("/api/v1/reports/{}/send", norcpt["id"].as_str().unwrap())).cookie(ck.clone())).await;
    assert!(r["error"].as_str().unwrap().contains("no recipient"), "{r}");
}

#[actix_web::test]
async fn the_mail_test_asks_who_to_send_to_and_the_profile_email_is_the_default() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (port, mut mails) = smtp_sink().await;
    let ch = smtp_channel(&app, &ck, port).await;
    let test = |body: Value| TestRequest::post().uri(&format!("/api/v1/notifications/channels/{ch}/test")).cookie(ck.clone()).set_json(body);
    let (st, r) = call(&app, test(json!({}))).await;
    assert_eq!(st, 400, "no address anywhere: say so instead of failing silently: {r}");
    let (st, _) = call(&app, test(json!({"to": "not an address"}))).await;
    assert_eq!(st, 400);
    let (st, r) = call(&app, test(json!({"to": "me@example.org"}))).await;
    assert_eq!((st.as_u16(), r["ok"].clone()), (200, json!(true)), "{r}");
    assert!(recv(&mut mails).await.contains("RCPT TO:<me@example.org>"));

    // the profile e-mail: validated, audited, returned by /auth/me and used as the default target
    for bad in ["nope", "a@b@c", "x <y@z>"] {
        let (st, _) = call(&app, TestRequest::patch().uri("/api/v1/auth/profile").cookie(ck.clone()).set_json(json!({"email": bad}))).await;
        assert_eq!(st, 400, "{bad}");
    }
    let (st, _) = call(&app, TestRequest::patch().uri("/api/v1/auth/profile").cookie(ck.clone()).set_json(json!({"email": " admin@example.org "}))).await;
    assert_eq!(st, 204);
    let (_, me) = call(&app, TestRequest::get().uri("/api/v1/auth/me").cookie(ck.clone())).await;
    assert_eq!(me["email"], json!("admin@example.org"));
    let (st, r) = call(&app, test(json!({}))).await;
    assert_eq!((st.as_u16(), r["ok"].clone()), (200, json!(true)));
    assert!(recv(&mut mails).await.contains("RCPT TO:<admin@example.org>"), "the profile address is the default");
    let (_, _) = call(&app, TestRequest::patch().uri("/api/v1/auth/profile").cookie(ck.clone()).set_json(json!({"email": ""}))).await;
    let (_, me) = call(&app, TestRequest::get().uri("/api/v1/auth/me").cookie(ck.clone())).await;
    assert_eq!(me["email"], json!(null), "an empty address clears it");
}
