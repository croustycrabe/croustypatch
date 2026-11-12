//! Integration: OpenSCAP reports and the module switches that govern agent data.
mod support;

use actix_web::test::TestRequest;
use actix_web::web;
use chrono::Utc;
use croustypatch_common::protocol::{AgentMessage, ComplianceReport, FailedRule, HostFacts, Inventory, PackageEntry};
use croustypatch_server::gateway;
use croustypatch_server::schema::{audit_log, compliance_reports, host_packages, hosts, tasks};
use diesel::prelude::*;
use serde_json::json;
use support::http::{admin, app, call};
use uuid::Uuid;

fn add_host(c: &mut PgConnection) -> Uuid {
    diesel::insert_into(hosts::table).values((hosts::hostname.eq("srv-1"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(c).unwrap()
}

fn report(score: f64) -> ComplianceReport {
    ComplianceReport { profile: "xccdf_org.ssgproject.content_profile_standard".into(), datastream: "/usr/share/xml/scap/ssg/content/ssg-debian12-ds.xml".into(), passed: 70, failed: 30, not_applicable: 5, other: 0, score, failed_rules: vec![FailedRule { rule_id: "rule_a".into(), severity: "high".into() }], results_sha256: "ab".repeat(32), collected_at: Utc::now() }
}

#[actix_web::test]
async fn compliance_reports_are_stored_scored_and_immutable() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let host = add_host(&mut db.conn());
    let scan = || TestRequest::post().uri(&format!("/api/v1/hosts/{host}/compliance-scan")).cookie(ck.clone()).set_json(json!({}));
    let (st, _) = call(&app, scan()).await;
    assert_eq!(st, 409, "OpenSCAP is off for this host");
    let (st, _) = call(&app, TestRequest::patch().uri(&format!("/api/v1/hosts/{host}")).cookie(ck.clone()).set_json(json!({ "modules": { "openscap": true } }))).await;
    assert_eq!(st, 200);
    let (st, r) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{host}/compliance-scan")).cookie(ck.clone()).set_json(json!({ "profile": "xccdf_org.ssgproject.content_profile_cis" }))).await;
    assert_eq!(st, 202, "{r}");
    let task: Uuid = r["task_id"].as_str().unwrap().parse().unwrap();
    let exec: Option<serde_json::Value> = tasks::table.find(task).select(tasks::exec).first(&mut db.conn()).unwrap();
    assert_eq!(exec.unwrap()["profile"], json!("xccdf_org.ssgproject.content_profile_cis"));
    let (st, _) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{host}/compliance-scan")).cookie(ck.clone()).set_json(json!({ "profile": "x; rm -rf /" }))).await;
    assert_eq!(st, 400, "profile ids are validated");

    gateway::handle_messages(&state, host, vec![AgentMessage::Compliance { task_id: task, report: report(70.0) }]).await;
    let (score, failed): (f64, i32) = compliance_reports::table.select((compliance_reports::score, compliance_reports::failed)).first(&mut db.conn()).unwrap();
    assert_eq!((score, failed), (70.0, 30));
    let actions: Vec<String> = audit_log::table.filter(audit_log::host_id.eq(host)).select(audit_log::action).load(&mut db.conn()).unwrap();
    assert!(actions.contains(&"compliance.score_low".to_string()), "70% is below the default 80% threshold: {actions:?}");

    // a report for someone else's task is refused
    let other = diesel::insert_into(hosts::table).values((hosts::hostname.eq("srv-2"), hosts::public_key_pem.eq("pem"))).returning(hosts::id).get_result::<Uuid>(&mut db.conn()).unwrap();
    gateway::handle_messages(&state, other, vec![AgentMessage::Compliance { task_id: task, report: report(100.0) }]).await;
    let n: i64 = compliance_reports::table.count().get_result(&mut db.conn()).unwrap();
    assert_eq!(n, 1);

    assert!(diesel::update(compliance_reports::table).set(compliance_reports::score.eq(100.0)).execute(&mut db.conn()).is_err(), "evidence cannot be edited");

    let (_, fleet) = call(&app, TestRequest::get().uri("/api/v1/compliance").cookie(ck.clone())).await;
    assert_eq!(fleet["score"], json!(70.0));
    let (_, dash) = call(&app, TestRequest::get().uri("/api/v1/dashboard").cookie(ck.clone())).await;
    assert_eq!(dash["compliance"]["score"], json!(70.0));
}

#[actix_web::test]
async fn container_inventory_is_kept_only_while_the_docker_module_is_on() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let host = add_host(&mut db.conn());
    let pkg = |name: &str, source: &str, cand: Option<&str>| PackageEntry { name: name.into(), version: "1".into(), candidate: cand.map(str::to_owned), is_security: false, cves: vec![], source: source.into(), meta: Default::default() };
    let inv = || AgentMessage::Inventory(Inventory { facts: HostFacts { hostname: "srv-1".into(), ..Default::default() }, packages: vec![pkg("bash", "system", None), pkg("docker:web", "docker", Some("2")), pkg("helper:vaultwarden", "helper-script", None)], reboot_required: false, collected_at: Utc::now(), app_updates: vec![] });
    let sources = |db: &support::TestDb| -> Vec<String> { let mut v: Vec<String> = host_packages::table.filter(host_packages::host_id.eq(host)).select(host_packages::source).load(&mut db.conn()).unwrap(); v.sort(); v };

    gateway::handle_messages(&state, host, vec![inv()]).await;
    assert_eq!(sources(&db), vec!["system"], "modules are off by default");

    let (st, _) = call(&app, TestRequest::patch().uri(&format!("/api/v1/hosts/{host}")).cookie(ck.clone()).set_json(json!({ "modules": { "docker": true, "helper_script": true } }))).await;
    assert!(st.is_success());
    gateway::handle_messages(&state, host, vec![inv(), inv()]).await;
    assert_eq!(sources(&db), vec!["docker", "helper-script", "system"], "and every source is replaced, never duplicated");
}

/// Content written for another release (Ubuntu 26.04 audited with the 24.04 datastream) marks every rule "not applicable".
/// That is not a 0 % score: no alert, and the host stays out of the fleet score.
#[actix_web::test]
async fn an_audit_in_which_no_rule_applies_raises_no_alert_and_has_no_score() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let state = web::Data::new(db.state());
    let (good, empty) = {
        let mut c = db.conn();
        let a = add_host(&mut c);
        let b: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("ubuntu-26"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(&mut c).unwrap();
        diesel::update(hosts::table.filter(hosts::id.eq_any([a, b]))).set(hosts::modules.eq(json!({"openscap": true}))).execute(&mut c).unwrap();
        (a, b)
    };
    let task_for = |h: Uuid| {
        let t: Uuid = diesel::insert_into(tasks::table).values((tasks::host_id.eq(h), tasks::kind.eq("compliance"), tasks::status.eq("running"), tasks::requested_by.eq("admin"))).returning(tasks::id).get_result(&mut db.conn()).unwrap();
        t
    };
    let (t1, t2) = (task_for(good), task_for(empty));
    gateway::handle_messages(&state, good, vec![AgentMessage::Compliance { task_id: t1, report: report(70.0) }]).await;
    let mut none = report(0.0);
    none.passed = 0;
    none.failed = 0;
    none.not_applicable = 405;
    none.datastream = "/usr/share/xml/scap/ssg/content/ssg-ubuntu2404-ds.xml".into();
    gateway::handle_messages(&state, empty, vec![AgentMessage::Compliance { task_id: t2, report: none }]).await;
    let stored: i64 = compliance_reports::table.filter(compliance_reports::host_id.eq(empty)).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(stored, 1, "the audit is kept as evidence");
    let (action, details): (String, serde_json::Value) = audit_log::table.filter(audit_log::host_id.eq(empty)).filter(audit_log::action.like("compliance.%")).select((audit_log::action, audit_log::details)).first(&mut db.conn()).unwrap();
    assert_eq!(action, "compliance.no_applicable_rules", "not compliance.score_low: nothing was found wrong");
    assert!(details["message"].as_str().unwrap().contains("ssg-ubuntu2404-ds.xml") && details["message"].as_str().unwrap().contains("does not cover this release"), "{details}");
    let (_, fleet) = call(&app, TestRequest::get().uri("/api/v1/compliance").cookie(ck.clone())).await;
    assert_eq!(fleet["score"], json!(70.0), "the fleet score is the other host's, not the mean of 70 and 0: {fleet}");
    assert_eq!(fleet["hosts"].as_array().unwrap().len(), 2, "the audit still shows on the Compliance page");
}
