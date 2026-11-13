//! Docker migration console (capability `docker_shell`): the steps of a migration (backup, stop, change the tag, start,
//! restore…) run one after the other as root with `sh -c`, so that an update that needs more than a new tag can be done
//! without leaving the app. The commands come from a person with the `docker:shell` permission (written, edited from a
//! template, or reviewed after an AI draft) in a signed command; nothing runs that was not in it.
//!
//! The first step that fails stops the run: the next ones assume it worked. Output is logged after each step, and the
//! task reports progress every minute while a step runs, so that a long dump or restore is not taken for a dead agent.
use crate::exec::{self, Cmd};
use crate::runner::{Agent, TaskLog};
use anyhow::{bail, Result};
use croustypatch_common::model::TaskStatus;
use croustypatch_common::protocol::{check_shell_steps, ShellStep};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// One step may run this long (a dump of a large database).
const STEP_TIMEOUT_SECS: u64 = 6 * 3600;
const HEARTBEAT: Duration = Duration::from_secs(60);
/// Lines of a step's output kept in the log (the head and the tail of it when it is longer).
const OUTPUT_LINES: usize = 80;

/// The last and first lines of an output, so that a long one stays readable.
pub fn clip(output: &str, max: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= max {
        return lines.iter().map(|l| l.to_string()).collect();
    }
    let (head, tail) = (max / 4, max - max / 4);
    let mut out: Vec<String> = lines[..head].iter().map(|l| l.to_string()).collect();
    out.push(format!("… {} lines left out …", lines.len() - max));
    out.extend(lines[lines.len() - tail..].iter().map(|l| l.to_string()));
    out
}

pub async fn run(agent: &Agent, id: Uuid, container: &str, steps: &[ShellStep], source: &str, log: &TaskLog) -> Result<()> {
    if let Err(e) = check_shell_steps(steps) {
        bail!("{e}");
    }
    let n = steps.len();
    log.line(format!("migration console for {container}: {n} step(s), written {}", if source == "ai" { "by the AI and reviewed by an administrator" } else { "by an administrator" }));
    agent.update(id, TaskStatus::Running, format!("{n} step(s) to run"), log, vec![], false);
    for (i, step) in steps.iter().enumerate() {
        let (num, title) = (i + 1, step.title.trim());
        log.line(format!("── step {num}/{n}: {title}"));
        for l in step.command.lines().take(40) {
            log.line(format!("$ {l}"));
        }
        if agent.runner.dry_run {
            log.line("[dry-run] not executed".into());
            continue;
        }
        let started = Instant::now();
        let cmd = Cmd::new("sh", &["-c", &format!("umask 022\n{}", step.command)]).env("HOME", "/root").timeout(STEP_TIMEOUT_SECS);
        let fut = exec::run(&cmd);
        tokio::pin!(fut);
        let mut beat = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
        let out = loop {
            tokio::select! {
                r = &mut fut => break r,
                _ = beat.tick() => agent.update(id, TaskStatus::Running, format!("step {num}/{n}: {title} — running for {} min", started.elapsed().as_secs() / 60), log, vec![], false),
            }
        };
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                log.line(format!("step {num} could not run: {e:#}"));
                agent.update(id, TaskStatus::Failed, format!("step {num}/{n} ({title}) could not run: {e:#}; the next steps were not run"), log, vec![], false);
                return Ok(());
            }
        };
        for l in clip(&out.stdout, OUTPUT_LINES).into_iter().chain(clip(&out.stderr, OUTPUT_LINES)) {
            log.line(format!("  {l}"));
        }
        if !out.ok() {
            log.line(format!("step {num} failed with exit code {}", out.code));
            agent.update(id, TaskStatus::Failed, format!("step {num}/{n} ({title}) failed with exit code {}: {}; the next steps were not run", out.code, out.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim()), log, vec![], false);
            return Ok(());
        }
        log.line(format!("step {num} done in {} s", started.elapsed().as_secs()));
        agent.update(id, TaskStatus::Running, format!("step {num}/{n} done: {title}"), log, vec![], false);
    }
    // the containers changed: report them again
    let inv = agent.send_inventory().await.ok();
    agent.update(id, TaskStatus::Succeeded, format!("{n} step(s) done"), log, vec![], inv.is_some_and(|i| i.reboot_required));
    Ok(())
}
