//! Tests for `planner`.
use chrono::{DateTime, Duration, Utc};
use croustypatch_server::engine::policy::PolicyDef;
use croustypatch_server::planner::*;
use croustypatch_server::engine::policy::MaintenanceWindow;

#[test]
fn batch_key_is_order_independent_and_version_sensitive() {
    let a = batch_key(&[("a".into(), "1".into()), ("b".into(), "2".into())]);
    assert_eq!(a, batch_key(&[("b".into(), "2".into()), ("a".into(), "1".into())]));
    assert_ne!(a, batch_key(&[("a".into(), "1".into()), ("b".into(), "3".into())]));
}

#[test]
fn schedule_respects_window_and_rings() {
    let now = DateTime::parse_from_rfc3339("2026-11-06T12:00:00Z").unwrap().with_timezone(&Utc); // Friday
    let mut def = PolicyDef::default();
    assert_eq!(compute_schedule(Some(&def), 0, now), now);
    def.rings.delay_minutes = 60;
    assert_eq!(compute_schedule(Some(&def), 2, now), now + Duration::minutes(120));
    def.maintenance_window = Some(MaintenanceWindow { days: vec![], start: "02:00".into(), end: "04:00".into(), ..Default::default() });
    assert_eq!(compute_schedule(Some(&def), 0, now), DateTime::parse_from_rfc3339("2026-11-07T02:00:00Z").unwrap());
}

#[test]
fn staging_less_critical_hosts_force_snapshots() {
    let mut def = PolicyDef::default();
    def.rollback.snapshot_required = false;
    assert!(!exec_policy(&def, true, "high").snapshot_required);
    assert!(exec_policy(&def, false, "high").snapshot_required);
    assert!(!exec_policy(&def, false, "low").snapshot_required);
}
