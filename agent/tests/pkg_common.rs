//! Tests for `pkg`.
use std::collections::HashMap;
use croustypatch_agent::pkg::*;

#[test]
fn diff_detects_upgrades_installs_and_removals() {
    let b: HashMap<String, String> = [("a", "1"), ("b", "1"), ("gone", "1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let a: HashMap<String, String> = [("a", "1"), ("b", "2"), ("new", "1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let d = diff(&b, &a);
    let names: Vec<_> = d.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["b", "gone", "new"]);
    assert_eq!(d[0].to.as_deref(), Some("2"));
    assert_eq!(d[1].to, None);
    assert_eq!(d[2].from, None);
}

#[test]
fn arrow_lines() {
    let v = parse_arrow_updates("linux 6.1.1-1 -> 6.1.2-1\ngarbage\nopenssl 3.0.7-1 -> 3.0.8-1\n");
    assert_eq!(v, [("linux".into(), "6.1.1-1".into(), "6.1.2-1".into()), ("openssl".into(), "3.0.7-1".into(), "3.0.8-1".into())]);
}
