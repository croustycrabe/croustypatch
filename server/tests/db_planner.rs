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

// ------------------------------------------------------------------ patch days (the policies seen in production)

fn at(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// A docker host in one group, three policies at the same priority: a security-only one that sorts first by name,
/// a weekly Saturday patch day for everything, and a Tuesday test window. Only ordinary updates are pending.
fn production_like(c: &mut PgConnection) -> (Uuid, Uuid) {
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("docker-service"), hosts::status.eq("online"), hosts::public_key_pem.eq("pem"), hosts::environment.eq("production"))).returning(hosts::id).get_result(c).unwrap();
    let g: Uuid = diesel::insert_into(host_groups::table).values(host_groups::name.eq("docker-host")).returning(host_groups::id).get_result(c).unwrap();
    diesel::insert_into(host_group_members::table).values((host_group_members::host_id.eq(host), host_group_members::group_id.eq(g))).execute(c).unwrap();
    let base = |security_only: bool, window: serde_json::Value| json!({ "security_only": security_only, "default_action": "AUTO_DEPLOY", "maintenance_window": window, "rollback": { "soak_minutes": 30, "probes": [], "auto_rollback": true, "snapshot_required": true } });
    let mut monthly = Uuid::nil();
    for (name, def) in [
        ("Auto update docker hosts on high CVE", base(true, json!(null))),
        ("Monthly update docker host", base(false, json!({ "days": [5], "start": "02:00", "end": "05:00", "utc_offset_minutes": 0 }))),
        ("Test update", base(false, json!({ "days": [1], "start": "16:15", "end": "16:45", "utc_offset_minutes": 0 }))),
    ] {
        let id: Uuid = diesel::insert_into(policies::table).values((policies::name.eq(name), policies::definition.eq(def))).returning(policies::id).get_result(c).unwrap();
        diesel::insert_into(policy_groups::table).values((policy_groups::policy_id.eq(id), policy_groups::group_id.eq(g))).execute(c).unwrap();
        if name.starts_with("Monthly") {
            monthly = id;
        }
    }
    for (n, old, new) in [("docker-compose-plugin", "5.7.0", "5.7.1"), ("snapd", "2.77.1", "2.78")] {
        diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(host), host_packages::name.eq(n), host_packages::version.eq(old), host_packages::candidate.eq(Some(new)), host_packages::is_security.eq(false))).execute(c).unwrap();
    }
    (host, monthly)
}

#[test]
fn a_policy_with_nothing_to_do_does_not_hide_the_next_one() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let (host, monthly) = production_like(&mut c);
    // Saturday 2026-11-14, 03:00 UTC: inside the monthly policy's window. The security-only policy comes first by
    // name and has nothing to take; it used to end planning for the host.
    let id = planner::plan_for_host(&mut c, host, at("2026-11-14T03:00:00Z")).unwrap().expect("the Saturday policy plans the updates");
    let (status, policy, by, n, deadline): (String, Option<Uuid>, String, serde_json::Value, Option<chrono::DateTime<Utc>>) =
        tasks::table.find(id).select((tasks::status, tasks::policy_id, tasks::requested_by, tasks::packages, tasks::deadline_at)).first(&mut c).unwrap();
    assert_eq!((status.as_str(), policy, by.as_str()), ("queued", Some(monthly), "policy:Monthly update docker host"));
    assert_eq!(n.as_array().unwrap().len(), 2);
    assert_eq!(deadline, Some(at("2026-11-14T05:00:00Z")), "the task must not start after its patch day");
}

#[test]
fn automatic_patches_wait_for_their_patch_day() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let (host, _) = production_like(&mut c);
    // Wednesday: no window is open, nothing is planned, and nothing is queued weeks ahead to expire or go stale
    assert!(planner::plan_for_host(&mut c, host, at("2026-11-11T12:00:00Z")).unwrap().is_none());
    let n: i64 = tasks::table.count().get_result(&mut c).unwrap();
    assert_eq!(n, 0);
    let e = planner::explain(&mut c, host, at("2026-11-11T12:00:00Z")).unwrap();
    assert_eq!(e["state"], json!("waiting_for_window"), "{e}");
    assert_eq!(e["window"]["next"][0]["start"], json!("2026-11-14T02:00:00Z"));
    assert_eq!(e["policies"][0]["matching"], json!(0), "the security-only policy takes nothing");
    assert_eq!(e["policies"][0]["not_security"], json!(2));
    // Saturday 03:00: the window is open
    assert_eq!(planner::explain(&mut c, host, at("2026-11-14T03:00:00Z")).unwrap()["state"], json!("will_plan"));
    assert!(planner::plan_for_host(&mut c, host, at("2026-11-14T03:00:00Z")).unwrap().is_some());
}

#[test]
fn a_task_that_missed_its_patch_day_is_not_sent_late_and_is_planned_again() {
    use croustypatch_server::gateway;
    let db = support::TestDb::new();
    let mut c = db.conn();
    let (host, _) = production_like(&mut c);
    let id = planner::plan_for_host(&mut c, host, at("2026-11-14T03:00:00Z")).unwrap().unwrap();
    // the host reconnects after the window closed: the order is not handed out
    let now = chrono::Utc::now();
    diesel::update(tasks::table.find(id)).set((tasks::scheduled_for.eq(now - chrono::Duration::hours(3)), tasks::deadline_at.eq(Some(now - chrono::Duration::hours(1))))).execute(&mut c).unwrap();
    let state = db.state();
    assert!(gateway::claim_due_commands(&mut c, &state, host).unwrap().is_empty(), "the window closed an hour ago");
    // while still inside its window it is
    diesel::update(tasks::table.find(id)).set(tasks::deadline_at.eq(Some(chrono::Utc::now() + chrono::Duration::hours(1)))).execute(&mut c).unwrap();
    assert_eq!(gateway::claim_due_commands(&mut c, &state, host).unwrap().len(), 1);
}

#[test]
fn a_host_outside_every_policy_is_explained() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("laptop"), hosts::status.eq("online"), hosts::public_key_pem.eq("pem"))).returning(hosts::id).get_result(&mut c).unwrap();
    assert_eq!(planner::explain(&mut c, host, Utc::now()).unwrap()["state"], json!("no_group"));
    let g: Uuid = diesel::insert_into(host_groups::table).values(host_groups::name.eq("Laptop")).returning(host_groups::id).get_result(&mut c).unwrap();
    diesel::insert_into(host_group_members::table).values((host_group_members::host_id.eq(host), host_group_members::group_id.eq(g))).execute(&mut c).unwrap();
    assert_eq!(planner::explain(&mut c, host, Utc::now()).unwrap()["state"], json!("no_policy"), "in a group that no policy is linked to");
    let p: Uuid = diesel::insert_into(policies::table).values((policies::name.eq("scoped"), policies::definition.eq(json!({ "target_scope": "asset_environment == 'dev'" })))).returning(policies::id).get_result(&mut c).unwrap();
    diesel::insert_into(policy_groups::table).values((policy_groups::policy_id.eq(p), policy_groups::group_id.eq(g))).execute(&mut c).unwrap();
    assert_eq!(planner::explain(&mut c, host, Utc::now()).unwrap()["state"], json!("out_of_scope"));
}

#[test]
fn a_missed_patch_day_is_recorded_and_stale_work_is_judged_by_its_due_date() {
    use croustypatch_server::schema::audit_log;
    let db = support::TestDb::new();
    let mut c = db.conn();
    let (host, _) = production_like(&mut c);
    let now = Utc::now();
    let mk = |c: &mut PgConnection, scheduled: chrono::DateTime<Utc>, deadline: Option<chrono::DateTime<Utc>>, created: chrono::DateTime<Utc>| -> Uuid {
        diesel::insert_into(tasks::table).values((tasks::host_id.eq(host), tasks::kind.eq("patch"), tasks::status.eq("queued"), tasks::scheduled_for.eq(scheduled), tasks::deadline_at.eq(deadline), tasks::created_at.eq(created))).returning(tasks::id).get_result(c).unwrap()
    };
    let missed = mk(&mut c, now - chrono::Duration::hours(5), Some(now - chrono::Duration::hours(1)), now - chrono::Duration::hours(6));
    // approved three weeks ago for next week's patch day: created long ago, but not due yet, so not stale
    let waiting = mk(&mut c, now + chrono::Duration::days(6), Some(now + chrono::Duration::days(6) + chrono::Duration::hours(3)), now - chrono::Duration::days(15));
    // genuinely stale: due eight days ago, never sent
    let stale = mk(&mut c, now - chrono::Duration::days(8), None, now - chrono::Duration::days(8));
    croustypatch_server::scheduler::housekeeping(&mut c).unwrap();
    let st = |c: &mut PgConnection, id: Uuid| -> String { tasks::table.find(id).select(tasks::status).first(c).unwrap() };
    assert_eq!((st(&mut c, missed), st(&mut c, waiting), st(&mut c, stale)), ("expired".into(), "queued".into(), "expired".into()));
    let audited: i64 = audit_log::table.filter(audit_log::action.eq("task.missed_window")).count().get_result(&mut c).unwrap();
    assert_eq!(audited, 1, "only the task that missed its patch day");
}

#[test]
fn a_policy_waiting_for_its_patch_day_does_not_hold_back_another_policys_patch_day() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let (host, _) = production_like(&mut c);
    // Tuesday 2026-11-10, 16:20 UTC: the "Test update" window (16:15-16:45) is open. "Monthly update docker host"
    // sorts first and is waiting for Saturday: it must not stop the Tuesday policy from patching.
    let e = planner::explain(&mut c, host, at("2026-11-10T16:20:00Z")).unwrap();
    assert_eq!(e["state"], json!("will_plan"), "{e}");
    let id = planner::plan_for_host(&mut c, host, at("2026-11-10T16:20:00Z")).unwrap().expect("the Tuesday policy plans the updates");
    let (by, deadline): (String, Option<chrono::DateTime<Utc>>) = tasks::table.find(id).select((tasks::requested_by, tasks::deadline_at)).first(&mut c).unwrap();
    assert_eq!(by, "policy:Test update");
    assert_eq!(deadline, Some(at("2026-11-10T16:45:00Z")));
}

#[test]
fn a_policy_decides_whether_container_updates_belong_to_it() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let (host, _) = production_like(&mut c);
    diesel::insert_into(host_packages::table)
        .values((host_packages::host_id.eq(host), host_packages::name.eq("docker:hoppscotch-db"), host_packages::source.eq("docker"), host_packages::version.eq("postgres:17 @3540"), host_packages::candidate.eq(Some("postgres:17 @e5d2"))))
        .execute(&mut c)
        .unwrap();
    let sat = at("2026-11-14T03:00:00Z");
    let take = |c: &mut PgConnection, mode: &str| -> Vec<String> {
        diesel::sql_query(format!("UPDATE policies SET definition = jsonb_set(definition, '{{docker}}', '\"{mode}\"') WHERE name = 'Monthly update docker host'")).execute(c).unwrap();
        let e = planner::explain(c, host, sat).unwrap();
        let p = e["policies"].as_array().unwrap().iter().find(|p| p["name"] == "Monthly update docker host").unwrap().clone();
        let _ = e;
        (0..p["matching"].as_u64().unwrap()).map(|_| String::new()).collect()
    };
    assert_eq!(take(&mut c, "include").len(), 3, "policies saved before the option keep taking containers with the system updates");
    assert_eq!(take(&mut c, "exclude").len(), 2, "containers left alone");
    assert_eq!(take(&mut c, "only").len(), 1, "a dedicated Docker policy takes nothing else");
    // and what it plans: exclude -> no container in the task
    take(&mut c, "exclude");
    let id = planner::plan_for_host(&mut c, host, sat).unwrap().unwrap();
    let pk: serde_json::Value = tasks::table.find(id).select(tasks::packages).first(&mut c).unwrap();
    assert!(pk.as_array().unwrap().iter().all(|p| !p["name"].as_str().unwrap().starts_with("docker:")), "{pk}");
    // a policy that is not told anything about Docker (stored by an older version) includes it
    let d: croustypatch_server::engine::policy::PolicyDef = serde_json::from_str("{}").unwrap();
    assert!(d.docker.takes("docker") && d.docker.takes("system"));
}
