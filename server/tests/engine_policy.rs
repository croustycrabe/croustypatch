//! Tests for `engine::policy`.
use chrono::{DateTime, Utc};
use croustypatch_common::model::{PolicyAction, RiskLevel};
use croustypatch_server::engine::policy::*;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}
fn win(days: Vec<u32>, start: &str, end: &str, off: i32) -> MaintenanceWindow {
    MaintenanceWindow { days, start: start.into(), end: end.into(), utc_offset_minutes: off }
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
