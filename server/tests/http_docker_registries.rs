//! Integration: Docker registries (local or distant), their passwords, what agents receive, moving a container to a
//! configured registry, and the offline rule: no Docker features without a local registry.
mod support;

use actix_web::test::TestRequest;
use croustypatch_server::schema::{host_packages, hosts, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call};
use uuid::Uuid;

fn registry(name: &str, kind: &str, host: &str) -> Value {
    json!({"name": name, "kind": kind, "host": host, "mirror_of": [], "path_prefix": "", "username": "", "password": "", "insecure": false})
}

fn add_host(db: &support::TestDb, name: &str, docker: bool) -> Uuid {
    let caps = json!({"capabilities": croustypatch_common::protocol::capability::ALL});
    diesel::insert_into(hosts::table).values((hosts::hostname.eq(name), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::modules.eq(json!({ "docker": docker })), hosts::facts.eq(caps))).returning(hosts::id).get_result(&mut db.conn()).unwrap()
}

#[actix_web::test]
async fn registries_are_validated_masked_and_keep_their_password_when_it_is_left_empty() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let put = |regs: Value| TestRequest::put().uri("/api/v1/docker/registries").cookie(ck.clone()).set_json(json!({ "registries": regs }));
    for bad in [
        json!([registry("", "local", "reg.lan")]),
        json!([registry("x", "nearby", "reg.lan")]),
        json!([registry("x", "local", "https://reg.lan")]),
        json!([registry("x", "local", "reg.lan/path")]),
        json!([registry("x", "local", "reg.lan:99999")]),
        json!([registry("x", "local", "reg.lan:")]),
        json!([registry("a", "local", "reg.lan"), registry("b", "distant", "REG.lan")]),
        json!([{"name": "x", "kind": "local", "host": "reg.lan", "mirror_of": ["not a host"]}]),
        json!([{"name": "x", "kind": "local", "host": "reg.lan", "path_prefix": "../x"}]),
        json!([{"name": "x", "kind": "local", "host": "reg.lan", "password": "a\nb"}]),
    ] {
        let (st, b) = call(&app, put(bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    let regs = json!([
        {"name": "Harbor", "kind": "local", "host": "Harbor.LAN:5000", "mirror_of": ["index.docker.io", "ghcr.io", "docker.io"], "path_prefix": "/dockerhub-proxy", "username": "robot", "password": "s3cret", "insecure": true},
        registry("GHCR", "distant", "ghcr.io"),
    ]);
    let (st, v) = call(&app, put(regs)).await;
    assert_eq!(st, 200, "{v}");
    let r = &v["registries"][0];
    assert_eq!((r["host"].clone(), r["mirror_of"].clone(), r["path_prefix"].clone(), r["password_set"].clone(), r["insecure"].clone()), (json!("harbor.lan:5000"), json!(["docker.io", "ghcr.io"]), json!("dockerhub-proxy/"), json!(true), json!(true)), "{v}");
    assert!(r.get("password").is_none() && !v.to_string().contains("s3cret"), "the password is never returned");
    assert_eq!(v["registries"][1]["password_set"], json!(false));
    let id = r["id"].as_str().unwrap().to_string();
    // saved again from what the page shows: the password stays
    let mut again = v["registries"].clone();
    again[0].as_object_mut().unwrap().remove("password_set");
    again[0]["name"] = json!("Harbor (lab)");
    let (st, _) = call(&app, put(again)).await;
    assert_eq!(st, 200);
    let stored = croustypatch_server::docker_registry::load(&mut db.conn());
    assert_eq!((stored[0].id.clone(), stored[0].name.as_str()), (id.clone(), "Harbor (lab)"));
    let st_app = db.state();
    let p = croustypatch_server::docker_registry::profiles(&stored, false, &st_app.master);
    assert_eq!((p[0].username.as_str(), p[0].password.as_str()), ("robot", "s3cret"), "the stored password survived the edit");
    assert!(!stored[0].password_ct.contains("s3cret"), "stored encrypted");
    // a new password replaces it; an empty user name drops the login
    let mut change = v["registries"].clone();
    change[0]["password"] = json!("n3w");
    call(&app, put(change)).await;
    let p = croustypatch_server::docker_registry::profiles(&croustypatch_server::docker_registry::load(&mut db.conn()), false, &st_app.master);
    assert_eq!(p[0].password, "n3w");
    let (_, listed) = call(&app, TestRequest::get().uri("/api/v1/docker/registries").cookie(ck.clone())).await;
    assert_eq!((listed["registries"].as_array().unwrap().len(), listed["offline"].clone(), listed["blocked"].clone()), (2, json!(false), json!(false)));
    // reading is for everyone who reads hosts, changing needs settings:write
    let (st, _) = call(&app, TestRequest::put().uri("/api/v1/docker/registries").set_json(json!({"registries": []}))).await;
    assert_eq!(st, 401);
}

#[actix_web::test]
async fn offline_mode_needs_a_local_registry_for_the_docker_features() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let set = |key: &str, v: Value| TestRequest::put().uri(&format!("/api/v1/settings/{key}")).cookie(ck.clone()).set_json(v);
    let regs = |v: Value| TestRequest::put().uri("/api/v1/docker/registries").cookie(ck.clone()).set_json(json!({ "registries": v }));
    let modules = |docker: bool| json!({"docker": docker, "helper_script": false, "openscap": false, "portal": false, "portal_updates": false, "notifications": false, "eco": true});
    let host = add_host(&db, "docker-off", false);

    // offline with no Docker feature on: allowed; the page says what is missing
    let (st, b) = call(&app, set("connectivity", json!({"mode": "air_gapped"}))).await;
    assert!(st.is_success(), "{b}");
    let (_, v) = call(&app, TestRequest::get().uri("/api/v1/docker/registries").cookie(ck.clone())).await;
    assert_eq!((v["blocked"].clone(), v["problem"].as_str().map(|p| p.contains("local Docker registry"))), (json!(true), Some(true)), "{v}");
    // ... but the Docker module cannot be switched on: for the fleet, nor for a host
    let (st, b) = call(&app, set("modules", modules(true))).await;
    assert_eq!(st, 400, "{b}");
    assert!(b["message"].as_str().unwrap().contains("Settings > Docker registries"), "the message says how to fix it: {b}");
    let (st, b) = call(&app, TestRequest::patch().uri(&format!("/api/v1/hosts/{host}")).cookie(ck.clone()).set_json(json!({"modules": {"docker": true}}))).await;
    assert_eq!(st, 400, "{b}");
    // a distant registry does not help offline
    let (st, _) = call(&app, regs(json!([registry("GHCR", "distant", "ghcr.io")]))).await;
    assert_eq!(st, 200, "listing it is fine");
    let (st, _) = call(&app, TestRequest::patch().uri(&format!("/api/v1/hosts/{host}")).cookie(ck.clone()).set_json(json!({"modules": {"docker": true}}))).await;
    assert_eq!(st, 400, "still no local registry");
    // with a local one it works
    let (st, _) = call(&app, regs(json!([registry("GHCR", "distant", "ghcr.io"), registry("Local", "local", "registry.lan:5000")]))).await;
    assert_eq!(st, 200);
    let (st, b) = call(&app, TestRequest::patch().uri(&format!("/api/v1/hosts/{host}")).cookie(ck.clone()).set_json(json!({"modules": {"docker": true}}))).await;
    assert_eq!(st, 200, "{b}");
    let (_, v) = call(&app, TestRequest::get().uri("/api/v1/docker/registries").cookie(ck.clone())).await;
    assert_eq!((v["blocked"].clone(), v["docker_enabled"].clone()), (json!(false), json!(true)));
    // and the last local registry cannot be taken away while the Docker module is on
    let (st, b) = call(&app, regs(json!([registry("GHCR", "distant", "ghcr.io")]))).await;
    assert_eq!(st, 400, "{b}");
    assert!(b["message"].as_str().unwrap().contains("last local registry"));
    let (st, _) = call(&app, regs(json!([registry("Other local", "local", "10.0.0.5:5000")]))).await;
    assert_eq!(st, 200, "replacing it by another local one is fine");

    // the other way round: online with Docker on and only distant registries, offline is refused until one is local
    let db2 = support::TestDb::new();
    let app2 = support::http::app(&db2).await;
    let ck2 = admin(&app2).await;
    let h2 = add_host(&db2, "docker-on", true);
    let _ = h2;
    let (st, b) = call(&app2, TestRequest::put().uri("/api/v1/settings/connectivity").cookie(ck2.clone()).set_json(json!({"mode": "air_gapped"}))).await;
    assert_eq!(st, 400, "{b}");
    assert!(b["message"].as_str().unwrap().contains("the Docker module is on"), "{b}");
    call(&app2, TestRequest::put().uri("/api/v1/docker/registries").cookie(ck2.clone()).set_json(json!({"registries": [registry("Local", "local", "registry.lan:5000")]}))).await;
    let (st, b) = call(&app2, TestRequest::put().uri("/api/v1/settings/connectivity").cookie(ck2.clone()).set_json(json!({"mode": "air_gapped"}))).await;
    assert!(st.is_success(), "{b}");
}

#[actix_web::test]
async fn agents_with_the_docker_module_get_the_registries_and_offline_ones_only_the_local() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (on, off) = (add_host(&db, "docker-yes", true), add_host(&db, "docker-no", false));
    call(&app, TestRequest::put().uri("/api/v1/docker/registries").cookie(ck.clone()).set_json(json!({"registries": [
        {"name": "Local", "kind": "local", "host": "registry.lan:5000", "mirror_of": ["docker.io"], "username": "robot", "password": "pw", "insecure": true},
        registry("GHCR", "distant", "ghcr.io"),
    ]}))).await;
    let st = db.state();
    let s = croustypatch_server::docker_registry::agent_settings(&mut db.conn(), &st, on).unwrap().unwrap();
    assert_eq!(s.registries.iter().map(|r| r.host.as_str()).collect::<Vec<_>>(), vec!["registry.lan:5000", "ghcr.io"]);
    assert_eq!((s.registries[0].password.as_str(), s.registries[0].insecure, s.registries[0].mirror_of.clone()), ("pw", true, vec!["docker.io".to_string()]));
    assert!(croustypatch_server::docker_registry::agent_settings(&mut db.conn(), &st, off).unwrap().unwrap().registries.is_empty(), "a host without the Docker module gets no registry (and no password)");
    call(&app, TestRequest::put().uri("/api/v1/settings/connectivity").cookie(ck.clone()).set_json(json!({"mode": "air_gapped"}))).await;
    let s = croustypatch_server::docker_registry::agent_settings(&mut db.conn(), &st, on).unwrap().unwrap();
    assert!(s.offline && s.registries.iter().map(|r| r.host.as_str()).collect::<Vec<_>>() == vec!["registry.lan:5000"], "offline: the distant registry is not sent");
}

#[actix_web::test]
async fn a_new_version_can_be_taken_from_a_configured_registry_which_the_container_keeps_using() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host = add_host(&db, "docker-m", true);
    let latest = json!({"tag": "v1.1.0", "major": false, "same_major": null, "apply": {"mode": "file", "file": "/opt/stacks/s/compose.yaml"}});
    for (name, image) in [("docker:web", "ghcr.io/x/y:v1.0.0"), ("docker:db", "nginx:1.27")] {
        let mut l = latest.clone();
        l["tag"] = json!(if name == "docker:db" { "1.28" } else { "v1.1.0" });
        diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(host), host_packages::name.eq(name), host_packages::source.eq("docker"), host_packages::version.eq(format!("{image} @aaaa")), host_packages::meta.eq(json!({"image": image, "latest": l})))).execute(&mut db.conn()).unwrap();
    }
    let (_, v) = call(&app, TestRequest::put().uri("/api/v1/docker/registries").cookie(ck.clone()).set_json(json!({"registries": [
        {"name": "Local", "kind": "local", "host": "registry.lan:5000", "mirror_of": ["docker.io"], "path_prefix": "mirror"},
        registry("Hub copy", "distant", "mirror.example.com"),
    ]}))).await;
    let (local, distant) = (v["registries"][0]["id"].as_str().unwrap().to_string(), v["registries"][1]["id"].as_str().unwrap().to_string());
    let retag_of = |h: Uuid| -> Value { let e: Option<Value> = tasks::table.filter(tasks::host_id.eq(h)).order(tasks::created_at.desc()).select(tasks::exec).first(&mut db.conn()).unwrap(); e.unwrap()["retag"].clone() };
    let post = |b: Value| TestRequest::post().uri("/api/v1/tasks").cookie(ck.clone()).set_json(b);
    let body = |pk: &str, reg: Value| json!({"host_ids": [host], "scope": "docker", "packages": [pk], "to_latest": true, "registry_id": reg});
    let (st, b) = call(&app, post(body("docker:web", json!("nope")))).await;
    assert_eq!(st, 400, "{b}");
    let (st, b) = call(&app, post(body("docker:web", json!(local)))).await;
    assert_eq!(st, 202, "{b}");
    assert_eq!(retag_of(host), json!({"docker:web": "registry.lan:5000/mirror/x/y:v1.1.0"}), "the same image and tag, held by the chosen registry");
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
    // a Docker Hub official image on a registry that mirrors Docker Hub keeps its library/ path
    let (st, b) = call(&app, post(body("docker:db", json!(local)))).await;
    assert_eq!(st, 202, "{b}");
    assert_eq!(retag_of(host), json!({"docker:db": "registry.lan:5000/mirror/library/nginx:1.28"}));
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
    // without a registry: as before
    let (st, _) = call(&app, post(body("docker:web", Value::Null))).await;
    assert_eq!(st, 202);
    assert_eq!(retag_of(host), json!({"docker:web": "ghcr.io/x/y:v1.1.0"}));
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
    // offline: only a local registry can be used
    call(&app, TestRequest::put().uri("/api/v1/settings/connectivity").cookie(ck.clone()).set_json(json!({"mode": "air_gapped"}))).await;
    let (st, b) = call(&app, post(body("docker:web", json!(distant)))).await;
    assert_eq!(st, 400, "{b}");
    assert!(b["message"].as_str().unwrap().contains("only a local registry"));
    let (st, b) = call(&app, post(body("docker:web", json!(local)))).await;
    assert_eq!(st, 202, "{b}");
    // pure unit checks of the reference
    let r = croustypatch_server::docker_registry::Stored { host: "reg.lan".into(), path_prefix: "p/".into(), mirror_of: vec![], ..Default::default() };
    assert_eq!(croustypatch_server::docker_registry::image_on(&r, "nginx:1.28").unwrap(), "reg.lan/p/nginx:1.28", "no library/ on a registry that does not mirror Docker Hub");
    assert_eq!(croustypatch_server::docker_registry::image_on(&r, "docker.io/grafana/grafana:11.0").unwrap(), "reg.lan/p/grafana/grafana:11.0");
    assert_eq!(croustypatch_server::docker_registry::image_on(&r, "localhost:5000/a/b").unwrap(), "reg.lan/p/a/b:latest");
    assert!(croustypatch_server::docker_registry::image_on(&r, "nginx@sha256:abc").is_none());
}
