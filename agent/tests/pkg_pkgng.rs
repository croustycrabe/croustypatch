//! Tests for `pkg::pkgng`.
use croustypatch_agent::pkg::pkgng::*;

#[test]
fn upgrade_plan() {
    let t = "Updating FreeBSD repository catalogue...\nChecking for upgrades (2 candidates): 100%\nThe following 2 package(s) will be affected (of 0 checked):\n\nInstalled packages to be UPGRADED:\n\tcurl: 8.4.0 -> 8.5.0\n\tca_root_nss: 3.95 -> 3.96_1\n\nNumber of packages to be upgraded: 2\n";
    assert_eq!(parse_upgrade_plan(t), [("curl".into(), "8.4.0".into(), "8.5.0".into()), ("ca_root_nss".into(), "3.95".into(), "3.96_1".into())]);
}

#[test]
fn audit_blocks() {
    let t = "curl-8.4.0 is vulnerable:\n  curl -- multiple vulnerabilities\n  CVE: CVE-2023-46218\n  CVE: CVE-2023-46219\n  WWW: https://vuxml.FreeBSD.org/freebsd/abc.html\n\npy311-requests-2.30.0_1 is vulnerable:\n  requests -- leak\n  WWW: https://vuxml.FreeBSD.org/freebsd/def.html\n\n2 problem(s) in 2 installed package(s) found.\n";
    let m = parse_audit(t);
    assert_eq!(m["curl"], ["CVE-2023-46218", "CVE-2023-46219"]);
    assert!(m["py311-requests"].is_empty(), "name containing digits and dashes is split at the last dash");
}

#[test]
fn installed_query() {
    assert_eq!(parse_installed("curl\t8.4.0\nbash\t5.2.21\n"), [("curl".into(), "8.4.0".into()), ("bash".into(), "5.2.21".into())]);
}
