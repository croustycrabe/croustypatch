//! Tests for `isolate`.
use croustypatch_agent::config::Config;
use croustypatch_agent::exec::Runner;
use croustypatch_agent::runner::TaskLog;
use croustypatch_agent::isolate::*;

#[test]
fn nft_ruleset_only_allows_server() {
    let r = nft_ruleset(&["192.0.2.10".parse().unwrap(), "2001:db8::1".parse().unwrap()]);
    assert!(r.contains("policy drop"));
    assert!(r.contains("ip saddr { 192.0.2.10 } accept"));
    assert!(r.contains("ip6 daddr { 2001:db8::1 } accept"));
    assert!(r.contains("type filter hook input priority -300"));
    // an IPv4-only server must not generate an empty ip6 set (invalid nft syntax)
    let r4 = nft_ruleset(&["192.0.2.10".parse().unwrap()]);
    assert!(!r4.contains("ip6"));
}

#[test]
fn pf_rules_block_by_default() {
    let r = pf_rules(&["192.0.2.10".parse().unwrap()]);
    assert!(r.starts_with("block drop all"));
    assert!(r.contains("to { 192.0.2.10 }"));
}

#[tokio::test]
async fn refuses_quarantine_without_server_address() {
    let r = Runner { dry_run: true };
    assert!(quarantine(&r, &[], &TaskLog::default()).await.is_err());
}

#[tokio::test]
async fn forensics_manifest_matches_files() {
    let d = tempfile::tempdir().unwrap();
    let cfg = Config { state_dir: d.path().into(), ..Default::default() };
    let log = TaskLog::default();
    log.line("hello".into());
    let t = uuid::Uuid::new_v4();
    let dir = forensics_freeze(&cfg, &Runner { dry_run: true }, t, &log, &[("restore.json", b"{}".to_vec())]).await.unwrap();
    let manifest = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
    for line in manifest.lines() {
        let (hash, name) = line.split_once("  ").unwrap();
        assert_eq!(hash, croustypatch_common::crypto::sha256_hex(&std::fs::read(dir.join(name)).unwrap()));
    }
    assert!(manifest.contains("task.log") && manifest.contains("restore.json"));
}
