//! Integration: the reboot module (downtime, completion, overdue alarm, reboot after patch by policy).
mod support;

use actix_web::test::TestRequest;
use actix_web::web;
use chrono::{Duration, Utc};
use croustypatch_common::model::TaskStatus;
use croustypatch_common::protocol::{AgentMessage, TaskUpdate};
use croustypatch_server::schema::{audit_log, hosts, policies, tasks};
use croustypatch_server::{gateway, reboot};
use diesel::prelude::*;
use serde_json::json;
use support::http::{admin, app, call};
use uuid::Uuid;

fn add_host(c: &mut PgConnection, asset: &str) -> Uuid {
    diesel::insert_into(hosts::table).values((hosts::hostname.eq(format!("{asset}-1")), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::asset_type.eq(asset), hosts::reboot_required.eq(true))).returning(hosts::id).get_result(c).unwrap()
}

fn update(task: Uuid, status: TaskStatus, msg: &str) -> AgentMessage {
    AgentMessage::TaskUpdate(TaskUpdate { task_id: task, status, message: msg.into(), log: vec![], changes: vec![], reboot_required: false })
}

fn actions(c: &mut PgConnection, host: Uuid) -> Vec<String> {
    audit_log::table.filter(audit_log::host_id.eq(host)).order(audit_log::seq.asc()).select(audit_log::action).load(c).unwrap()
}

#[actix_web::test]
async fn a_forced_reboot_warns_workstation_users_but_not_servers() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let cookie = admin(&app).await;
    let (ws, srv) = { let mut c = db.conn(); (add_host(&mut c, "workstation"), add_host(&mut c, "server")) };
    for (h, want) in [(ws, 10), (srv, 0)] {
        let (st, body) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{h}/reboot")).cookie(cookie.clone())).await;
        assert_eq!(st, 202, "{body}");
        let exec: Option<serde_json::Value> = tasks::table.filter(tasks::host_id.eq(h)).select(tasks::exec).first(&mut db.conn()).unwrap();
        assert_eq!(exec.unwrap()["delay_minutes"], json!(want));
    }
    let (st, _) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{srv}/reboot")).cookie(cookie.clone()).set_json(json!({"delay_minutes": 5}))).await;
    assert_eq!(st, 409, "one reboot at a time per host");
}

#[actix_web::test]
async fn a_reboot_opens_a_downtime_window_and_closes_it_when_the_host_is_back() {
    let db = support::TestDb::new();
    let state = web::Data::new(db.state());
    let (host, task) = {
        let mut c = db.conn();
        let h = add_host(&mut c, "server");
        let t: Uuid = diesel::insert_into(tasks::table).values((tasks::host_id.eq(h), tasks::kind.eq("reboot"), tasks::status.eq("sent"), tasks::exec.eq(Some(json!({"delay_minutes": 2}))))).returning(tasks::id).get_result(&mut c).unwrap();
        (h, t)
    };
    let before = Utc::now();
    gateway::handle_messages(&state, host, vec![update(task, TaskStatus::Running, "rebooting now")]).await;
    let until: Option<chrono::DateTime<Utc>> = hosts::table.find(host).select(hosts::downtime_until).first(&mut db.conn()).unwrap();
    let until = until.expect("downtime window opened");
    assert!(until >= before + Duration::minutes(2 + reboot::TIMEOUT_MINUTES) - Duration::seconds(5) && until <= Utc::now() + Duration::minutes(2 + reboot::TIMEOUT_MINUTES));

    gateway::handle_messages(&state, host, vec![update(task, TaskStatus::Succeeded, "host back after reboot (48 s)")]).await;
    let (until, reboot_req, status): (Option<chrono::DateTime<Utc>>, bool, String) = hosts::table.find(host).select((hosts::downtime_until, hosts::reboot_required, hosts::status)).first(&mut db.conn()).unwrap();
    assert_eq!((until, reboot_req, status.as_str()), (None, false, "online"));
    let a = actions(&mut db.conn(), host);
    assert!(a.contains(&"reboot.started".into()) && a.contains(&"reboot.completed".into()), "{a:?}");
}

#[actix_web::test]
async fn a_host_that_does_not_come_back_fails_its_reboot_and_raises_the_alarm() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let host = add_host(&mut c, "server");
    let task: Uuid = diesel::insert_into(tasks::table).values((tasks::host_id.eq(host), tasks::kind.eq("reboot"), tasks::status.eq("running"))).returning(tasks::id).get_result(&mut c).unwrap();
    reboot::overdue(&mut c, host, Utc::now()).unwrap();
    let (st, msg): (String, String) = tasks::table.find(task).select((tasks::status, tasks::message)).first(&mut c).unwrap();
    assert_eq!(st, "failed");
    assert!(msg.contains("did not come back"));
    assert!(actions(&mut c, host).contains(&"reboot.overdue".into()));
}

#[actix_web::test]
async fn a_policy_in_auto_mode_reboots_after_a_patch_inside_its_window() {
    let db = support::TestDb::new();
    let state = web::Data::new(db.state());
    let (host, patch) = {
        let mut c = db.conn();
        let h = add_host(&mut c, "server");
        let def = json!({"reboot": {"mode": "auto"}, "maintenance_window": {"days": [], "start": "02:00", "end": "03:00"}});
        let p: Uuid = diesel::insert_into(policies::table).values((policies::name.eq("p"), policies::definition.eq(def))).returning(policies::id).get_result(&mut c).unwrap();
        let t: Uuid = diesel::insert_into(tasks::table).values((tasks::host_id.eq(h), tasks::kind.eq("patch"), tasks::status.eq("running"), tasks::policy_id.eq(Some(p)))).returning(tasks::id).get_result(&mut c).unwrap();
        (h, t)
    };
    let msg = AgentMessage::TaskUpdate(TaskUpdate { task_id: patch, status: TaskStatus::Succeeded, message: "3 package(s) updated".into(), log: vec![], changes: vec![], reboot_required: true });
    gateway::handle_messages(&state, host, vec![msg]).await;
    let (kind, status, when, parent): (String, String, chrono::DateTime<Utc>, Option<Uuid>) = tasks::table.filter(tasks::host_id.eq(host)).filter(tasks::kind.eq("reboot")).select((tasks::kind, tasks::status, tasks::scheduled_for, tasks::parent_task_id)).first(&mut db.conn()).expect("a reboot was scheduled");
    assert_eq!((kind.as_str(), status.as_str(), parent), ("reboot", "queued", Some(patch)));
    use chrono::Timelike;
    assert_eq!(when.hour(), 2, "scheduled inside the maintenance window, not right away: {when}");
}
