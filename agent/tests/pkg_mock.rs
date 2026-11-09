//! Tests for `pkg::mock`.
use std::collections::HashMap;
use uuid::Uuid;
use croustypatch_agent::pkg::mock::*;

#[tokio::test]
async fn apply_then_restore_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db.json");
    let db = MockDb { installed: vec![MockPkg { name: "libssl3".into(), version: "1".into(), candidate: Some("2".into()), is_security: true, cves: vec![] }, MockPkg { name: "linux".into(), version: "6.1".into(), candidate: Some("6.2".into()), is_security: false, cves: vec![] }], ..Default::default() };
    std::fs::write(&path, serde_json::to_string(&db).unwrap()).unwrap();
    let m = Mock::new(path);
    let before: HashMap<String, String> = m.inventory().await.unwrap().into_iter().map(|p| (p.name, p.version)).collect();
    let rp = m.snapshot(Uuid::new_v4(), &["libssl3".into(), "linux".into()], &before).unwrap();
    m.apply(&["libssl3".into(), "linux".into()]).await.unwrap();
    let inv = m.inventory().await.unwrap();
    assert_eq!(inv[0].version, "2");
    assert!(inv[0].candidate.is_none());
    assert!(m.reboot_required().await, "kernel update sets the reboot flag");
    m.restore(&rp).await.unwrap();
    let inv = m.inventory().await.unwrap();
    assert_eq!((inv[0].version.as_str(), inv[0].candidate.as_deref()), ("1", Some("2")));
}
