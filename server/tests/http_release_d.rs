//! Integration (0.8): the Logs page and AUR update watching.
mod support;

use actix_web::test::TestRequest;
use chrono::{Duration, Utc};
use croustypatch_server::schema::{hosts, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call};
use uuid::Uuid;

// ------------------------------------------------------------------------------------------------- 0.9

#[actix_web::test]
async fn the_logs_page_merges_tasks_notifications_reports_and_the_servers_own_lines() {
    use croustypatch_server::schema::{notification_channels, notifications, task_events};
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let now = Utc::now();
    {
        let mut c = db.conn();
        let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("srv-logs"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(&mut c).unwrap();
        let task: Uuid = diesel::insert_into(tasks::table).values((tasks::host_id.eq(host), tasks::kind.eq("patch"), tasks::status.eq("failed"), tasks::message.eq("x"))).returning(tasks::id).get_result(&mut c).unwrap();
        for (i, (st, msg)) in [("queued", "requested by admin"), ("running", "applying 3 packages"), ("failed", "dpkg was interrupted")].iter().enumerate() {
            diesel::insert_into(task_events::table).values((task_events::task_id.eq(task), task_events::ts.eq(now - Duration::minutes(10 - i as i64)), task_events::status.eq(*st), task_events::message.eq(*msg))).execute(&mut c).unwrap();
        }
        let ch: Uuid = diesel::insert_into(notification_channels::table).values((notification_channels::name.eq("SIEM"), notification_channels::kind.eq("webhook"), notification_channels::config.eq(json!({})))).returning(notification_channels::id).get_result(&mut c).unwrap();
        diesel::insert_into(notifications::table)
            .values((notifications::channel_id.eq(ch), notifications::event.eq("task.failed"), notifications::severity.eq("high"), notifications::title.eq("Patch failed"), notifications::body.eq(""), notifications::payload.eq(json!({})), notifications::recipients.eq(json!([])), notifications::status.eq("failed"), notifications::attempts.eq(5), notifications::last_error.eq("connection refused"), notifications::created_at.eq(now - Duration::minutes(5))))
            .execute(&mut c)
            .unwrap();
    }
    croustypatch_server::logs::push_server(now - Duration::minutes(1), "warn", "croustypatch_server::scheduler", "planner failed for host".into());

    let get = |q: &str| TestRequest::get().uri(&format!("/api/v1/logs?{q}")).cookie(ck.clone());
    let (st, all) = call(&app, get("limit=50")).await;
    assert_eq!(st, 200, "{all}");
    let rows = all.as_array().unwrap();
    let ts: Vec<&str> = rows.iter().map(|r| r["ts"].as_str().unwrap()).collect();
    assert!(ts.windows(2).all(|w| w[0] >= w[1]), "newest first: {ts:?}");
    let sources: std::collections::HashSet<&str> = rows.iter().map(|r| r["source"].as_str().unwrap()).collect();
    assert!(["tasks", "notifications", "server"].iter().all(|s| sources.contains(s)), "{sources:?}");
    let failed = rows.iter().find(|r| r["message"] == json!("dpkg was interrupted")).unwrap();
    assert_eq!((failed["level"].clone(), failed["hostname"].clone(), failed["title"].clone()), (json!("error"), json!("srv-logs"), json!("patch · failed")));
    let n = rows.iter().find(|r| r["source"] == json!("notifications")).unwrap();
    assert!(n["message"].as_str().unwrap().contains("connection refused") && n["title"] == json!("task.failed → SIEM"), "{n}");

    // filters: errors only, one source, words
    let (_, errs) = call(&app, get("level=error")).await;
    assert!(errs.as_array().unwrap().iter().all(|r| r["level"] == json!("error")) && errs.as_array().unwrap().len() >= 2, "{errs}");
    let (_, srv) = call(&app, get("source=server&level=warn&q=planner")).await;
    assert!(srv.as_array().unwrap().iter().any(|r| r["message"] == json!("planner failed for host")), "{srv}");
    let (_, words) = call(&app, get("q=srv-logs%20dpkg")).await;
    assert_eq!(words.as_array().unwrap().len(), 1, "{words}");
    // paging: strictly older than the last line shown
    let (_, page) = call(&app, get(&format!("source=tasks&before={}", urlencode(failed["ts"].as_str().unwrap())))).await;
    assert_eq!(page.as_array().unwrap().iter().map(|r| r["message"].as_str().unwrap()).collect::<Vec<_>>(), vec!["applying 3 packages", "requested by admin"]);
    let (st, _) = call(&app, get("source=nope")).await;
    assert_eq!(st, 400);
    // like the audit log: not for operators
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/logs")).await;
    assert_eq!(st, 401);
}

fn urlencode(s: &str) -> String {
    s.replace('+', "%2B").replace(':', "%3A")
}

#[actix_web::test]
async fn arch_agents_watch_the_aur_packages_of_the_catalog_and_the_app_center_shows_their_updates() {
    use croustypatch_common::protocol::{AgentMessage, AppUpdate, HostFacts, Inventory};
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host: Uuid = {
        let mut c = db.conn();
        diesel::insert_into(hosts::table)
            .values((hosts::hostname.eq("crshell"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::package_manager.eq("pacman"), hosts::modules.eq(json!({"portal": true})), hosts::facts.eq(json!({"capabilities": croustypatch_common::protocol::capability::ALL}))))
            .returning(hosts::id)
            .get_result(&mut c)
            .unwrap()
    };
    let recipe = json!({"pacman": {"command": {"install": "makepkg -si", "uninstall": "pacman -R visual-studio-code-bin", "check": "/usr/bin/code", "latest": {"aur": "visual-studio-code-bin"}}}});
    let (st, a) = call(&app, TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(json!({"name": "Visual Studio Code", "recipes": recipe}))).await;
    assert_eq!(st, 201, "{a}");
    let s = croustypatch_server::modules::settings_for(&mut db.conn(), host).unwrap().unwrap();
    assert_eq!(s.watch_aur, vec!["visual-studio-code-bin".to_string()]);

    // installed through the app center, then the agent reports a newer version in the AUR
    {
        let mut c = db.conn();
        diesel::insert_into(tasks::table).values((tasks::host_id.eq(host), tasks::kind.eq("app"), tasks::status.eq("succeeded"), tasks::message.eq("ok"), tasks::finished_at.eq(Some(Utc::now())), tasks::exec.eq(Some(json!({"app_id": a["id"], "install": true}))))).execute(&mut c).unwrap();
    }
    let state = actix_web::web::Data::new(db.state());
    let inv = Inventory {
        facts: HostFacts { hostname: "crshell".into(), package_manager: Some(croustypatch_common::model::PackageManager::Pacman), capabilities: croustypatch_common::protocol::capability::ALL.iter().map(|s| s.to_string()).collect(), ..Default::default() },
        packages: vec![],
        reboot_required: false,
        collected_at: Utc::now(),
        app_updates: vec![AppUpdate { package: "visual-studio-code-bin".into(), installed: "1.145.2-1".into(), latest: "1.146.0-1".into() }],
    };
    croustypatch_server::gateway::handle_messages(&state, host, vec![AgentMessage::Inventory(inv)]).await;
    let (st, v) = call(&app, TestRequest::get().uri(&format!("/api/v1/portal/hosts/{host}")).cookie(ck.clone())).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["apps"][0]["update"], json!({"package": "visual-studio-code-bin", "installed": "1.145.2-1", "latest": "1.146.0-1"}));
    // updating is installing again
    let (st, r) = call(&app, TestRequest::post().uri("/api/v1/portal/install").cookie(ck.clone()).set_json(json!({"host_id": host, "app_id": a["id"], "action": "INSTALL"}))).await;
    assert_eq!(st, 202, "{r}");
    // and an inventory without updates clears it
    let inv = Inventory { facts: HostFacts { hostname: "crshell".into(), package_manager: Some(croustypatch_common::model::PackageManager::Pacman), ..Default::default() }, packages: vec![], reboot_required: false, collected_at: Utc::now(), app_updates: vec![] };
    croustypatch_server::gateway::handle_messages(&state, host, vec![AgentMessage::Inventory(inv)]).await;
    let (_, v) = call(&app, TestRequest::get().uri(&format!("/api/v1/portal/hosts/{host}")).cookie(ck.clone())).await;
    assert_eq!(v["apps"][0]["update"], Value::Null);
}
