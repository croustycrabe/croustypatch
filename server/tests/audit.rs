//! Tests for `audit`.
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;
use croustypatch_server::audit::*;

#[test]
fn hash_depends_on_every_field() {
    let ts = Utc::now();
    let base = entry_hash("p", ts, "a", "x", "t", "1", None, &json!({"k": 1}), "ip");
    assert_eq!(base, entry_hash("p", ts, "a", "x", "t", "1", None, &json!({"k": 1}), "ip"));
    assert_ne!(base, entry_hash("q", ts, "a", "x", "t", "1", None, &json!({"k": 1}), "ip"));
    assert_ne!(base, entry_hash("p", ts, "b", "x", "t", "1", None, &json!({"k": 1}), "ip"));
    assert_ne!(base, entry_hash("p", ts, "a", "x", "t", "1", None, &json!({"k": 2}), "ip"));
    assert_ne!(base, entry_hash("p", ts, "a", "x", "t", "1", Some(Uuid::nil()), &json!({"k": 1}), "ip"));
    // key order in input does not matter
    assert_eq!(entry_hash("p", ts, "a", "x", "t", "1", None, &json!({"a": 1, "b": 2}), ""), entry_hash("p", ts, "a", "x", "t", "1", None, &json!({"b": 2, "a": 1}), ""));
}
