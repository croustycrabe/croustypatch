//! The terminal app center: state and keys, rendering, and the agent's socket relay end to end.
use crossterm::event::{KeyCode, KeyModifiers};
use croustypatch_agent::config::Config;
use croustypatch_agent::identity::Identity;
use croustypatch_agent::portal::{self, Relay, Request};
use croustypatch_agent::tui::{render, Cmd, Presence, State, Tone, EN, FR};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn catalog() -> Value {
    json!({ "host": { "hostname": "srv-01" }, "apps": [
        { "id": "a", "name": "htop", "version": "3.3", "category": "Outils", "description": "Process viewer", "packages": ["htop"], "installed": true, "pending": null },
        { "id": "b", "name": "Dev tools", "version": "", "category": "Développement", "description": "git and make", "packages": ["git", "make"], "installed": false, "pending": null },
        { "id": "c", "name": "ncdu", "version": "2", "category": "Outils", "description": "", "packages": ["ncdu"], "installed": false, "pending": { "install": true, "status": "running" } },
    ]})
}
const NONE: KeyModifiers = KeyModifiers::NONE;

#[test]
fn keys_navigate_and_installs_need_a_confirmation() {
    let mut s = State::new(&EN);
    s.load(&catalog());
    assert_eq!(s.categories(), vec!["All", "Développement", "Outils"]);
    assert_eq!(s.selected().unwrap().name, "htop");
    assert!(s.busy(), "ncdu is being installed");
    assert_eq!(s.on_key(KeyCode::Char('i'), NONE), None, "htop is installed: nothing to install");
    assert!(s.confirm.is_none());
    s.on_key(KeyCode::Down, NONE);
    assert_eq!(s.selected().unwrap().name, "Dev tools");
    assert_eq!(s.on_key(KeyCode::Char('i'), NONE), None, "first a confirmation");
    assert!(s.confirm.is_some());
    assert_eq!(s.on_key(KeyCode::Char('n'), NONE), None, "n cancels");
    s.on_key(KeyCode::Char('i'), NONE);
    assert_eq!(s.on_key(KeyCode::Char('y'), NONE), Some(Cmd::Install("b".into())));
    s.on_key(KeyCode::Down, NONE);
    assert_eq!(s.on_key(KeyCode::Char('i'), NONE), None);
    assert!(s.confirm.is_none(), "an app with a request in progress is left alone");
    s.on_key(KeyCode::Right, NONE);
    s.on_key(KeyCode::Right, NONE);
    assert_eq!(s.visible().len(), 2, "category Outils");
    assert_eq!(s.on_key(KeyCode::Char('u'), NONE), None);
    assert_eq!(s.on_key(KeyCode::Enter, NONE), Some(Cmd::Uninstall("a".into())));
    assert_eq!(s.on_key(KeyCode::Char('q'), NONE), Some(Cmd::Quit));
    assert_eq!(s.on_key(KeyCode::Char('c'), KeyModifiers::CONTROL), Some(Cmd::Quit));
}

#[test]
fn a_refresh_keeps_the_selected_app() {
    let mut s = State::new(&EN);
    s.load(&catalog());
    s.on_key(KeyCode::Down, NONE);
    let mut c = catalog();
    c["apps"].as_array_mut().unwrap().insert(0, json!({ "id": "z", "name": "aaa", "category": "Outils", "packages": ["aaa"], "installed": false }));
    s.load(&c);
    assert_eq!(s.selected().unwrap().name, "Dev tools");
}

#[test]
fn the_screen_shows_the_brand_the_apps_and_their_state() {
    let mut s = State::new(&FR);
    s.load(&catalog());
    let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
    t.draw(|f| render(f, &s)).unwrap();
    let text: String = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    for want in ["[ CroustyPatch ]", "Portail applicatif", "srv-01", "htop", "Dev tools", "● Présent", "○ Disponible", "Installation…", "Process viewer", "q quitter", "État et notifications", "Installation ncdu", "applications déjà présentes"] {
        assert!(text.contains(want), "missing {want:?}");
    }
    s.on_key(KeyCode::Down, NONE);
    s.on_key(KeyCode::Char('i'), NONE);
    t.draw(|f| render(f, &s)).unwrap();
    let text: String = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("Installer Dev tools ?") && text.contains("o / n"));
}

#[test]
fn local_accounts_and_groups_are_resolved() {
    let passwd = "root:x:0:0::/root:/bin/sh\nbob:x:1000:1000::/home/bob:/bin/bash\n";
    assert_eq!(portal::user_name(passwd, 1000), "bob");
    assert_eq!(portal::user_name(passwd, 4242), "uid4242");
    assert_eq!(portal::group_id("devs", "root:x:0:\ndevs:x:1500:bob\n"), Some(1500));
    assert_eq!(serde_json::to_value(Request::Install { app_id: "b".into() }).unwrap(), json!({ "op": "install", "app_id": "b" }));
}

/// Fake server for the relay: answers the two agent endpoints and returns what it received.
async fn fake_server() -> (String, tokio::sync::mpsc::UnboundedReceiver<(String, String, String)>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut buf = vec![];
                let mut tmp = [0u8; 8192];
                let (head, body) = loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 { return; }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_string();
                        let len: usize = head.to_lowercase().lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
                        while buf.len() < p + 4 + len { let n = s.read(&mut tmp).await.unwrap_or(0); if n == 0 { break; } buf.extend_from_slice(&tmp[..n]); }
                        break (head, String::from_utf8_lossy(&buf[p + 4..]).to_string());
                    }
                };
                let line = head.lines().next().unwrap_or("").to_string();
                let auth = head.lines().find(|l| l.to_lowercase().starts_with("authorization:")).unwrap_or("").to_string();
                let reply = if line.starts_with("GET /api/v1/agent/portal ") { catalog().to_string() } else { json!({ "status": "queued", "task_id": "t1" }).to_string() };
                let _ = tx.send((line, auth, body));
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await;
            });
        }
    });
    (url, rx)
}

#[tokio::test]
async fn the_socket_relays_to_the_server_as_the_agent_and_names_the_local_user() {
    let (url, mut seen) = fake_server().await;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("portal.sock").to_string_lossy().to_string();
    let kp = croustypatch_common::crypto::generate_rsa(2048).unwrap();
    let cfg = Config { server_url: url, portal_socket: sock.clone(), state_dir: dir.path().to_path_buf(), ..Default::default() };
    let relay = Arc::new(Relay { agent: None, cfg, identity: Identity { agent_id: uuid::Uuid::new_v4(), server_public_key_pem: String::new(), private_key_pem: kp.private_pem }, server_ips: Arc::new(|| vec!["127.0.0.1".parse().unwrap()]) });
    tokio::spawn(portal::serve(relay));
    for _ in 0..50 { if std::path::Path::new(&sock).exists() { break; } tokio::time::sleep(std::time::Duration::from_millis(20)).await; }
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777, 0o666, "any local user may ask (the catalog is admin-approved)");

    let r = portal::ask(&sock, &Request::List).await.unwrap();
    assert!(r.ok, "{}", r.error);
    assert_eq!(r.data["apps"][0]["name"], json!("htop"));
    let (line, auth, _) = seen.recv().await.unwrap();
    assert!(line.starts_with("GET /api/v1/agent/portal "), "{line}");
    assert!(auth.contains("Bearer ey"), "the agent's own signed token: {auth}");

    let r = portal::ask(&sock, &Request::Install { app_id: "b".into() }).await.unwrap();
    assert!(r.ok);
    let (line, _, body) = seen.recv().await.unwrap();
    assert!(line.starts_with("POST /api/v1/agent/portal/install "));
    let me = std::env::var("USER").unwrap_or_default();
    let b: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((b["app_id"].clone(), b["action"].clone()), (json!("b"), json!("INSTALL")));
    if !me.is_empty() { assert_eq!(b["user"], json!(me), "the user comes from the socket's peer credentials"); }

    let mut raw = tokio::net::UnixStream::connect(&sock).await.unwrap();
    raw.write_all(b"{\"op\":\"format_disk\"}\n").await.unwrap();
    let mut out = vec![0u8; 256];
    let n = raw.read(&mut out).await.unwrap();
    assert!(String::from_utf8_lossy(&out[..n]).contains("bad request"));
}

#[tokio::test]
async fn updates_are_relayed_as_the_agent_for_the_local_user_and_the_names_are_checked_first() {
    let (url, mut seen) = fake_server().await;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("portal.sock").to_string_lossy().to_string();
    let kp = croustypatch_common::crypto::generate_rsa(2048).unwrap();
    let cfg = Config { server_url: url, portal_socket: sock.clone(), state_dir: dir.path().to_path_buf(), ..Default::default() };
    let relay = Arc::new(Relay { agent: None, cfg, identity: Identity { agent_id: uuid::Uuid::new_v4(), server_public_key_pem: String::new(), private_key_pem: kp.private_pem }, server_ips: Arc::new(|| vec!["127.0.0.1".parse().unwrap()]) });
    tokio::spawn(portal::serve(relay));
    for _ in 0..50 { if std::path::Path::new(&sock).exists() { break; } tokio::time::sleep(std::time::Duration::from_millis(20)).await; }

    let r = portal::ask(&sock, &Request::Updates).await.unwrap();
    assert!(r.ok, "{}", r.error);
    let (line, auth, _) = seen.recv().await.unwrap();
    assert!(line.starts_with("GET /api/v1/agent/portal/updates "), "{line}");
    assert!(auth.contains("Bearer ey"), "as the agent: {auth}");

    let r = portal::ask(&sock, &Request::Update { packages: vec!["vim".into(), "libssl3".into()] }).await.unwrap();
    assert!(r.ok, "{}", r.error);
    let (line, _, body) = seen.recv().await.unwrap();
    assert!(line.starts_with("POST /api/v1/agent/portal/update "), "{line}");
    let b: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(b["packages"], json!(["vim", "libssl3"]));
    let me = std::env::var("USER").unwrap_or_default();
    if !me.is_empty() { assert_eq!(b["user"], json!(me), "the person is named from the socket's peer credentials, never from the request"); }

    // no names = all of them; a list that is not made of package names never leaves the machine
    let r = portal::ask(&sock, &Request::Update { packages: vec![] }).await.unwrap();
    assert!(r.ok);
    let (_, _, body) = seen.recv().await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["packages"], json!([]));
    for bad in [vec!["vim;reboot".to_string()], vec!["-rf".to_string()], vec!["a b".to_string()], (0..1001).map(|i| format!("p{i}")).collect()] {
        let r = portal::ask(&sock, &Request::Update { packages: bad }).await.unwrap();
        assert!(!r.ok && r.error.contains("bad package list"), "{}", r.error);
    }
    // an update check needs the agent itself (the relay has none in this test)
    let r = portal::ask(&sock, &Request::Check).await.unwrap();
    assert!(!r.ok);
    // nothing but the three valid requests reached the server
    assert!(tokio::time::timeout(std::time::Duration::from_millis(200), seen.recv()).await.is_err());

    // the wire format of the new requests
    assert_eq!(serde_json::to_string(&Request::Updates).unwrap(), r#"{"op":"updates"}"#);
    assert_eq!(serde_json::to_string(&Request::Check).unwrap(), r#"{"op":"check"}"#);
    assert_eq!(serde_json::from_str::<Request>(r#"{"op":"update"}"#).unwrap(), Request::Update { packages: vec![] }, "names are optional");
}


fn screen(s: &State) -> String {
    let mut t = Terminal::new(TestBackend::new(110, 28)).unwrap();
    t.draw(|f| render(f, s)).unwrap();
    t.backend().buffer().content().iter().map(|c| c.symbol()).collect()
}

#[test]
fn presence_comes_from_the_local_check_and_shows_partial_installs() {
    let mut v = catalog();
    let installed: std::collections::HashSet<String> = ["htop", "git"].iter().map(|s| s.to_string()).collect();
    portal::annotate_local(&mut v, &installed, |_| false);
    assert_eq!(v["apps"][1]["local"], json!({ "present": 1, "total": 2, "missing": ["make"] }));
    let mut s = State::new(&EN);
    s.load(&v);
    assert_eq!(s.apps[0].presence(), Presence::Present);
    assert_eq!(s.apps[1].presence(), Presence::Partial(1, 2), "git is there, make is not");
    s.on_key(KeyCode::Down, NONE);
    let text = screen(&s);
    assert!(text.contains("◑ Partial 1/2") && text.contains("1 missing"), "partial installs are shown in amber with what is missing");
    // a partial app can be completed or removed
    s.on_key(KeyCode::Char('i'), NONE);
    assert!(s.confirm.is_some());
    s.on_key(KeyCode::Char('n'), NONE);
    s.on_key(KeyCode::Char('u'), NONE);
    assert!(s.confirm.is_some());
    // software installed by hand counts as present even when the server's inventory is older
    let mut by_hand = catalog();
    portal::annotate_local(&mut by_hand, &["htop", "git", "make"].iter().map(|s| s.to_string()).collect(), |_| false);
    let mut s2 = State::new(&EN);
    s2.load(&by_hand);
    assert_eq!(s2.apps[1].presence(), Presence::Present);
}

#[test]
fn the_status_area_follows_an_operation_and_notifies_its_outcome() {
    let mut s = State::new(&EN);
    let mut v = catalog();
    v["recent"] = json!([{ "task_id": "old", "status": "failed", "app_name": "x", "install": true, "message": "boom" }]);
    s.load(&v);
    assert!(s.notes.iter().all(|n| n.tone != Tone::Danger), "failures from before the portal opened are history, not news");
    assert!(s.notes[0].text.contains("1 applications already present") && s.notes[0].text.contains("htop"));
    let (name, install, ratio, stage) = s.progress().unwrap();
    assert_eq!((name.as_str(), install, stage), ("ncdu", true, "running"));
    assert!(ratio > 0.5);

    // ncdu's install succeeds, then Dev tools is requested and fails
    let mut done = catalog();
    done["apps"][2]["pending"] = Value::Null;
    done["apps"][2]["installed"] = json!(true);
    done["recent"] = json!([
        { "task_id": "t2", "status": "succeeded", "app_name": "ncdu", "install": true, "message": "1 package(s) installed" },
        { "task_id": "old", "status": "failed", "app_name": "x", "install": true, "message": "boom" },
    ]);
    s.load(&done);
    assert_eq!((s.notes[0].tone, s.notes[0].text.as_str()), (Tone::Ok, "ncdu installed"));
    assert!(s.progress().is_none());
    let mut fail = done.clone();
    fail["recent"].as_array_mut().unwrap().insert(0, json!({ "task_id": "t3", "status": "failed", "app_name": "Dev tools", "install": true, "message": "installation exited with 100: E: Unable to locate package make" }));
    s.load(&fail);
    assert_eq!(s.notes[0].tone, Tone::Danger);
    assert!(s.notes[0].text.contains("Dev tools: failed") && s.notes[0].text.contains("Unable to locate package"));
    s.load(&fail);
    assert_eq!(s.notes.iter().filter(|n| n.text.contains("Dev tools")).count(), 1, "each change is notified once");
    let text = screen(&s);
    assert!(text.contains("✔") && text.contains("✖") && text.contains("No operation in progress."));
    s.on_key(KeyCode::Char('c'), NONE);
    assert!(s.notes.is_empty(), "c clears the log");
}
