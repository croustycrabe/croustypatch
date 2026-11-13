//! The Docker migration console through the real task runner: steps run in order as `sh -c`, the first failure stops
//! the run, and what is not valid is refused before anything runs.
mod support;

use croustypatch_common::model::TaskStatus;
use croustypatch_common::protocol::{check_shell_steps, CommandAction, ShellStep};
use uuid::Uuid;

fn step(title: &str, command: &str) -> ShellStep {
    ShellStep { title: title.into(), command: command.into() }
}

async fn run(steps: Vec<ShellStep>, dry: bool) -> (Vec<croustypatch_common::protocol::TaskUpdate>, tempfile::TempDir) {
    let mut rig = support::rig_with(serde_json::json!({ "installed": [] }), dry);
    let task = Uuid::new_v4();
    rig.send(task, CommandAction::DockerShell { container: "docker:db".into(), steps, source: "manual".into() });
    let updates = rig.until_terminal(task, 60).await;
    (updates, rig.dir)
}

#[tokio::test]
async fn the_steps_run_in_order_and_each_one_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("order.txt");
    let (u, _d) = run(vec![step("first", &format!("echo one > {}", f.display())), step("second", &format!("echo two >> {} && echo \"from sh: $0\"", f.display())), step("umask", "umask")], false).await;
    let last = u.last().unwrap();
    assert_eq!(last.status, TaskStatus::Succeeded, "{}", last.message);
    assert_eq!(last.message, "3 step(s) done");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "one\ntwo\n");
    let log = last.log.join("\n");
    assert!(log.contains("── step 1/3: first") && log.contains("$ echo one") && log.contains("step 3 done"), "{log}");
    assert!(log.contains("0022"), "commands run with umask 022: {log}");
    assert!(u.iter().any(|x| x.status == TaskStatus::Running && x.message.contains("step 2/3 done")), "progress is reported after each step");
}

#[tokio::test]
async fn the_first_failure_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("never");
    let (u, _d) = run(vec![step("backup", "echo ok"), step("break", "echo 'pg_dump: no such database' >&2; exit 3"), step("destroy", &format!("touch {}", marker.display()))], false).await;
    let last = u.last().unwrap();
    assert_eq!(last.status, TaskStatus::Failed);
    assert!(last.message.contains("step 2/3 (break) failed with exit code 3") && last.message.contains("no such database") && last.message.contains("not run"), "{}", last.message);
    assert!(!marker.exists(), "the step after the failed one never ran");
}

#[tokio::test]
async fn a_dry_run_executes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("touched");
    let (u, _d) = run(vec![step("touch", &format!("touch {}", marker.display()))], true).await;
    assert_eq!(u.last().unwrap().status, TaskStatus::Succeeded);
    assert!(!marker.exists());
}

#[test]
fn the_steps_are_checked_before_anything_runs() {
    assert!(check_shell_steps(&[step("a", "true")]).is_ok());
    assert!(check_shell_steps(&[]).is_err());
    assert!(check_shell_steps(&vec![step("a", "true"); 21]).is_err(), "at most 20 steps");
    assert!(check_shell_steps(&[step("", "true")]).is_err());
    assert!(check_shell_steps(&[step("a", "   ")]).is_err());
    assert!(check_shell_steps(&[step("a", &"x".repeat(8001))]).is_err());
    assert!(check_shell_steps(&[step("a", "true\0")]).is_err());
    assert!(check_shell_steps(&[step("line\nbreak", "true")]).is_err());
}

#[test]
fn a_long_output_keeps_its_head_and_its_tail() {
    use croustypatch_agent::docker_shell::clip;
    let all: String = (1..=300).map(|i| format!("line {i}\n")).collect();
    let c = clip(&all, 80);
    assert_eq!(c.len(), 81);
    assert_eq!((c[0].as_str(), c[20].as_str(), c[80].as_str()), ("line 1", "… 220 lines left out …", "line 300"));
    assert_eq!(clip("a\nb", 80), vec!["a", "b"]);
}
