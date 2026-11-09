//! Tests for `model`.
use croustypatch_common::model::*;

#[test]
fn package_names() {
    assert!(is_valid_package_name("libssl3"));
    assert!(is_valid_package_name("g++"));
    assert!(!is_valid_package_name("-oFoo"));
    assert!(!is_valid_package_name("a;rm -rf /"));
    assert!(!is_valid_package_name("$(id)"));
    assert!(!is_valid_package_name(""));
}

#[test]
fn enums_roundtrip() {
    assert_eq!(TaskStatus::parse("rolled_back"), Some(TaskStatus::RolledBack));
    assert_eq!(serde_json::to_string(&PolicyAction::AutoDeploy).unwrap(), "\"AUTO_DEPLOY\"");
    assert!(RiskLevel::Critical.rank() > RiskLevel::High.rank());
    assert_eq!(RiskLevel::from_cvss(9.8), RiskLevel::Critical);
}
