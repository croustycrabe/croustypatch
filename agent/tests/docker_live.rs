//! Applying a new Docker version against a real Docker daemon with the compose plugin, through the real task runner:
//! the compose file is changed, the service recreated, and everything put back when it fails.
//!
//! Needs docker, `docker compose` and internet (it pulls two small nginx images). Run on a machine you can start throw-away
//! containers on:
//!   cargo test -p croustypatch-agent --test docker_live -- --ignored --nocapture --test-threads=1
mod support;

use croustypatch_common::model::TaskStatus;
use croustypatch_common::protocol::{CommandAction, ExecPolicy};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use uuid::Uuid;

const OLD: &str = "nginx:1.27-alpine";
const NEW: &str = "nginx:1.28-alpine";

fn docker(args: &[&str]) -> (bool, String) {
    let o = Command::new("docker").args(args).output().expect("docker is installed");
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string())
}

/// A throw-away compose project with one service, removed when dropped.
struct Project {
    dir: tempfile::TempDir,
    name: String,
    container: String,
}

impl Project {
    fn new(name: &str, compose: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let p = Project { dir, name: format!("cptest-{name}"), container: format!("cptest-{name}") };
        std::fs::write(p.file(), compose).unwrap();
        let (ok, out) = docker(&["compose", "-p", &p.name, "-f", &p.file().to_string_lossy(), "up", "-d", "--quiet-pull"]);
        assert!(ok, "compose up: {out}");
        p
    }
    fn file(&self) -> PathBuf {
        self.dir.path().join("compose.yaml")
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.file()).unwrap()
    }
    /// The image reference and state the container runs now.
    fn running(&self) -> String {
        docker(&["inspect", "--format", "{{.Config.Image}} {{.State.Status}}", &self.container]).1
    }
    fn health(&self) -> String {
        docker(&["inspect", "--format", "{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}", &self.container]).1
    }
    /// A recreated container is `starting` for a moment before its first health check passes.
    fn healthy_soon(&self) -> bool {
        for _ in 0..30 {
            if self.health() == "healthy" {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        false
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.container]);
        let _ = docker(&["compose", "-p", &self.name, "-f", &self.file().to_string_lossy(), "down", "-v", "--remove-orphans"]);
    }
}

fn compose(image: &str, healthy_when: &str) -> String {
    format!(
        "# a stack like the others\nservices:\n  web:\n    container_name: {{NAME}}\n    image: {image} # pinned\n    healthcheck:\n      test: [\"CMD-SHELL\", \"{healthy_when}\"]\n      interval: 2s\n      timeout: 2s\n      retries: 2\n"
    )
}

fn policy() -> ExecPolicy {
    ExecPolicy { snapshot_required: true, soak_minutes: 0, probes: vec![], auto_rollback: true }
}

fn apply(rig: &support::Rig, container: &str, retag_to: &str) -> Uuid {
    let task = Uuid::new_v4();
    let mut retag = BTreeMap::new();
    retag.insert(format!("docker:{container}"), retag_to.to_string());
    rig.send(task, CommandAction::ApplyUpdates { packages: vec![format!("docker:{container}")], exec: policy(), retag });
    task
}

fn start(name: &str, image: &str, healthy_when: &str) -> Project {
    let c = compose(image, healthy_when).replace("{NAME}", &format!("cptest-{name}"));
    Project::new(name, &c)
}

#[tokio::test]
#[ignore = "needs docker, compose and internet"]
async fn the_version_changes_in_the_file_and_the_container_and_a_rollback_puts_both_back() {
    let p = start("ok", OLD, "nginx -v >/dev/null 2>&1");
    let original = p.text();
    assert!(p.running().starts_with(&format!("{OLD} running")), "{}", p.running());
    let mut rig = support::rig_with(serde_json::json!({ "installed": [] }), false);

    let task = apply(&rig, "cptest-ok", NEW);
    let updates = rig.until_terminal(task, 300).await;
    let last = updates.last().unwrap();
    println!("apply: {:?} {}", last.status, last.message);
    for u in &updates {
        println!("   {:?}: {}", u.status, u.message);
    }
    assert_eq!(last.status, TaskStatus::Succeeded, "{}", last.message);
    // the file says the new tag, and only that line changed (comment and layout kept)
    let edited = p.text();
    assert_eq!(edited, original.replace(&format!("image: {OLD} # pinned"), &format!("image: {NEW} # pinned")));
    // the container really runs it, healthy
    assert!(p.running().starts_with(&format!("{NEW} running")), "{}", p.running());
    assert_eq!(p.health(), "healthy");

    // a manual rollback of that task: the file first, then the previous image
    let rb = Uuid::new_v4();
    rig.send(rb, CommandAction::Rollback { target_task: task });
    let updates = rig.until_terminal(rb, 300).await;
    let last = updates.last().unwrap();
    println!("rollback: {:?} {}", last.status, last.message);
    assert_eq!(last.status, TaskStatus::Succeeded, "{}", last.message);
    assert_eq!(p.text(), original, "the file is back, byte for byte");
    assert!(p.running().starts_with(&format!("{OLD} running")), "{}", p.running());
    assert!(p.healthy_soon(), "healthy again: {}", p.health());
}

#[tokio::test]
#[ignore = "needs docker, compose and internet"]
async fn a_tag_that_comes_from_a_variable_is_refused_and_nothing_changes() {
    let c = compose("nginx:${NGINX_VERSION:-1.27-alpine}", "nginx -v >/dev/null 2>&1").replace("{NAME}", "cptest-var");
    let p = Project::new("var", &c);
    let before = (p.text(), p.running());
    let mut rig = support::rig_with(serde_json::json!({ "installed": [] }), false);
    let task = apply(&rig, "cptest-var", NEW);
    let last = rig.until_terminal(task, 120).await.pop().unwrap();
    println!("{:?} {}", last.status, last.message);
    assert_eq!(last.status, TaskStatus::Failed);
    assert!(last.message.contains("variable") && last.message.contains("nothing was changed"), "{}", last.message);
    assert_eq!((p.text(), p.running()), before, "file and container untouched");
}

#[tokio::test]
#[ignore = "needs docker, compose and internet"]
async fn a_tag_that_cannot_be_pulled_leaves_the_file_and_the_container_as_they_were() {
    let p = start("pull", OLD, "nginx -v >/dev/null 2>&1");
    let before = (p.text(), p.running());
    let mut rig = support::rig_with(serde_json::json!({ "installed": [] }), false);
    let task = apply(&rig, "cptest-pull", "nginx:1.27-alpine-no-such-tag-9.9.9");
    let updates = rig.until_terminal(task, 300).await;
    let last = updates.last().unwrap();
    for u in &updates {
        println!("   {:?}: {}", u.status, u.message);
    }
    assert_eq!(last.status, TaskStatus::Failed, "{}", last.message);
    assert!(last.message.contains("nothing was changed"), "{}", last.message);
    assert_eq!(p.text(), before.0, "the compose file was written for the pull, and is back byte for byte");
    assert_eq!(p.running(), before.1, "the container was never touched");
}

#[tokio::test]
#[ignore = "needs docker, compose and internet"]
async fn a_new_version_that_turns_unhealthy_is_rolled_back_with_its_file() {
    // healthy on 1.27 only: the 1.28 image starts, then fails its health check
    let p = start("sick", OLD, "nginx -v 2>&1 | grep -q 1.27");
    let original = p.text();
    assert!(p.healthy_soon(), "the old version is healthy to begin with");
    let mut rig = support::rig_with(serde_json::json!({ "installed": [] }), false);
    let task = apply(&rig, "cptest-sick", NEW);
    let updates = rig.until_terminal(task, 400).await;
    let last = updates.last().unwrap();
    for u in &updates {
        println!("   {:?}: {}", u.status, u.message);
    }
    assert_eq!(last.status, TaskStatus::RolledBack, "{}", last.message);
    assert!(last.message.contains("not healthy"), "{}", last.message);
    assert_eq!(p.text(), original, "the compose file is back, byte for byte");
    assert!(p.running().starts_with(&format!("{OLD} running")), "back on the previous image: {}", p.running());
    assert!(p.healthy_soon(), "and healthy again: {}", p.health());
}
