//! Integration: the Docker menu says why a container has no suggested version.
mod support;

use actix_web::test::TestRequest;
use croustypatch_server::schema::{host_packages, hosts};
use diesel::prelude::*;
use serde_json::json;
use support::http::{admin, app, call};
use uuid::Uuid;

#[actix_web::test]
async fn the_docker_menu_carries_what_the_registry_checks_found() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("docker-x"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::modules.eq(json!({"docker": true})))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    let add = |name: &str, meta: serde_json::Value| {
        diesel::insert_into(host_packages::table)
            .values((host_packages::host_id.eq(host), host_packages::name.eq(format!("docker:{name}")), host_packages::source.eq("docker"), host_packages::version.eq("ghcr.io/x/y:v1.0.0 @abc"), host_packages::meta.eq(meta)))
            .execute(&mut db.conn())
            .unwrap();
    };
    add("failed", json!({"image": "ghcr.io/x/y:v1.0.0", "check": {"digest": "ok", "tags": "error: registry answered 401"}}));
    add("probed", json!({"image": "ghcr.io/x/y:v1.0.0", "check": {"digest": "ok", "tags": "probed"}, "latest": {"tag": "v1.2.0", "major": false}}));
    add("old-agent", json!({"image": "ghcr.io/x/y:v1.0.0"}));
    let (st, v) = call(&app, TestRequest::get().uri("/api/v1/docker").cookie(ck)).await;
    assert_eq!(st, 200, "{v}");
    let by = |n: &str| v["containers"].as_array().unwrap().iter().find(|c| c["name"] == json!(n)).unwrap().clone();
    assert_eq!(by("failed")["check"], json!({"digest": "ok", "tags": "error: registry answered 401"}));
    assert_eq!(by("failed")["latest"], serde_json::Value::Null);
    assert_eq!((by("probed")["check"]["tags"].clone(), by("probed")["latest"]["tag"].clone()), (json!("probed"), json!("v1.2.0")));
    assert_eq!(by("old-agent")["check"], serde_json::Value::Null, "agents before this release send no check");
}
