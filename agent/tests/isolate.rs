//! Tests for `isolate`.
use croustypatch_agent::config::Config;
use croustypatch_agent::exec::Runner;
use croustypatch_agent::runner::TaskLog;
use croustypatch_agent::isolate::*;
use croustypatch_common::protocol::IsolationSettings;

fn closed() -> IsolationSettings {
    IsolationSettings { keep_ssh: false, ..Default::default() }
}

#[test]
fn nft_ruleset_only_allows_server() {
    let r = nft_ruleset(&["192.0.2.10".parse().unwrap(), "2001:db8::1".parse().unwrap()], &closed());
    assert!(r.contains("policy drop"));
    assert!(r.contains("ip saddr { 192.0.2.10 } accept"));
    assert!(r.contains("ip6 daddr { 2001:db8::1 } accept"));
    assert!(r.contains("type filter hook input priority -300"));
    // an IPv4-only server must not generate an empty ip6 set (invalid nft syntax)
    let r4 = nft_ruleset(&["192.0.2.10".parse().unwrap()], &closed());
    assert!(!r4.contains("dport"), "SSH closed when asked");
    assert!(!r4.contains("ip6"));
}

#[tokio::test]
async fn refuses_quarantine_without_server_address() {
    let r = Runner { dry_run: true };
    assert!(quarantine(&r, &[], &IsolationSettings::default(), &TaskLog::default()).await.is_err());
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

#[test]
fn ssh_stays_reachable_during_isolation_by_default() {
    let server = ["192.0.2.10".parse().unwrap()];
    let r = nft_ruleset(&server, &IsolationSettings::default());
    assert!(r.contains("tcp dport 22 accept"), "{r}");
    let input = r.split("chain output").next().unwrap();
    assert!(input.contains("tcp dport 22"), "inbound only: replies leave through the established rule");
    assert!(describe(&server, &IsolationSettings::default()).contains("inbound SSH (port 22)"));
}

#[test]
fn ssh_can_be_limited_to_admin_networks() {
    let server = ["192.0.2.10".parse().unwrap()];
    let iso = IsolationSettings { keep_ssh: true, ssh_port: 2222, ssh_sources: vec!["10.0.10.0/24".into(), "10.0.15.10".into(), "2001:db8::/32".into()] };
    let r = nft_ruleset(&server, &iso);
    assert!(r.contains("ip saddr { 10.0.10.0/24, 10.0.15.10/32 } tcp dport 2222 accept"), "{r}");
    assert!(r.contains("ip6 saddr { 2001:db8::/32 } tcp dport 2222 accept"), "{r}");
    assert!(!r.contains("    tcp dport 2222 accept"), "never open to everyone when sources are given");
    // malformed sources never reach the firewall, and a list with nothing valid keeps SSH closed
    let bad = IsolationSettings { keep_ssh: true, ssh_port: 22, ssh_sources: vec!["10.0.0.0/24; flush ruleset".into(), "x".into()] };
    let r = nft_ruleset(&server, &bad);
    assert!(!r.contains("dport") && !r.contains("flush"), "{r}");
}
