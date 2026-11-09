use croustypatch_agent::preflight::sandbox_problems;

const FULL_ROOT: &str = "CapEff:\t000001ffffffffff\nCapBnd:\t000001ffffffffff\n";
// What the old service unit produced on Ubuntu 26.04: CAP_SETUID and CAP_SYS_MODULE stripped below the bounding set.
const SYSTEMD_SANDBOX: &str = "CapEff:\t000001fffffeff7f\nCapBnd:\t000001ffffffffff\n";
// An unprivileged container: the bounding set itself lacks CAP_SYS_MODULE, and the agent has all that is left.
const CONTAINER: &str = "CapEff:\t000001fffffeffff\nCapBnd:\t000001fffffeffff\n";

#[test]
fn plain_root_may_patch() {
    assert!(sandbox_problems(FULL_ROOT, Some("mnt:[4026531841]"), Some("mnt:[4026531841]")).is_empty());
}

#[test]
fn a_private_mount_namespace_is_refused() {
    let p = sandbox_problems(FULL_ROOT, Some("mnt:[4026532301]"), Some("mnt:[4026531841]"));
    assert_eq!(p.len(), 1);
    assert!(p[0].contains("mount namespace"));
}

#[test]
fn capabilities_stripped_below_the_bounding_set_are_refused_and_named() {
    let p = sandbox_problems(SYSTEMD_SANDBOX, Some("mnt:[1]"), Some("mnt:[1]"));
    assert_eq!(p.len(), 1);
    assert!(p[0].contains("CAP_SETUID") && p[0].contains("CAP_SYS_MODULE"), "{p:?}");
}

#[test]
fn a_container_root_is_not_second_guessed() {
    assert!(sandbox_problems(CONTAINER, Some("mnt:[1]"), Some("mnt:[1]")).is_empty());
}

#[test]
fn unreadable_information_does_not_block() {
    assert!(sandbox_problems("", None, None).is_empty());
}
