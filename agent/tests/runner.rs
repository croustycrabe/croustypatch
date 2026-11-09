//! Tests for `runner`.
use croustypatch_agent::config::Config;
use uuid::Uuid;
use croustypatch_agent::runner::*;

#[test]
fn probe_tracker_needs_consecutive_failures() {
    let mut t = ProbeTracker::new(&[Ok(()), Ok(())]);
    let bad = || vec![Ok(()), Err("down".to_string())];
    let good = || vec![Ok(()), Ok(())];
    assert_eq!(t.record(&bad()), None, "one failure is a blip");
    assert_eq!(t.record(&good()), None, "recovery resets the counter");
    assert_eq!(t.record(&bad()), None);
    assert_eq!(t.record(&bad()), Some(1), "two in a row trigger");
}

#[test]
fn preexisting_failures_are_ignored() {
    let mut t = ProbeTracker::new(&[Err("already down".into()), Ok(())]);
    for _ in 0..5 {
        assert_eq!(t.record(&[Err("still down".into()), Ok(())]), None);
    }
    assert_eq!(t.record(&[Err("x".into()), Err("y".into())]), None);
    assert_eq!(t.record(&[Err("x".into()), Err("y".into())]), Some(1));
}

#[test]
fn a_task_is_claimed_only_once() {
    let d = tempfile::tempdir().unwrap();
    let cfg = Config { state_dir: d.path().into(), ..Default::default() };
    let id = Uuid::new_v4();
    assert!(claim_task(&cfg, id));
    assert!(!claim_task(&cfg, id), "replayed command must not run again");
    assert!(claim_task(&cfg, Uuid::new_v4()));
}
