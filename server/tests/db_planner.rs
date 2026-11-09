//! Integration: the policy planner against a real PostgreSQL.
mod support;

use chrono::Utc;
use croustypatch_server::planner;
use croustypatch_server::schema::{host_group_members, host_groups, host_packages, hosts, policies, policy_groups, tasks, waivers};
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

struct Fixture {
    host: Uuid,
    groups: Vec<Uuid>,
}

fn fixture(c: &mut PgConnection, n_groups: usize, definition: serde_json::Value) -> Fixture {
    let host: Uuid = diesel::insert_into(hosts::table)
        .values((hosts::hostname.eq("web-01"), hosts::status.eq("online"), hosts::public_key_pem.eq("pem"), hosts::environment.eq("production")))
        .returning(hosts::id)
        .get_result(c)
        .unwrap();
    let mut groups = vec![];
    for i in 0..n_groups {
        let g: Uuid = diesel::insert_into(host_groups::table).values(host_groups::name.eq(format!("g{i}"))).returning(host_groups::id).get_result(c).unwrap();
        diesel::insert_into(host_group_members::table).values((host_group_members::host_id.eq(host), host_group_members::group_id.eq(g))).execute(c).unwrap();
        groups.push(g);
    }
    let policy: Uuid = diesel::insert_into(policies::table).values((policies::name.eq("p"), policies::definition.eq(definition))).returning(policies::id).get_result(c).unwrap();
    for g in &groups {
        diesel::insert_into(policy_groups::table).values((policy_groups::policy_id.eq(policy), policy_groups::group_id.eq(g))).execute(c).unwrap();
    }
    diesel::insert_into(host_packages::table)
        .values((host_packages::host_id.eq(host), host_packages::name.eq("libssl3"), host_packages::version.eq("1.0"), host_packages::candidate.eq(Some("1.1")), host_packages::is_security.eq(true), host_packages::cves.eq(json!(["CVE-2026-0001"]))))
        .execute(c)
        .unwrap();
    Fixture { host, groups }
}

/// Regression: `SELECT DISTINCT … ORDER BY priority` is rejected by PostgreSQL when the ORDER BY column is
/// not selected. It made planning fail for every host, and only showed up against a real database.
#[test]
fn policy_linked_to_two_groups_of_the_host_still_plans() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let f = fixture(&mut c, 2, json!({}));
    let id = planner::plan_for_host(&mut c, f.host, Utc::now()).expect("planning must not error").expect("a task is created");
    let (status, requested_by): (String, String) = tasks::table.find(id).select((tasks::status, tasks::requested_by)).first(&mut c).unwrap();
    assert_eq!(status, "pending_approval", "the default action asks for approval");
    assert_eq!(requested_by, "policy:p");
}

#[test]
fn the_same_batch_is_never_planned_twice() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let f = fixture(&mut c, 1, json!({}));
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_some());
    // pending_approval counts as an open task: nothing more is planned
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_none());
    // even once that task is rejected, the identical batch is not re-planned
    diesel::update(tasks::table).set(tasks::status.eq("rejected")).execute(&mut c).unwrap();
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_none());
    // a newer candidate version is a new batch
    diesel::update(host_packages::table).set(host_packages::candidate.eq(Some("1.2"))).execute(&mut c).unwrap();
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_some());
}

#[test]
fn hosts_without_a_linked_group_get_nothing() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let f = fixture(&mut c, 0, json!({}));
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_none());
}

#[test]
fn auto_deploy_decision_queues_and_block_records() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let auto = json!({"decisions": [{"conditions": {"contextual_risk_min": "Low"}, "action": "AUTO_DEPLOY"}]});
    let f = fixture(&mut c, 1, auto);
    let id = planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().unwrap();
    let status: String = tasks::table.find(id).select(tasks::status).first(&mut c).unwrap();
    assert_eq!(status, "queued");

    let db2 = support::TestDb::new();
    let mut c2 = db2.conn();
    let block = json!({"decisions": [{"conditions": {"contextual_risk_min": "Low"}, "action": "BLOCK"}]});
    let f2 = fixture(&mut c2, 1, block);
    let id = planner::plan_for_host(&mut c2, f2.host, Utc::now()).unwrap().unwrap();
    let (status, finished): (String, Option<chrono::DateTime<Utc>>) = tasks::table.find(id).select((tasks::status, tasks::finished_at)).first(&mut c2).unwrap();
    assert_eq!(status, "blocked");
    assert!(finished.is_some(), "a blocked task is terminal from the start");
}

#[test]
fn security_only_ignores_ordinary_updates() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let f = fixture(&mut c, 1, json!({"security_only": true}));
    diesel::update(host_packages::table).set(host_packages::is_security.eq(false)).execute(&mut c).unwrap();
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_none());
}

#[test]
fn an_approved_waiver_suppresses_planning_until_revoked() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let f = fixture(&mut c, 1, json!({}));
    let waiver: Uuid = diesel::insert_into(waivers::table)
        .values((waivers::host_id.eq(Some(f.host)), waivers::cve_id.eq(Some("CVE-2026-0001")), waivers::reason.eq("legacy app pins it"), waivers::compensating_controls.eq("WAF rule 4711"), waivers::requested_by.eq("biz"), waivers::expires_at.eq(Utc::now() + chrono::Duration::days(5)), waivers::status.eq("approved")))
        .returning(waivers::id)
        .get_result(&mut c)
        .unwrap();
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_none(), "waived");
    diesel::update(waivers::table.find(waiver)).set(waivers::status.eq("revoked")).execute(&mut c).unwrap();
    assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_some(), "revoking brings it back");

    // an expired approval no longer protects
    let db2 = support::TestDb::new();
    let mut c2 = db2.conn();
    let f2 = fixture(&mut c2, 1, json!({}));
    diesel::insert_into(waivers::table)
        .values((waivers::host_id.eq(Some(f2.host)), waivers::cve_id.eq(Some("CVE-2026-0001")), waivers::reason.eq("old"), waivers::compensating_controls.eq("none"), waivers::requested_by.eq("biz"), waivers::expires_at.eq(Utc::now() - chrono::Duration::days(1)), waivers::status.eq("approved")))
        .execute(&mut c2)
        .unwrap();
    assert!(planner::plan_for_host(&mut c2, f2.host, Utc::now()).unwrap().is_some());
    let _ = f.groups;
}

#[test]
fn quarantined_and_pending_hosts_are_left_alone() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let f = fixture(&mut c, 1, json!({}));
    for st in ["quarantined", "pending"] {
        diesel::update(hosts::table).set(hosts::status.eq(st)).execute(&mut c).unwrap();
        assert!(planner::plan_for_host(&mut c, f.host, Utc::now()).unwrap().is_none(), "{st}");
    }
}
