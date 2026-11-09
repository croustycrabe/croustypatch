//! Tests for `pkg::apt`.
    use croustypatch_agent::pkg::apt::*;

    const SIM: &str = "\
Reading package lists...
The following packages will be upgraded:
  libssl3 openssh-server tzdata
Inst libssl3 [3.0.2-0ubuntu1.10] (3.0.2-0ubuntu1.12 Ubuntu:22.04/jammy-updates, Ubuntu:22.04/jammy-security [amd64])
Inst openssh-server [1:8.9p1-3ubuntu0.4] (1:8.9p1-3ubuntu0.6 Ubuntu:22.04/jammy-updates, Ubuntu:22.04/jammy-security [amd64]) []
Inst tzdata [2024a-0ubuntu0.22.04] (2024b-0ubuntu0.22.04.1 Ubuntu:22.04/jammy-updates [all])
Inst linux-image-6.8.0-50 (6.8.0-50.51 Ubuntu:24.04/noble-updates [amd64])
Conf libssl3 (3.0.2-0ubuntu1.12 Ubuntu:22.04/jammy-updates [amd64])
";

    #[test]
    fn simulated_upgrades() {
        let u = parse_simulated_upgrades(SIM);
        assert_eq!(u.len(), 3, "new installs and Conf lines are not upgrades");
        assert_eq!(u[0], Upgrade { name: "libssl3".into(), old: "3.0.2-0ubuntu1.10".into(), new: "3.0.2-0ubuntu1.12".into(), security: true });
        assert!(u[1].security);
        assert_eq!(u[1].new, "1:8.9p1-3ubuntu0.6");
        assert!(!u[2].security);
    }

    #[test]
    fn debian_security_origin() {
        let u = parse_simulated_upgrades("Inst curl [7.88.1-10+deb12u4] (7.88.1-10+deb12u5 Debian-Security:12/stable-security [amd64])\n");
        assert!(u[0].security);
    }

    #[test]
    fn dpkg_status_filter() {
        let t = "bash\t5.1-6\tii \nremoved-pkg\t1.0\trc \nlibc6\t2.35-0\tii \n";
        assert_eq!(parse_dpkg_installed(t), [("bash".into(), "5.1-6".into()), ("libc6".into(), "2.35-0".into())]);
    }

    #[test]
    fn madison() {
        let m = "  libssl3 | 3.0.2-0ubuntu1.12 | http://archive/ jammy-updates/main amd64 Packages\n  libssl3 | 3.0.2-0ubuntu1.10 | http://archive/ jammy-security/main amd64 Packages\n";
        assert!(madison_has(m, "3.0.2-0ubuntu1.10"));
        assert!(!madison_has(m, "3.0.2-0ubuntu1.1"));
    }

#[test]
fn capability_check_reads_cap_eff() {
    // CapEff with CAP_SETUID (7) and CAP_SETGID (6) set: a normal root
    assert!(has_cap_setuid("Name:\tx\nCapEff:\t000001ffffffffff\n"));
    assert!(has_cap_setuid("CapEff:\t00000000000000c0\n"));
    // either one missing is enough to break apt's privilege drop
    assert!(!has_cap_setuid("CapEff:\t0000000000000080\n"), "no SETGID");
    assert!(!has_cap_setuid("CapEff:\t0000000000000040\n"), "no SETUID");
    // what `docker run --cap-drop ALL` shows
    assert!(!has_cap_setuid("CapEff:\t0000000000000000\n"));
    // unreadable or odd input keeps apt's default behaviour
    assert!(has_cap_setuid(""));
    assert!(has_cap_setuid("CapEff:\tnot-hex\n"));
}

#[test]
fn deb_file_names_follow_dpkg_conventions() {
    assert!(deb_file_matches("base-files_14ubuntu6.2_amd64.deb", "base-files", "14ubuntu6.2"));
    assert!(deb_file_matches("libssl3_3.0.2-0ubuntu1.12_amd64.deb", "libssl3", "3.0.2-0ubuntu1.12"));
    // an epoch's colon is written %3a in file names
    assert!(deb_file_matches("openssh-server_1%3a8.9p1-3ubuntu0.4_amd64.deb", "openssh-server", "1:8.9p1-3ubuntu0.4"));
    // another version, another package with the same prefix, or a partial download never match
    assert!(!deb_file_matches("base-files_14ubuntu6.3_amd64.deb", "base-files", "14ubuntu6.2"));
    assert!(!deb_file_matches("base-files-extra_14ubuntu6.2_amd64.deb", "base-files", "14ubuntu6.2"));
    assert!(!deb_file_matches("base-files_14ubuntu6.2_amd64.deb.partial", "base-files", "14ubuntu6.2"));
}
