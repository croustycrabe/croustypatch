//! Tests for `engine::expr`.
use croustypatch_server::engine::expr::*;

fn ctx() -> Context {
    let mut c = Context::new();
    c.insert("cvss_score".into(), 9.1.into());
    c.insert("asset_environment".into(), "production".into());
    c.insert("port_exposed_to_internet".into(), true.into());
    c.insert("has_staging".into(), false.into());
    c.insert("tags".into(), vec!["pci".to_string(), "web".to_string()].into());
    c
}

#[test]
fn spec_example_rule() {
    let r = "cvss_score >= 8.0 AND asset_environment == 'production' AND port_exposed_to_internet == true";
    assert!(eval_str(r, &ctx()).unwrap());
    let mut c = ctx();
    c.insert("cvss_score".into(), 7.9.into());
    assert!(!eval_str(r, &c).unwrap());
}

#[test]
fn precedence_not_in_and_bare_bool() {
    let c = ctx();
    assert!(eval_str("'pci' in tags AND NOT has_staging", &c).unwrap());
    assert!(eval_str("has_staging OR port_exposed_to_internet", &c).unwrap());
    assert!(eval_str("has_staging == false || cvss_score < 1", &c).unwrap());
    // AND binds tighter than OR
    assert!(eval_str("false OR true AND true", &c).unwrap());
    assert!(!eval_str("(false OR true) AND false", &c).unwrap());
    assert!(eval_str("asset_environment == 'PRODUCTION'", &c).unwrap(), "string compare is case-insensitive");
}

#[test]
fn unknown_identifier_never_matches() {
    let c = ctx();
    assert!(!eval_str("nope == 1", &c).unwrap());
    assert!(!eval_str("nope != 1", &c).unwrap());
    assert!(eval_str("NOT nope", &c).unwrap());
}

#[test]
fn syntax_errors_are_reported_with_position() {
    for bad in ["", "cvss_score >=", "a = 1", "(a == 1", "a == 1 b", "'open", "a & b", "a @ b"] {
        assert!(parse(bad).is_err(), "{bad:?} should not parse");
    }
    assert_eq!(parse("a == 1 b").unwrap_err().pos, 7);
}
