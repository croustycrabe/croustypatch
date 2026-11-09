//! Tests for `scheduler`.
use croustypatch_server::scheduler::*;

#[test]
fn eco_hosts_get_long_silence_tolerance() {
    assert_eq!(silence_limit("online", 60), 180);
    assert!(silence_limit("eco", 60) >= 5400);
    assert_eq!(silence_limit("online", 120), 360);
}
