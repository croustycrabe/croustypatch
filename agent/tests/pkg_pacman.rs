//! Tests for `pkg::pacman`.
use croustypatch_agent::pkg::pacman::*;

#[test]
fn audit_lines() {
    let t = "Package openssl is affected by CVE-2022-4450, CVE-2022-4304. High risk! Update to 3.0.8-1!\nPackage foo is affected by CVE-2023-0001. Low risk!\nnoise\n";
    let m = parse_arch_audit(t);
    assert_eq!(m["openssl"], (vec!["CVE-2022-4450".to_string(), "CVE-2022-4304".to_string()], true));
    assert_eq!(m["foo"], (vec!["CVE-2023-0001".to_string()], false));
    assert_eq!(m.len(), 2);
}

#[test]
fn installed_lines() {
    assert_eq!(parse_installed("linux 6.1.1.arch1-1\nbash 5.2-1\n"), [("linux".into(), "6.1.1.arch1-1".into()), ("bash".into(), "5.2-1".into())]);
}

#[test]
fn cache_lookup_distinguishes_similar_names() {
    let d = tempfile::tempdir().unwrap();
    for f in ["linux-lts-6.1.1-1-x86_64.pkg.tar.zst", "linux-6.1.1-1-x86_64.pkg.tar.zst", "linux-6.1.1-1-x86_64.pkg.tar.zst.sig", "bash-5.2-1-x86_64.pkg.tar.zst"] {
        std::fs::write(d.path().join(f), b"").unwrap();
    }
    let hit = cache_file(d.path(), "linux", "6.1.1-1").unwrap();
    assert!(hit.ends_with("linux-6.1.1-1-x86_64.pkg.tar.zst"), "{hit}");
    assert!(cache_file(d.path(), "linux", "6.1.2-1").is_none());
    assert!(cache_file(d.path(), "lts", "6.1.1-1").is_none());
}
