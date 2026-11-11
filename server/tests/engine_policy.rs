//! Tests for `engine::policy`.
use chrono::{DateTime, Utc};
use croustypatch_common::model::{PolicyAction, RiskLevel};
use croustypatch_server::engine::policy::*;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}
fn win(days: Vec<u32>, start: &str, end: &str, off: i32) -> MaintenanceWindow {
    MaintenanceWindow { days, start: start.into(), end: end.into(), utc_offset_minutes: off, ..Default::default() }
}

#[test]
fn spec_policy_json_parses() {
    let j = r#"{
        "ai_mode_enabled": true,
        "target_scope": "asset_environment == 'production'",
        "decisions": [{"conditions": {"contextual_risk": "Critical", "stability_risk": "Low"}, "action": "AUTO_DEPLOY"}],
        "rules": [{"rule_id": "rule_no_ai_01", "condition": "cvss_score >= 8.0 AND asset_environment == 'production' AND port_exposed_to_internet == true", "fallback_contextual_risk": "Critical"}]
    }"#;
    let p: PolicyDef = serde_json::from_str(j).unwrap();
    p.validate().unwrap();
    assert_eq!(p.decisions[0].action, PolicyAction::AutoDeploy);
    assert_eq!(p.default_action, PolicyAction::RequireApproval);
    assert!(p.decisions[0].conditions.matches(RiskLevel::Critical, RiskLevel::Low));
    assert!(!p.decisions[0].conditions.matches(RiskLevel::Critical, RiskLevel::High));
}

#[test]
fn invalid_policy_rejected() {
    let mut p = PolicyDef::default();
    p.rules.push(RiskRule { rule_id: "x".into(), condition: "a ==".into(), contextual_risk: RiskLevel::Low });
    assert!(p.validate().unwrap_err().contains("rule x"));
    let mut p = PolicyDef::default();
    p.maintenance_window = Some(win(vec![9], "02:00", "04:00", 0));
    assert!(p.validate().is_err());
}

#[test]
fn window_same_day() {
    // 2026-11-06 is a Friday (4)
    let w = win(vec![4], "02:00", "04:00", 0);
    assert!(w.contains(at("2026-11-06T03:00:00Z")));
    assert!(!w.contains(at("2026-11-06T04:00:00Z")));
    assert!(!w.contains(at("2026-11-07T03:00:00Z")), "saturday not allowed");
    assert_eq!(w.next_start(at("2026-11-06T05:00:00Z")), at("2026-11-13T02:00:00Z"));
    assert_eq!(w.next_start(at("2026-11-06T01:00:00Z")), at("2026-11-06T02:00:00Z"));
}

#[test]
fn window_overnight_and_offset() {
    let w = win(vec![], "22:00", "02:00", 0);
    assert!(w.contains(at("2026-11-06T23:00:00Z")));
    assert!(w.contains(at("2026-11-07T01:00:00Z")));
    assert!(!w.contains(at("2026-11-07T03:00:00Z")));
    // UTC+1: 02:00 local == 01:00 UTC
    let w = win(vec![], "02:00", "04:00", 60);
    assert!(w.contains(at("2026-11-06T01:30:00Z")));
    assert_eq!(w.next_start(at("2026-11-06T12:00:00Z")), at("2026-11-07T01:00:00Z"));
}

fn monthly(days: Vec<u32>, nth: u32, start: &str, end: &str, tz: &str) -> MaintenanceWindow {
    MaintenanceWindow { days, start: start.into(), end: end.into(), timezone: Some(tz.into()), repeat: croustypatch_server::engine::policy::Repeat::MonthlyWeekday, nth: Some(nth), ..Default::default() }
}

#[test]
fn old_stored_windows_load_as_weekly_windows() {
    let w: MaintenanceWindow = serde_json::from_str(r#"{"end": "05:00", "days": [5], "start": "02:00", "utc_offset_minutes": 0}"#).unwrap();
    assert_eq!(w.repeat, croustypatch_server::engine::policy::Repeat::Weekly);
    assert!(w.timezone.is_none());
    // 2026-11-07 is a Saturday
    assert!(w.contains(at("2026-11-07T03:00:00Z")));
    assert_eq!(w.next_start(at("2026-11-07T06:00:00Z")), at("2026-11-14T02:00:00Z"));
}

#[test]
fn the_first_monday_of_the_month() {
    // 2026-11-02 is the first Monday of November, 2026-12-07 of December.
    let w = monthly(vec![0], 1, "02:00", "04:00", "UTC");
    assert!(w.contains(at("2026-11-02T03:00:00Z")));
    assert!(!w.contains(at("2026-11-09T03:00:00Z")), "the second Monday is not a patch day");
    assert_eq!(w.next_start(at("2026-11-02T05:00:00Z")), at("2026-12-07T02:00:00Z"));
    let up: Vec<_> = w.upcoming(at("2026-11-02T05:00:00Z"), 3).iter().map(|o| o.start).collect();
    assert_eq!(up, vec![at("2026-12-07T02:00:00Z"), at("2027-01-04T02:00:00Z"), at("2027-02-01T02:00:00Z")]);
}

#[test]
fn the_last_friday_and_a_day_of_the_month() {
    let w = monthly(vec![4], 5, "22:00", "02:00", "UTC");
    // the last Friday of November 2026 is the 27th; the window runs overnight into Saturday
    assert!(w.contains(at("2026-11-27T23:00:00Z")));
    assert!(w.contains(at("2026-11-28T01:00:00Z")), "the morning part still belongs to Friday's window");
    assert!(!w.contains(at("2026-11-20T23:00:00Z")));
    assert_eq!(w.occurrence(at("2026-11-28T01:00:00Z")).unwrap().end, at("2026-11-28T02:00:00Z"));
    let d = MaintenanceWindow { repeat: croustypatch_server::engine::policy::Repeat::MonthlyDay, day_of_month: Some(15), start: "03:00".into(), end: "05:00".into(), ..Default::default() };
    assert_eq!(d.next_start(at("2026-11-16T00:00:00Z")), at("2026-12-15T03:00:00Z"));
    assert!(d.contains(at("2026-11-15T04:00:00Z")));
}

#[test]
fn a_named_timezone_follows_daylight_saving_time() {
    let w = monthly(vec![0], 1, "02:00", "04:00", "Europe/Paris");
    // winter: 02:00 CET is 01:00 UTC; summer: 02:00 CEST is 00:00 UTC
    assert_eq!(w.next_start(at("2026-11-03T00:00:00Z")), at("2026-12-07T01:00:00Z"));
    assert_eq!(w.next_start(at("2027-06-01T00:00:00Z")), at("2027-06-07T00:00:00Z"));
    // the clocks go forward on 2027-03-28: 02:30 does not exist, the window opens at the first moment after the gap
    let sunday = MaintenanceWindow { days: vec![6], start: "02:30".into(), end: "04:00".into(), timezone: Some("Europe/Paris".into()), ..Default::default() };
    let o = sunday.occurrence(at("2027-03-28T00:00:00Z")).unwrap();
    assert_eq!((o.start, o.end), (at("2027-03-28T01:30:00Z"), at("2027-03-28T02:00:00Z")));
    // the clocks go back on 2026-10-25: 02:30 happens twice, the window opens at the first one
    let o = sunday.occurrence(at("2026-10-25T00:00:00Z")).unwrap();
    assert_eq!((o.start, o.end), (at("2026-10-25T00:30:00Z"), at("2026-10-25T03:00:00Z")));
}

#[test]
fn bad_schedules_are_rejected_when_saved() {
    let mut p = PolicyDef::default();
    for w in [
        MaintenanceWindow { timezone: Some("Mars/Olympus".into()), ..Default::default() },
        monthly(vec![], 1, "02:00", "04:00", "UTC"),
        monthly(vec![0], 0, "02:00", "04:00", "UTC"),
        monthly(vec![0], 6, "02:00", "04:00", "UTC"),
        MaintenanceWindow { repeat: croustypatch_server::engine::policy::Repeat::MonthlyDay, day_of_month: Some(31), ..Default::default() },
        MaintenanceWindow { repeat: croustypatch_server::engine::policy::Repeat::MonthlyDay, ..Default::default() },
    ] {
        p.maintenance_window = Some(w);
        assert!(p.validate().is_err());
    }
    p.maintenance_window = Some(monthly(vec![0], 1, "02:00", "04:00", "Europe/Paris"));
    p.validate().unwrap();
}
