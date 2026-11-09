//! Integration: the app center (catalog, scope, install / uninstall requests, signed command).
mod support;

use actix_web::test::TestRequest;
use croustypatch_server::schema::{host_group_members, host_groups, host_packages, hosts, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call};
use uuid::Uuid;

#[actix_web::test]
async fn the_app_center_installs_only_catalog_apps_in_scope() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (dev_host, other_host, dev_group) = {
        let mut c = db.conn();
        let g: Uuid = diesel::insert_into(host_groups::table).values(host_groups::name.eq("devs")).returning(host_groups::id).get_result(&mut c).unwrap();
        let a: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("ws-dev"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(&mut c).unwrap();
        let b: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("ws-sales"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(&mut c).unwrap();
        diesel::insert_into(host_group_members::table).values((host_group_members::host_id.eq(a), host_group_members::group_id.eq(g))).execute(&mut c).unwrap();
        diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(a), host_packages::name.eq("git"), host_packages::version.eq("2.43"))).execute(&mut c).unwrap();
        (a, b, g)
    };
    let post = |b: Value| TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(b);
    for bad in [json!({"name": "x", "packages": []}), json!({"name": "x", "packages": ["docker:evil"]}), json!({"name": "x", "packages": ["-rf"]}), json!({"name": "x", "packages": ["vim"], "icon": "<svg>"})] {
        let (st, b) = call(&app, post(bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    let (st, vscode) = call(&app, post(json!({"name": "Dev tools", "category": "Développement", "icon": "code", "packages": ["git", "make"], "group_ids": [dev_group]}))).await;
    assert_eq!(st, 201, "{vscode}");
    let (_, editor) = call(&app, post(json!({"name": "Vim", "category": "Outils", "packages": ["vim"]}))).await;

    // module off: the portal refuses
    let view = |h: Uuid| TestRequest::get().uri(&format!("/api/v1/portal/hosts/{h}")).cookie(ck.clone());
    let (st, _) = call(&app, view(dev_host)).await;
    assert_eq!(st, 409);
    let (st, _) = call(&app, TestRequest::put().uri("/api/v1/settings/modules").cookie(ck.clone()).set_json(json!({"docker": false, "helper_script": false, "openscap": false, "portal": true, "notifications": true, "eco": true}))).await;
    assert!(st.is_success());

    let (_, v) = call(&app, view(dev_host)).await;
    let names: Vec<&str> = v["apps"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Dev tools", "Vim"]);
    assert_eq!(v["apps"][0]["installed"], json!(false), "git alone is not the whole app");
    let (_, v2) = call(&app, view(other_host)).await;
    assert_eq!(v2["apps"].as_array().unwrap().len(), 1, "the dev app is scoped to the dev group");

    let req = |h: Uuid, a: &Value, action: &str| TestRequest::post().uri("/api/v1/portal/install").cookie(ck.clone()).set_json(json!({"host_id": h, "app_id": a["id"], "action": action}));
    let (st, _) = call(&app, req(other_host, &vscode, "INSTALL")).await;
    assert_eq!(st, 403, "an app outside the host's scope cannot be requested");
    let (st, r) = call(&app, req(dev_host, &vscode, "INSTALL")).await;
    assert_eq!(st, 202, "{r}");
    assert_eq!(r["status"], json!("queued"));
    let (kind, exec): (String, Option<Value>) = tasks::table.filter(tasks::host_id.eq(dev_host)).select((tasks::kind, tasks::exec)).first(&mut db.conn()).unwrap();
    assert_eq!(kind, "app");
    assert_eq!(exec.unwrap()["packages"], json!(["git", "make"]));
    let (st, _) = call(&app, req(dev_host, &editor, "UNINSTALL")).await;
    assert_eq!(st, 409, "one app operation at a time per host");
    let (_, v3) = call(&app, view(dev_host)).await;
    assert_eq!(v3["apps"][0]["pending"]["status"], json!("queued"));
    assert_eq!((v3["recent"][0]["app_name"].clone(), v3["recent"][0]["status"].clone(), v3["recent"][0]["install"].clone()), (json!("Dev tools"), json!("queued"), json!(true)), "recent operations are listed with their outcome");

    // the signed command carries the packages
    let state = db.state();
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, dev_host).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!((payload["action"]["type"].clone(), payload["action"]["packages"].clone()), (json!("app"), json!(["git", "make"])), "{payload}");
    assert_eq!(payload["action"]["install"], json!(true));
}

#[actix_web::test]
async fn a_local_user_asks_through_the_agent_for_its_own_host_only() {
    use croustypatch_common::crypto;
    use croustypatch_common::jwt::{self, Claims};
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let kp = crypto::generate_rsa(2048).unwrap();
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("srv-tui"), hosts::public_key_pem.eq(&kp.public_pem), hosts::status.eq("online"))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    call(&app, TestRequest::put().uri("/api/v1/settings/modules").cookie(ck.clone()).set_json(json!({"docker": false, "helper_script": false, "openscap": false, "portal": true, "notifications": true, "eco": true}))).await;
    let (_, a) = call(&app, TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(json!({"name": "htop", "packages": ["htop"]}))).await;
    let mint = |jti: &str| { let now = chrono::Utc::now().timestamp(); jwt::sign_rs256(&kp.private_pem, &Claims { sub: host.to_string(), iss: "agent".into(), iat: now - 5, exp: now + 60, jti: jti.into(), scope: None }).unwrap() };

    let (st, v) = call(&app, TestRequest::get().uri("/api/v1/agent/portal").insert_header(("authorization", format!("Bearer {}", mint("p1"))))).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["host"]["hostname"], json!("srv-tui"));
    assert_eq!(v["apps"][0]["name"], json!("htop"));

    let req = |jti: &str, user: &str| TestRequest::post().uri("/api/v1/agent/portal/install").insert_header(("authorization", format!("Bearer {}", mint(jti)))).set_json(json!({"app_id": a["id"], "action": "INSTALL", "user": user}));
    let (st, _) = call(&app, req("p2", "bob; rm -rf /")).await;
    assert_eq!(st, 400, "the local user name is validated");
    let (st, r) = call(&app, req("p3", "bob")).await;
    assert_eq!(st, 202, "{r}");
    let by: String = tasks::table.filter(tasks::host_id.eq(host)).select(tasks::requested_by).first(&mut db.conn()).unwrap();
    assert_eq!(by, "local:bob@srv-tui", "the audit trail names the local account");
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/agent/portal")).await;
    assert_eq!(st, 401, "only an enrolled agent can ask");
}
