//! A compliance audit through the real task runner against the real `oscap` and SCAP Security Guide content of the
//! machine it runs on. On Ubuntu 26.04 with `ssg-debderived` (24.04 content at most) it checks that the content is
//! pointed at this release and that rules apply, instead of every one being "not applicable".
//!
//! Needs openscap-scanner and ssg-debderived (or ssg-debian) installed, and root for a meaningful result. For example in
//! a throw-away `ubuntu:26.04` container:
//!   cargo test -p croustypatch-agent --test openscap_live -- --ignored --nocapture
mod support;

use croustypatch_common::model::TaskStatus;
use croustypatch_common::protocol::CommandAction;
use uuid::Uuid;

#[tokio::test]
#[ignore = "needs oscap and SCAP content on this machine"]
async fn an_audit_with_no_profile_uses_the_recommended_one_and_rules_apply() {
    let mut rig = support::rig_with(serde_json::json!({ "installed": [] }), false);
    // CP_SERVER_SCAP=<dir>: datastreams as the server hands them out (0.9), placed where the agent keeps them
    if let Ok(src) = std::env::var("CP_SERVER_SCAP") {
        let dst = croustypatch_agent::modules::openscap::server_dir(&rig.dir.path().join("state"));
        std::fs::create_dir_all(&dst).unwrap();
        for e in std::fs::read_dir(src).unwrap().flatten() {
            std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
        }
    }
    let task = Uuid::new_v4();
    rig.send(task, CommandAction::ComplianceScan { profile: String::new(), tailoring: None, auto_install: false });
    let updates = rig.until_terminal(task, 3000).await;
    for u in &updates {
        println!("{:?}: {}", u.status, u.message);
        for l in &u.log {
            println!("    {l}");
        }
    }
    let last = updates.last().unwrap();
    assert_eq!(last.status, TaskStatus::Succeeded, "{}", last.message);
    assert!(!last.message.contains("no rule applies"), "rules must apply: {}", last.message);
    let log: Vec<&String> = updates.iter().flat_map(|u| u.log.iter()).collect();
    if croustypatch_agent::modules::openscap::host_codename() == "resolute" {
        assert!(log.iter().any(|l| l.contains("pointed at resolute")), "the 24.04 content is adapted on 26.04");
        assert!(rig.dir.path().join("state/oscap-content/ssg-ubuntu2404-ds-resolute.xml").is_file());
    }
    let info = croustypatch_agent::modules::openscap::info("ubuntu26", false).await.expect("oscap installed");
    println!("datastream: {} · recommended: {} · adapted: {}", info.datastream, info.recommended, info.adapted);
}
