//! Tests for `engine::risk`.
use croustypatch_common::model::{PolicyAction, RiskLevel};
use croustypatch_server::engine::policy::PolicyDef;
use croustypatch_server::engine::risk::*;
use croustypatch_server::engine::policy::{DecisionConditions, DecisionRule, RiskRule};

fn input() -> AssessInput {
    AssessInput {
        max_cvss: 9.1,
        security_count: 1,
        package_count: 1,
        asset_environment: "production".into(),
        asset_type: "server".into(),
        criticality: "low".into(),
        internet_exposed: true,
        has_staging: true,
        ..Default::default()
    }
}

fn spec_policy() -> PolicyDef {
    PolicyDef {
        rules: vec![RiskRule {
            rule_id: "rule_no_ai_01".into(),
            condition: "cvss_score >= 8.0 AND asset_environment == 'production' AND port_exposed_to_internet == true".into(),
            contextual_risk: RiskLevel::Critical,
        }],
        decisions: vec![DecisionRule {
            conditions: DecisionConditions { contextual_risk: Some(RiskLevel::Critical), stability_risk: Some(RiskLevel::Low), ..Default::default() },
            action: PolicyAction::AutoDeploy,
        }],
        ..Default::default()
    }
}

#[test]
fn critical_low_stability_auto_deploys() {
    let a = assess_deterministic(&spec_policy(), &input());
    assert_eq!(a.contextual_risk, RiskLevel::Critical);
    assert_eq!(a.stability_risk, RiskLevel::Low);
    assert_eq!(a.action, PolicyAction::AutoDeploy);
    assert_eq!(a.matched_rule.as_deref(), Some("rule_no_ai_01"));
}

#[test]
fn unstable_change_needs_approval() {
    let mut i = input();
    i.includes_kernel = true;
    i.critical_packages = vec!["linux".into(), "systemd".into(), "openssl".into()];
    i.criticality = "high".into();
    i.has_staging = false;
    let a = assess_deterministic(&spec_policy(), &i);
    assert!(a.stability_risk.rank() >= RiskLevel::High.rank());
    assert_eq!(a.action, PolicyAction::RequireApproval, "falls to default action");
}

#[test]
fn fallback_from_cvss_without_rules() {
    let mut i = input();
    i.internet_exposed = false;
    i.max_cvss = 5.0;
    let (l, rule, _) = contextual_deterministic(&PolicyDef::default(), &i);
    assert_eq!((l, rule), (RiskLevel::Medium, None));
    i.internet_exposed = true;
    assert_eq!(contextual_deterministic(&PolicyDef::default(), &i).0, RiskLevel::High);
    i.internet_exposed = false;
    i.max_cvss = 2.0;
    i.any_exploited = true;
    assert_eq!(contextual_deterministic(&PolicyDef::default(), &i).0, RiskLevel::High);
}

#[test]
fn block_decision() {
    let mut p = PolicyDef::default();
    p.decisions.push(DecisionRule { conditions: DecisionConditions { stability_risk_max: None, contextual_risk_min: Some(RiskLevel::Low), ..Default::default() }, action: PolicyAction::Block });
    assert_eq!(assess_deterministic(&p, &input()).action, PolicyAction::Block);
}
