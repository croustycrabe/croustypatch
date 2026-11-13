//! Integration: per-host modules, app-center recipes per system and command recipes, agent self-update.
mod support;

use actix_web::cookie::Cookie;
use actix_web::test::TestRequest;
use croustypatch_common::protocol::{capability, EnrollRequest, HostFacts};
use croustypatch_server::schema::{audit_log, hosts, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call, login, session};
use uuid::Uuid;

fn add_host(c: &mut PgConnection, name: &str, pm: &str, caps: &[&str]) -> Uuid {
    let facts = json!({ "hostname": name, "os_family": "linux", "arch": "x86_64", "agent_version": "0.4.0", "capabilities": caps });
    diesel::insert_into(hosts::table)
        .values((hosts::hostname.eq(name), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::package_manager.eq(pm), hosts::facts.eq(facts), hosts::modules.eq(json!({ "portal": true }))))
        .returning(hosts::id)
        .get_result(c)
        .unwrap()
}

#[actix_web::test]
async fn apps_follow_each_system_and_commands_need_their_own_permission() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (debian, arch, freebsd) = {
        let mut c = db.conn();
        (add_host(&mut c, "deb", "apt", capability::ALL), add_host(&mut c, "arch", "pacman", &[]), add_host(&mut c, "bsd", "pkg", capability::ALL))
    };
    let post = |ck: &Cookie<'static>, b: Value| TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(b);

    // validation of recipes
    for bad in [
        json!({"name": "x", "recipes": {}}),
        json!({"name": "x", "recipes": {"yum": {"packages": ["x"]}}}),
        json!({"name": "x", "recipes": {"apt": {"packages": ["fd-find"], "command": {"install": "a", "uninstall": "b", "check": "/x"}}}}),
        json!({"name": "x", "recipes": {"*": {"command": {"install": "a", "uninstall": "b", "check": "relative/path"}}}}),
        json!({"name": "x", "recipes": {"*": {"command": {"install": "a", "uninstall": "b", "check": "/opt/../etc/shadow"}}}}),
        json!({"name": "x", "recipes": {"*": {"command": {"install": "", "uninstall": "b", "check": "/x"}}}}),
    ] {
        let (st, b) = call(&app, post(&ck, bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }

    let (st, fd) = call(&app, post(&ck, json!({"name": "fd", "recipes": {"apt": {"packages": ["fd-find"]}, "pacman": {"packages": ["fd"]}}}))).await;
    assert_eq!(st, 201, "{fd}");
    let script = "curl -fsSL https://intranet/tool.tar.gz | tar -xz -C /opt\nln -sf /opt/tool/bin/tool /usr/local/bin/tool";
    let internal = json!({"name": "Internal tool", "recipes": {"*": {"command": {"install": script, "uninstall": "rm -rf /opt/tool /usr/local/bin/tool", "check": "/usr/local/bin/tool"}}}});

    // an admin (settings:write) without portal:commands cannot write a command
    let (_, roles) = call(&app, TestRequest::get().uri("/api/v1/roles").cookie(ck.clone())).await;
    let admin_role = roles.as_array().unwrap().iter().find(|r| r["name"] == "admin").unwrap()["id"].clone();
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/users").cookie(ck.clone()).set_json(json!({"username": "adm2", "password": "temporary-pass-12", "role_id": admin_role}))).await;
    assert_eq!(st, 201);
    let first = session(&login(&app, "adm2", "temporary-pass-12").await);
    call(&app, TestRequest::post().uri("/api/v1/auth/password").cookie(first).set_json(json!({"current_password": "temporary-pass-12", "new_password": "adm2-new-password-1"}))).await;
    let adm2 = session(&login(&app, "adm2", "adm2-new-password-1").await);
    let (st, _) = call(&app, post(&adm2, internal.clone())).await;
    assert_eq!(st, 403, "commands run as root: separate permission");
    let (st, tool) = call(&app, post(&ck, internal.clone())).await;
    assert_eq!(st, 201, "{tool}");
    assert_eq!(tool["has_command"], json!(true));
    let (st, _) = call(&app, TestRequest::put().uri(&format!("/api/v1/portal/apps/{}", tool["id"].as_str().unwrap())).cookie(adm2.clone()).set_json(json!({"name": "Internal tool", "packages": ["htop"], "group_ids": []}))).await;
    assert_eq!(st, 403, "nor change an app that runs commands");
    let (st, _) = call(&app, TestRequest::delete().uri(&format!("/api/v1/portal/apps/{}", tool["id"].as_str().unwrap())).cookie(adm2.clone())).await;
    assert_eq!(st, 403);
    let audited: Vec<Value> = audit_log::table.filter(audit_log::action.eq("portal.app_created")).select(audit_log::details).load(&mut db.conn()).unwrap();
    assert!(audited.iter().any(|d| d["recipes"]["*"]["command"]["install"] == json!(script)), "the full command is audited");

    // each host sees what fits its package manager
    let view = |h: Uuid| TestRequest::get().uri(&format!("/api/v1/portal/hosts/{h}")).cookie(ck.clone());
    let names = |v: &Value| v["apps"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let (_, v) = call(&app, view(debian)).await;
    let app_named = |v: &Value, n: &str| v["apps"].as_array().unwrap().iter().find(|a| a["name"] == n).cloned().unwrap();
    let mut seen = names(&v);
    seen.sort();
    assert_eq!(seen, vec!["Internal tool", "fd"]);
    assert_eq!(app_named(&v, "fd")["packages"], json!(["fd-find"]));
    assert_eq!(app_named(&v, "Internal tool")["kind"], json!("command"));
    let (_, v) = call(&app, view(arch)).await;
    assert_eq!(app_named(&v, "fd")["packages"], json!(["fd"]));
    let (_, v) = call(&app, view(freebsd)).await;
    assert_eq!(names(&v), vec!["Internal tool"], "no pkg recipe for fd: not offered on FreeBSD");

    // command apps need an agent that runs them
    let req = |h: Uuid, a: &Value| TestRequest::post().uri("/api/v1/portal/install").cookie(ck.clone()).set_json(json!({"host_id": h, "app_id": a["id"], "action": "INSTALL"}));
    let (st, b) = call(&app, req(arch, &tool)).await;
    assert_eq!(st, 409, "old agent: {b}");
    let (st, b) = call(&app, req(debian, &tool)).await;
    assert_eq!(st, 202, "{b}");
    let state = db.state();
    std::thread::sleep(std::time::Duration::from_secs(2)); // a remote test database may run a moment ahead of this machine
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, debian).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!(payload["action"]["command"]["install"], json!(script), "the command travels inside the signed order");
    assert_eq!(payload["action"]["command"]["check"], json!("/usr/local/bin/tool"));
    assert!(payload["action"].get("user").is_none(), "a web request names no local user");
}

#[actix_web::test]
async fn modules_are_set_per_host_from_the_enrolment_token_onwards() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/enrollment-tokens").cookie(ck.clone()).set_json(json!({"modules": {"docker": true, "bogus": true}}))).await;
    assert_eq!(st, 400);
    let (st, tok) = call(&app, TestRequest::post().uri("/api/v1/enrollment-tokens").cookie(ck.clone()).set_json(json!({"modules": {"docker": true, "eco": true}, "max_uses": 2}))).await;
    assert_eq!(st, 201, "{tok}");
    let kp = croustypatch_common::crypto::generate_rsa(2048).unwrap();
    let enroll = |name: &str, token: &str| {
        TestRequest::post().uri("/api/v1/agent/enroll").set_json(EnrollRequest { token: token.into(), facts: HostFacts { hostname: name.into(), ..Default::default() }, public_key_pem: kp.public_pem.clone() })
    };
    // the machine type comes from the agent: battery = laptop, graphical desktop = workstation, else server
    let (_, tok3) = call(&app, TestRequest::post().uri("/api/v1/enrollment-tokens").cookie(ck.clone()).set_json(json!({"max_uses": 2}))).await;
    for (name, laptop, desktop, want) in [("ws", false, true, "workstation"), ("lt", true, true, "laptop")] {
        let req = TestRequest::post().uri("/api/v1/agent/enroll").set_json(EnrollRequest { token: tok3["token"].as_str().unwrap().into(), facts: HostFacts { hostname: name.into(), is_laptop: laptop, is_desktop: desktop, ..Default::default() }, public_key_pem: kp.public_pem.clone() });
        let (st, r) = call(&app, req).await;
        assert_eq!(st, 201, "{r}");
        let t: String = hosts::table.find(r["agent_id"].as_str().unwrap().parse::<Uuid>().unwrap()).select(hosts::asset_type).first(&mut db.conn()).unwrap();
        assert_eq!(t, want);
        // laptops and workstations may start their own updates from the app center unless the token says otherwise
        let m: Value = hosts::table.find(r["agent_id"].as_str().unwrap().parse::<Uuid>().unwrap()).select(hosts::modules).first(&mut db.conn()).unwrap();
        assert_eq!(m["portal_updates"], json!(true), "{name}: {m}");
    }
    // ... and the token has the last word, either way
    let enroll_with = |name: &str, token: &str, laptop: bool| {
        TestRequest::post().uri("/api/v1/agent/enroll").set_json(EnrollRequest { token: token.into(), facts: HostFacts { hostname: name.into(), is_laptop: laptop, is_desktop: laptop, ..Default::default() }, public_key_pem: kp.public_pem.clone() })
    };
    for (name, laptop, flag) in [("lt-off", true, false), ("srv-on", false, true)] {
        let (_, t) = call(&app, TestRequest::post().uri("/api/v1/enrollment-tokens").cookie(ck.clone()).set_json(json!({"modules": {"portal_updates": flag}}))).await;
        let (st, r) = call(&app, enroll_with(name, t["token"].as_str().unwrap(), laptop)).await;
        assert_eq!(st, 201, "{r}");
        let m: Value = hosts::table.find(r["agent_id"].as_str().unwrap().parse::<Uuid>().unwrap()).select(hosts::modules).first(&mut db.conn()).unwrap();
        assert_eq!(m["portal_updates"], json!(flag), "{name}: {m}");
    }
    let (st, r) = call(&app, enroll("chosen", tok["token"].as_str().unwrap())).await;
    assert_eq!(st, 201, "{r}");
    let id: Uuid = r["agent_id"].as_str().unwrap().parse().unwrap();
    let m: Value = hosts::table.find(id).select(hosts::modules).first(&mut db.conn()).unwrap();
    assert_eq!(m, json!({"docker": true, "helper_script": false, "portal": false, "portal_updates": false, "eco": true}), "a server does not get the self-service updates unless the token says so");

    // without a choice on the token: the defaults of the day (eco on unless switched off)
    let (_, tok2) = call(&app, TestRequest::post().uri("/api/v1/enrollment-tokens").cookie(ck.clone()).set_json(json!({}))).await;
    let (_, r2) = call(&app, enroll("plain", tok2["token"].as_str().unwrap())).await;
    let id2: Uuid = r2["agent_id"].as_str().unwrap().parse().unwrap();
    let m2: Value = hosts::table.find(id2).select(hosts::modules).first(&mut db.conn()).unwrap();
    assert_eq!(m2["eco"], json!(true));
    assert_eq!(m2["docker"], json!(false));

    // a partial change only touches the named modules, and is audited
    let (st, h) = call(&app, TestRequest::patch().uri(&format!("/api/v1/hosts/{id}")).cookie(ck.clone()).set_json(json!({"modules": {"eco": true}}))).await;
    assert_eq!(st, 200, "{h}");
    assert_eq!((h["modules"]["docker"].clone(), h["modules"]["eco"].clone()), (json!(true), json!(true)));
    let (st, _) = call(&app, TestRequest::patch().uri(&format!("/api/v1/hosts/{id}")).cookie(ck.clone()).set_json(json!({"modules": {"docker": "yes"}}))).await;
    assert_eq!(st, 400);
    let n: i64 = audit_log::table.filter(audit_log::action.eq("host.modules_changed")).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(n, 1);

    // settings reach agents that understand them (push mode: in the telemetry reply)
    assert!(croustypatch_server::modules::settings_for(&mut db.conn(), id).unwrap().is_none(), "an old agent gets nothing it cannot read");
    diesel::update(hosts::table.find(id)).set(hosts::facts.eq(json!({"capabilities": [capability::SETTINGS]}))).execute(&mut db.conn()).unwrap();
    let s = croustypatch_server::modules::settings_for(&mut db.conn(), id).unwrap().unwrap();
    assert!(s.modules.docker && s.modules.eco && !s.modules.portal);
    assert!(s.isolation.keep_ssh && s.isolation.ssh_port == 22 && s.isolation.ssh_sources.is_empty(), "SSH stays reachable by default");

    // isolation rules: validated, then carried to the agents with their settings
    let put = |b: Value| TestRequest::put().uri("/api/v1/settings/isolation").cookie(ck.clone()).set_json(b);
    for bad in [json!({"keep_ssh": true, "ssh_port": 0, "ssh_sources": []}), json!({"keep_ssh": true, "ssh_port": 22, "ssh_sources": ["10.0.0.0/24; flush ruleset"]}), json!({"keep_ssh": "yes"})] {
        let (st, b) = call(&app, put(bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    let (st, _) = call(&app, put(json!({"keep_ssh": true, "ssh_port": 2222, "ssh_sources": ["10.0.10.0/24"]}))).await;
    assert!(st.is_success());
    let s = croustypatch_server::modules::settings_for(&mut db.conn(), id).unwrap().unwrap();
    assert_eq!((s.isolation.ssh_port, s.isolation.ssh_sources.clone()), (2222, vec!["10.0.10.0/24".to_string()]));
}

#[actix_web::test]
async fn the_agent_updates_itself_only_when_it_can() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("croustypatch-agent-linux-x86_64"), b"new agent build").unwrap();
    std::fs::write(dir.path().join("croustypatch-portal-gui-linux-x86_64"), b"new desktop build").unwrap();
    let db = support::TestDb::new();
    let agent_dir = dir.path().to_path_buf();
    let state = db.state_with(|c| c.agent_dir = vec![agent_dir.clone()]);
    let app = support::http::app_with(&db, db.state_with(|c| c.agent_dir = vec![agent_dir.clone()])).await;
    let ck = admin(&app).await;
    let (old, new) = {
        let mut c = db.conn();
        (add_host(&mut c, "old", "apt", &[]), add_host(&mut c, "new", "apt", capability::ALL))
    };
    let upd = |h: Uuid| TestRequest::post().uri(&format!("/api/v1/hosts/{h}/agent-update")).cookie(ck.clone());
    let (st, b) = call(&app, upd(old)).await;
    assert_eq!(st, 409, "{b}");
    assert!(b.to_string().contains("installer"), "tells how to update an old agent: {b}");
    let (st, b) = call(&app, upd(new)).await;
    assert_eq!(st, 202, "{b}");
    // `scheduled_for` defaults to the database's now(): a test database on another machine may run slightly ahead.
    std::thread::sleep(std::time::Duration::from_secs(2));
    let (st, _) = call(&app, upd(new)).await;
    assert_eq!(st, 409, "one update at a time");
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, new).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!(payload["action"]["type"], json!("update_agent"));
    assert_eq!(payload["action"]["platform"], json!("linux-x86_64"));
    assert_eq!(payload["action"]["sha256"], json!(croustypatch_common::crypto::sha256_hex(b"new agent build")));
    assert_eq!(payload["action"]["version"], json!(env!("CARGO_PKG_VERSION")));
    assert!(payload["action"].get("gui_sha256").is_none(), "no desktop app on this host: nothing to refresh");

    // a laptop with the desktop app center gets it refreshed in the same order
    let laptop = add_host(&mut db.conn(), "laptop", "pacman", capability::ALL);
    diesel::update(hosts::table.find(laptop)).set(hosts::facts.eq(json!({"os_family": "linux", "arch": "x86_64", "capabilities": capability::ALL, "portal_gui": true}))).execute(&mut db.conn()).unwrap();
    let (st, _) = call(&app, upd(laptop)).await;
    assert_eq!(st, 202);
    std::thread::sleep(std::time::Duration::from_secs(2));
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, laptop).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!(payload["action"]["gui_sha256"], json!(croustypatch_common::crypto::sha256_hex(b"new desktop build")));
    let kind: String = tasks::table.filter(tasks::host_id.eq(new)).select(tasks::kind).first(&mut db.conn()).unwrap();
    assert_eq!(kind, "agent_update");
}
