//! Integration: resource use reported with the heartbeat, its history, and the dashboard sections that use it.
mod support;

use actix_web::test::TestRequest;
use actix_web::web;
use croustypatch_common::protocol::{AgentMessage, DiskUsage, Heartbeat, Metrics};
use croustypatch_server::gateway;
use croustypatch_server::schema::{host_metrics, hosts};
use diesel::prelude::*;
use serde_json::json;
use support::http::{admin, app, call};
use uuid::Uuid;

fn beat(cpu: Option<f32>, used: u64) -> AgentMessage {
    AgentMessage::Heartbeat(Heartbeat {
        on_battery: false, eco: false, load_1m: 0.5, uptime_secs: 3600, running_tasks: vec![],
        metrics: Some(Metrics { cpu_pct: cpu, mem_total_mb: 8000, mem_used_mb: used, swap_total_mb: 2000, swap_used_mb: 100, load_5m: 0.4, load_15m: 0.3, disks: vec![DiskUsage { mount: "/".into(), fs: "ext4".into(), total_mb: 100_000, used_mb: 91_000 }] }),
    })
}

fn add_host(c: &mut PgConnection, name: &str) -> Uuid {
    diesel::insert_into(hosts::table).values((hosts::hostname.eq(name), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(c).unwrap()
}

#[actix_web::test]
async fn heartbeats_keep_the_latest_reading_and_a_point_every_five_minutes() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let host = add_host(&mut db.conn(), "web-01");

    gateway::handle_messages(&state, host, vec![beat(None, 2000)]).await;
    gateway::handle_messages(&state, host, vec![beat(Some(40.0), 4000), beat(Some(42.0), 4100)]).await;
    let points: i64 = host_metrics::table.filter(host_metrics::host_id.eq(host)).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(points, 1, "heartbeats arriving seconds apart make one history point");

    let (st, m) = call(&app, TestRequest::get().uri(&format!("/api/v1/hosts/{host}/metrics")).cookie(ck.clone())).await;
    assert_eq!(st, 200, "{m}");
    assert_eq!(m["current"]["cpu_pct"], json!(42.0), "the latest reading wins");
    assert_eq!(m["current"]["mem_used_mb"], json!(4100));
    assert_eq!(m["current"]["mem_pct"].as_f64().unwrap().round(), 51.0);
    assert_eq!(m["current"]["disks"][0]["mount"], json!("/"));
    assert_eq!(m["points"].as_array().unwrap().len(), 1);
    assert!(m["at"].is_string());

    // an older point is added once five minutes have passed
    diesel::update(host_metrics::table).set(host_metrics::at.eq(chrono::Utc::now() - chrono::Duration::minutes(6))).execute(&mut db.conn()).unwrap();
    gateway::handle_messages(&state, host, vec![beat(Some(10.0), 3000)]).await;
    let (_, m) = call(&app, TestRequest::get().uri(&format!("/api/v1/hosts/{host}/metrics?hours=1")).cookie(ck.clone())).await;
    assert_eq!(m["points"].as_array().unwrap().len(), 2);

    // a heartbeat from an agent before 0.6 changes nothing but the liveness
    let old = AgentMessage::Heartbeat(Heartbeat { on_battery: false, eco: false, load_1m: 0.1, uptime_secs: 1, running_tasks: vec![], metrics: None });
    gateway::handle_messages(&state, host, vec![old]).await;
    let (_, m) = call(&app, TestRequest::get().uri(&format!("/api/v1/hosts/{host}/metrics")).cookie(ck.clone())).await;
    assert_eq!(m["current"]["mem_used_mb"], json!(3000));
}

#[actix_web::test]
async fn the_dashboard_summarises_resources_and_patching() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let (a, b) = { let mut c = db.conn(); (add_host(&mut c, "busy"), add_host(&mut c, "idle")) };
    gateway::handle_messages(&state, a, vec![beat(Some(90.0), 7600)]).await;
    gateway::handle_messages(&state, b, vec![beat(Some(5.0), 1000)]).await;
    let (st, d) = call(&app, TestRequest::get().uri("/api/v1/dashboard").cookie(ck.clone())).await;
    assert_eq!(st, 200, "{d}");
    assert_eq!(d["resources"]["reporting"], json!(2));
    assert_eq!(d["resources"]["avg_cpu"], json!(47.5));
    assert_eq!(d["resources"]["busiest"][0]["hostname"], json!("busy"), "busiest first");
    assert_eq!(d["resources"]["busiest"][0]["disk"].as_f64().unwrap().round(), 91.0, "the fullest disk is reported");
    assert_eq!(d["patching"]["uncovered_hosts"], json!(2), "no policy covers these hosts");
    assert_eq!(d["patching"]["week"]["failed"], json!(0));
    assert!(d["patching"]["next"].as_array().unwrap().is_empty());

    // a host that stopped reporting drops out of the averages
    diesel::update(hosts::table.find(a)).set(hosts::metrics_at.eq(chrono::Utc::now() - chrono::Duration::hours(2))).execute(&mut db.conn()).unwrap();
    let (_, d) = call(&app, TestRequest::get().uri("/api/v1/dashboard").cookie(ck)).await;
    assert_eq!(d["resources"]["reporting"], json!(1));
}
