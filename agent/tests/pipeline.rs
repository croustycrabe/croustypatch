//! Integration: the agent's task runner end to end (snapshot, apply, soak, rollback, quarantine, replay).
mod support;

use croustypatch_common::model::{HealthProbe, TaskStatus};
use croustypatch_common::protocol::{AgentMessage, CommandAction, ExecPolicy};
use serde_json::json;
use support::{pkg, rig, Service};
use uuid::Uuid;

fn statuses(u: &[croustypatch_common::protocol::TaskUpdate]) -> Vec<TaskStatus> {
    let mut v: Vec<TaskStatus> = u.iter().map(|x| x.status).collect();
    v.dedup();
    v
}

fn apply(names: &[&str], exec: ExecPolicy) -> CommandAction {
    CommandAction::ApplyUpdates { packages: names.iter().map(|s| s.to_string()).collect(), exec }
}

fn exec_with_probe(port: u16, auto_rollback: bool) -> ExecPolicy {
    ExecPolicy { snapshot_required: true, soak_minutes: 1, probes: vec![HealthProbe::Tcp { host: "127.0.0.1".into(), port }], auto_rollback }
}

#[tokio::test]
async fn patch_succeeds_updates_only_the_requested_packages_and_keeps_a_restore_point() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true), pkg("tzdata", "2024a", "2024b", false)]}));
    let id = Uuid::new_v4();
    r.send(id, apply(&["libssl3"], ExecPolicy { soak_minutes: 0, ..Default::default() }));
    let updates = r.until_terminal(id, 15).await;
    let last = updates.last().unwrap();
    assert_eq!(last.status, TaskStatus::Succeeded, "{updates:?}");
    assert_eq!(last.changes.len(), 1);
    assert_eq!((last.changes[0].name.as_str(), last.changes[0].from.as_deref(), last.changes[0].to.as_deref()), ("libssl3", Some("1.0"), Some("2.0")));
    assert_eq!(r.version("libssl3"), "2.0");
    assert_eq!(r.version("tzdata"), "2024a", "an unrequested package is left alone");
    let rp = r.agent.cfg.restore_dir().join(format!("{id}.json"));
    assert!(rp.exists(), "restore point saved before the change");
}

#[tokio::test]
async fn a_failing_probe_during_the_soak_rolls_back_automatically_and_health_returns() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true)]}));
    let svc = Service::start(r.mock_path.clone(), "2"); // version 2.x breaks the service
    let id = Uuid::new_v4();
    r.send(id, apply(&["libssl3"], exec_with_probe(svc.port, true)));
    let updates = r.until_terminal(id, 40).await;
    let seq = statuses(&updates);
    assert!(seq.contains(&TaskStatus::Soaking) && seq.contains(&TaskStatus::RollingBack), "{seq:?}");
    assert_eq!(seq.last(), Some(&TaskStatus::RolledBack), "{updates:?}");
    assert_eq!(r.version("libssl3"), "1.0", "previous version restored");
    assert!(updates.last().unwrap().message.contains("health restored"));
}

#[tokio::test]
async fn when_the_rollback_also_fails_the_host_is_isolated_and_evidence_is_frozen() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true)], "fail_restore": true}));
    let svc = Service::start(r.mock_path.clone(), "2");
    let id = Uuid::new_v4();
    r.send(id, apply(&["libssl3"], exec_with_probe(svc.port, true)));
    let updates = r.until_terminal(id, 40).await;
    let last = updates.last().unwrap();
    assert_eq!(last.status, TaskStatus::Quarantined, "{updates:?}");
    assert!(last.message.contains("double failure") && last.message.contains("isolated"), "{}", last.message);
    let dir = r.agent.cfg.forensics_dir().join(id.to_string());
    let manifest = std::fs::read_to_string(dir.join("MANIFEST.sha256")).expect("evidence manifest");
    for line in manifest.lines() {
        let (hash, name) = line.split_once("  ").unwrap();
        assert_eq!(hash, croustypatch_common::crypto::sha256_hex(&std::fs::read(dir.join(name)).unwrap()), "{name}");
    }
    assert!(manifest.contains("restore.json") && manifest.contains("task.log"));
}

#[tokio::test]
async fn a_failed_apply_rolls_back_when_allowed_and_fails_cleanly_when_not() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true)], "fail_apply": true}));
    let id = Uuid::new_v4();
    r.send(id, apply(&["libssl3"], ExecPolicy { soak_minutes: 0, auto_rollback: true, ..Default::default() }));
    assert_eq!(r.until_terminal(id, 15).await.last().unwrap().status, TaskStatus::RolledBack);

    let id2 = Uuid::new_v4();
    r.send(id2, apply(&["libssl3"], ExecPolicy { soak_minutes: 0, auto_rollback: false, ..Default::default() }));
    let last = r.until_terminal(id2, 15).await.pop().unwrap();
    assert_eq!(last.status, TaskStatus::Failed);
    assert!(last.message.contains("auto-rollback is disabled"), "{}", last.message);
}

#[tokio::test]
async fn a_probe_that_was_already_failing_is_not_blamed_on_the_patch() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true)]}));
    // nothing listens on this port from the start
    let dead = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
    let id = Uuid::new_v4();
    r.send(id, apply(&["libssl3"], exec_with_probe(dead, true)));
    let updates = r.until_terminal(id, 20).await;
    assert_eq!(updates.last().unwrap().status, TaskStatus::Succeeded, "{updates:?}");
    let log = updates.last().unwrap().log.join("\n");
    assert!(log.contains("already failing") && log.contains("soak skipped"), "{log}");
    assert_eq!(r.version("libssl3"), "2.0");
}

#[tokio::test]
async fn a_replayed_task_id_never_runs_twice() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true)]}));
    let id = Uuid::new_v4();
    r.send(id, apply(&["libssl3"], ExecPolicy { soak_minutes: 0, ..Default::default() }));
    r.until_terminal(id, 15).await;
    // flip the package back so a second run would be visible
    let mut db = r.db();
    db["installed"][0]["version"] = json!("1.0");
    db["installed"][0]["candidate"] = json!("2.0");
    std::fs::write(&r.mock_path, serde_json::to_vec(&db).unwrap()).unwrap();
    r.send(id, apply(&["libssl3"], ExecPolicy { soak_minutes: 0, ..Default::default() }));
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let mut late = vec![];
    while let Ok(m) = r.out_rx.try_recv() {
        if let AgentMessage::TaskUpdate(u) = m {
            late.push(u);
        }
    }
    assert!(late.is_empty(), "the duplicate command produced updates: {late:?}");
    assert_eq!(r.version("libssl3"), "1.0", "and changed nothing");
}

#[tokio::test]
async fn dangerous_or_pointless_requests_are_refused_without_touching_the_system() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true), json!({"name": "bash", "version": "5.2"})]}));
    for bad in ["-oFoo", "a;rm -rf /", "$(id)", ""] {
        let id = Uuid::new_v4();
        r.send(id, apply(&[bad], ExecPolicy::default()));
        let last = r.until_terminal(id, 10).await.pop().unwrap();
        assert_eq!(last.status, TaskStatus::Failed, "{bad:?}");
        assert!(last.message.contains("invalid package name"), "{}", last.message);
    }
    assert_eq!(r.version("libssl3"), "1.0");
    let id = Uuid::new_v4();
    r.send(id, apply(&["bash"], ExecPolicy::default()));
    let last = r.until_terminal(id, 10).await.pop().unwrap();
    assert_eq!((last.status, last.message.contains("nothing to update")), (TaskStatus::Succeeded, true));
}

#[tokio::test]
async fn scan_reports_an_inventory_and_manual_rollback_restores_versions() {
    let mut r = rig(json!({"installed": [pkg("libssl3", "1.0", "2.0", true)]}));
    let scan = Uuid::new_v4();
    r.send(scan, CommandAction::Scan);
    // the inventory message precedes the terminal task update
    let mut inventory = None;
    loop {
        match r.out_rx.recv().await.unwrap() {
            AgentMessage::Inventory(i) => inventory = Some(i),
            AgentMessage::TaskUpdate(u) if u.task_id == scan && u.status.is_terminal() => {
                assert_eq!(u.status, TaskStatus::Succeeded);
                break;
            }
            _ => {}
        }
    }
    let inv = inventory.expect("scan sends an inventory");
    assert_eq!(inv.packages.len(), 1);
    assert_eq!(inv.packages[0].candidate.as_deref(), Some("2.0"));
    assert!(inv.packages[0].is_security);

    let patch = Uuid::new_v4();
    r.send(patch, apply(&["libssl3"], ExecPolicy { soak_minutes: 0, ..Default::default() }));
    r.until_terminal(patch, 15).await;
    assert_eq!(r.version("libssl3"), "2.0");

    let rb = Uuid::new_v4();
    r.send(rb, CommandAction::Rollback { target_task: patch });
    let last = r.until_terminal(rb, 15).await.pop().unwrap();
    assert_eq!(last.status, TaskStatus::Succeeded, "{}", last.message);
    assert_eq!(r.version("libssl3"), "1.0");

    let ghost = Uuid::new_v4();
    r.send(ghost, CommandAction::Rollback { target_task: Uuid::new_v4() });
    let last = r.until_terminal(ghost, 15).await.pop().unwrap();
    assert_eq!(last.status, TaskStatus::Failed, "rolling back something that never existed fails cleanly");
}

#[tokio::test]
async fn quarantine_and_release_commands_succeed_in_dry_run() {
    let mut r = rig(json!({"installed": []}));
    for action in [CommandAction::Quarantine, CommandAction::Release] {
        let id = Uuid::new_v4();
        r.send(id, action);
        assert_eq!(r.until_terminal(id, 10).await.last().unwrap().status, TaskStatus::Succeeded);
    }
    // without a known management server address, isolating the host would cut it off for good: refused
    *r.agent.server_ips.lock().unwrap() = vec![];
    let id = Uuid::new_v4();
    r.send(id, CommandAction::Quarantine);
    assert_eq!(r.until_terminal(id, 10).await.last().unwrap().status, TaskStatus::Failed);
}

mod reboot_logic {
    use chrono::{Duration, Utc};
    use croustypatch_agent::reboot::{resume_decision, shutdown_args, PendingReboot, Resume};
    use uuid::Uuid;

    #[test]
    fn shutdown_gets_a_delay_and_a_clean_message() {
        assert_eq!(shutdown_args(0, "x")[..2], ["-r".to_string(), "now".to_string()]);
        let a = shutdown_args(10, "bye\n\x07now");
        assert_eq!(a[1], "+10");
        assert!(!a[2].chars().any(|c| c.is_control()), "no control characters reach the broadcast");
        assert!(shutdown_args(5, "  ")[2].contains("Save your work"), "a default warning");
    }

    #[test]
    fn a_new_boot_closes_the_task_and_the_same_boot_waits_then_fails() {
        let t = Uuid::new_v4();
        let now = Utc::now();
        let p = PendingReboot { task_id: t, boot_id: "boot-a".into(), requested_at: now - Duration::seconds(50), delay_minutes: 0 };
        assert_eq!(resume_decision(Some(&p), "boot-b", now), Resume::BackAfterReboot { task_id: t, seconds: 50 });
        assert_eq!(resume_decision(Some(&p), "boot-a", now), Resume::StillWaiting);
        assert_eq!(resume_decision(Some(&p), "boot-a", now + Duration::minutes(31)), Resume::DidNotReboot { task_id: t });
        assert_eq!(resume_decision(None, "boot-a", now), Resume::Nothing);
    }
}
