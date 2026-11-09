//! Reboot module (agent side).
//!
//! The agent cannot report "done" from a machine that is going down, so a reboot is a two-part task: before the
//! restart it saves which task asked for it and the current boot id, tells the server (which opens a downtime window
//! so supervision does not page anyone), and hands the restart to the system's `shutdown`, whose broadcast warns
//! logged-in users `delay_minutes` ahead. When the agent starts again it compares boot ids and closes the task.
use crate::config::{write_private, Config};
use crate::exec::Cmd;
use crate::runner::{Agent, TaskLog};
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use croustypatch_common::model::TaskStatus;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingReboot {
    pub task_id: Uuid,
    pub boot_id: String,
    pub requested_at: DateTime<Utc>,
    pub delay_minutes: u32,
}

fn pending_file(cfg: &Config) -> std::path::PathBuf {
    cfg.state_dir.join("pending_reboot.json")
}

/// Identifier of the current boot: changes on every restart.
pub fn boot_id() -> String {
    if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
        return s.trim().to_string();
    }
    // FreeBSD and others: the boot time is just as good.
    std::process::Command::new("/sbin/sysctl").args(["-n", "kern.boottime"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_default()
}

/// Arguments for shutdown(8): Linux and FreeBSD both take `-r +N message` (`now` for no delay).
pub fn shutdown_args(delay_minutes: u32, message: &str) -> Vec<String> {
    let when = if delay_minutes == 0 { "now".to_string() } else { format!("+{delay_minutes}") };
    let msg = if message.trim().is_empty() { "CroustyPatch: this machine restarts to finish installing updates. Save your work.".to_string() } else { message.chars().filter(|c| !c.is_control()).take(200).collect() };
    vec!["-r".into(), when, msg]
}

/// What a freshly started agent should report about a reboot it asked for before.
#[derive(Debug, PartialEq)]
pub enum Resume {
    Nothing,
    BackAfterReboot { task_id: Uuid, seconds: i64 },
    DidNotReboot { task_id: Uuid },
    StillWaiting,
}

pub fn resume_decision(p: Option<&PendingReboot>, current_boot: &str, now: DateTime<Utc>) -> Resume {
    let Some(p) = p else { return Resume::Nothing };
    if !current_boot.is_empty() && current_boot != p.boot_id {
        return Resume::BackAfterReboot { task_id: p.task_id, seconds: (now - p.requested_at).num_seconds().max(0) };
    }
    // Same boot: give shutdown its delay plus a margin before calling it a failure (the agent may just have been
    // restarted by an upgrade in the meantime).
    if now - p.requested_at > chrono::Duration::minutes(p.delay_minutes as i64 + 30) {
        Resume::DidNotReboot { task_id: p.task_id }
    } else {
        Resume::StillWaiting
    }
}

pub async fn start(agent: &Agent, id: Uuid, delay_minutes: u32, message: &str, log: &TaskLog) -> Result<()> {
    let delay = delay_minutes.min(24 * 60);
    let args = shutdown_args(delay, message);
    let p = PendingReboot { task_id: id, boot_id: boot_id(), requested_at: Utc::now(), delay_minutes: delay };
    if agent.runner.dry_run {
        log.line(format!("dry-run: would run shutdown {}", args.join(" ")));
        agent.update(id, TaskStatus::Succeeded, "dry-run: reboot skipped", log, vec![], false);
        return Ok(());
    }
    write_private(&pending_file(&agent.cfg), &serde_json::to_vec(&p)?)?;
    let when = if delay == 0 { "now".to_string() } else { format!("in {delay} min") };
    agent.update(id, TaskStatus::Running, format!("rebooting {when}"), log, vec![], false);
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = agent.runner.mutate(Cmd::new("shutdown", &refs), log).await?;
    if !out.ok() {
        let _ = std::fs::remove_file(pending_file(&agent.cfg));
        bail!("shutdown failed ({}): {}", out.code, out.stderr.trim());
    }
    Ok(())
}

/// Called once when the agent starts: close the reboot task this host was waiting on.
pub fn resume(agent: &Agent) {
    let path = pending_file(&agent.cfg);
    let p: Option<PendingReboot> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let log = TaskLog::default();
    match resume_decision(p.as_ref(), &boot_id(), Utc::now()) {
        Resume::BackAfterReboot { task_id, seconds } => {
            log.line(format!("new boot detected ({seconds} s after the request)"));
            agent.update(task_id, TaskStatus::Succeeded, format!("host back after reboot ({seconds} s)"), &log, vec![], false);
            let _ = std::fs::remove_file(&path);
        }
        Resume::DidNotReboot { task_id } => {
            agent.update(task_id, TaskStatus::Failed, "the host did not restart (same boot as before the request)", &log, vec![], false);
            let _ = std::fs::remove_file(&path);
        }
        Resume::Nothing | Resume::StillWaiting => {}
    }
}
