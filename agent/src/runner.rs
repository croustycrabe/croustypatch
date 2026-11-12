//! Executes verified commands: scan, patch (snapshot → apply → soak → auto-rollback), manual rollback,
//! quarantine/release. Tasks run strictly one at a time.
use crate::config::{write_private, Config};
use crate::exec::Runner;
use crate::health;
use crate::identity::Identity;
use crate::isolate;
use crate::pkg::{self, Backend, RestorePoint};
use anyhow::{anyhow, Result};
use chrono::Utc;
use croustypatch_common::model::{is_valid_package_name, HealthProbe, TaskStatus};
use croustypatch_common::protocol::*;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Consecutive failures of one probe that count as "the service is down".
const PROBE_FAIL_THRESHOLD: u32 = 2;

#[derive(Clone, Default)]
pub struct TaskLog(Arc<Mutex<Vec<String>>>);

impl TaskLog {
    pub fn line(&self, s: String) {
        tracing::debug!("{s}");
        let mut v = self.0.lock().unwrap();
        v.push(s);
        if v.len() > 2000 {
            v.drain(..500);
        }
    }
    pub fn snapshot(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    pub fn tail(&self, n: usize) -> Vec<String> {
        let v = self.0.lock().unwrap();
        v[v.len().saturating_sub(n)..].to_vec()
    }
}

pub struct Agent {
    pub cfg: Config,
    pub identity: Identity,
    pub backend: Backend,
    pub runner: Runner,
    pub out: mpsc::UnboundedSender<AgentMessage>,
    pub server_ips: Mutex<Vec<IpAddr>>,
    pub running: Mutex<HashSet<Uuid>>,
    /// Per-host settings from the server (None until received from a server that sends them).
    pub settings: Mutex<Option<AgentSettings>>,
}

fn settings_file(cfg: &Config) -> std::path::PathBuf {
    cfg.state_dir.join("settings.json")
}

/// The settings saved by the last run, so module switches apply before the server is reachable.
pub fn load_settings(cfg: &Config) -> Option<AgentSettings> {
    std::fs::read(settings_file(cfg)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

impl Agent {
    pub fn settings(&self) -> Option<AgentSettings> {
        self.settings.lock().unwrap().clone()
    }

    /// Air-gapped installation: nothing is looked up on the internet.
    pub fn offline(&self) -> bool {
        self.settings().is_some_and(|s| s.offline)
    }

    /// Store new settings from the server; returns true if they changed.
    pub fn apply_settings(&self, s: AgentSettings) -> bool {
        let mut cur = self.settings.lock().unwrap();
        if cur.as_ref() == Some(&s) {
            return false;
        }
        tracing::info!(modules = ?s.modules, "settings received from the server");
        if let Ok(b) = serde_json::to_vec(&s) {
            let _ = write_private(&settings_file(&self.cfg), &b);
        }
        *cur = Some(s);
        true
    }

    /// Is this module switched on for this host? Before the server has said anything, everything detected is
    /// collected (the server drops what it does not want).
    pub fn module_on(&self, name: &str) -> bool {
        self.settings().map_or(true, |s| s.modules.get(name))
    }

    /// What isolation keeps reachable: the server's setting when known, else the default (SSH kept open).
    pub fn isolation(&self) -> croustypatch_common::protocol::IsolationSettings {
        self.settings().map(|s| s.isolation).unwrap_or_default()
    }

    /// Economy profile allowed: the server's switch when known, else the local configuration.
    pub fn eco_allowed(&self) -> bool {
        self.settings().map_or(self.cfg.eco_enabled, |s| s.modules.eco)
    }

    /// Addresses of the management server, copied out so no lock is held across an await.
    pub fn server_ips(&self) -> Vec<IpAddr> {
        self.server_ips.lock().unwrap().clone()
    }

    pub fn update(&self, task: Uuid, status: TaskStatus, message: impl Into<String>, log: &TaskLog, changes: Vec<PackageChange>, reboot_required: bool) {
        let message = message.into();
        tracing::info!(%task, status = %status, "{message}");
        let _ = self.out.send(AgentMessage::TaskUpdate(TaskUpdate { task_id: task, status, message, log: log.tail(200), changes, reboot_required }));
    }

    pub async fn collect_inventory(&self) -> Result<Inventory> {
        let facts = crate::facts::collect(&self.cfg.server_url, self.backend.kind(), crate::modules::detected()).await;
        let mut packages = self.backend.inventory(&self.runner).await?;
        // The mock backend simulates a whole system: never mix the real host's modules into it.
        if !matches!(self.backend, Backend::Mock(_)) {
            if self.module_on("docker") && crate::modules::docker::available() {
                packages.extend(crate::modules::docker::inventory(&self.runner, !self.offline()).await);
            }
            if self.module_on("helper_script") {
                packages.extend(crate::modules::helper_scripts::detect(std::path::Path::new("/opt")));
            }
        }
        let reboot_required = self.backend.reboot_required(&self.runner).await;
        Ok(Inventory { facts, packages, reboot_required, collected_at: Utc::now() })
    }

    pub async fn send_inventory(&self) -> Result<Inventory> {
        let inv = self.collect_inventory().await?;
        let _ = self.out.send(AgentMessage::Inventory(inv.clone()));
        Ok(inv)
    }
}

/// Tracks consecutive probe failures; a probe that was already broken before the patch is ignored.
pub struct ProbeTracker {
    ignored: HashSet<usize>,
    fails: HashMap<usize, u32>,
}

impl ProbeTracker {
    pub fn new(baseline: &[Result<(), String>]) -> Self {
        Self { ignored: baseline.iter().enumerate().filter(|(_, r)| r.is_err()).map(|(i, _)| i).collect(), fails: HashMap::new() }
    }
    /// Feed one round of results; returns the index of a probe that has failed too many times in a row.
    pub fn record(&mut self, results: &[Result<(), String>]) -> Option<usize> {
        for (i, r) in results.iter().enumerate() {
            if self.ignored.contains(&i) {
                continue;
            }
            let n = self.fails.entry(i).or_default();
            match r {
                Ok(()) => *n = 0,
                Err(_) => {
                    *n += 1;
                    if *n >= PROBE_FAIL_THRESHOLD {
                        return Some(i);
                    }
                }
            }
        }
        None
    }
}

pub async fn worker(agent: Arc<Agent>, mut rx: mpsc::UnboundedReceiver<CommandPayload>) {
    while let Some(p) = rx.recv().await {
        let id = p.task_id;
        agent.running.lock().unwrap().insert(id);
        execute(&agent, p).await;
        agent.running.lock().unwrap().remove(&id);
    }
}

fn done_marker(cfg: &Config, id: Uuid) -> std::path::PathBuf {
    cfg.done_dir().join(id.to_string())
}

/// True if this task was already started (replay or duplicate delivery); marks it otherwise.
pub fn claim_task(cfg: &Config, id: Uuid) -> bool {
    let m = done_marker(cfg, id);
    if m.exists() {
        return false;
    }
    let _ = std::fs::create_dir_all(cfg.done_dir());
    std::fs::write(m, Utc::now().to_rfc3339()).is_ok()
}

async fn execute(agent: &Agent, p: CommandPayload) {
    let log = TaskLog::default();
    if !claim_task(&agent.cfg, p.task_id) {
        tracing::warn!(task = %p.task_id, "task already handled; ignoring duplicate command");
        return;
    }
    let id = p.task_id;
    let res = match p.action {
        CommandAction::Scan => scan(agent, id, &log).await,
        CommandAction::ApplyUpdates { packages, exec, retag } => apply_updates(agent, id, packages, retag, exec, &log).await,
        CommandAction::Rollback { target_task } => manual_rollback(agent, id, target_task, &log).await,
        CommandAction::Quarantine => isolate::quarantine(&agent.runner, &agent.server_ips(), &agent.isolation(), &log).await.map(|_| agent.update(id, TaskStatus::Succeeded, "host isolated from the network", &log, vec![], false)),
        CommandAction::Release => isolate::release(&agent.runner, &log).await.map(|_| agent.update(id, TaskStatus::Succeeded, "network isolation lifted", &log, vec![], false)),
        CommandAction::Reboot { delay_minutes, message } => crate::reboot::start(agent, id, delay_minutes, &message, &log).await,
        CommandAction::ComplianceScan { profile, tailoring, auto_install } => compliance(agent, id, &profile, tailoring.as_ref(), auto_install, &log).await,
        CommandAction::App { install, command: Some(cmd), user, .. } => app_command(agent, id, install, &cmd, user.as_deref(), &log).await,
        CommandAction::App { install, packages, command: None, .. } => app(agent, id, install, &packages, &log).await,
        CommandAction::UpdateAgent { platform, sha256, version, gui_sha256 } => crate::selfupdate::start(agent, id, &platform, &sha256, &version, gui_sha256.as_deref(), &log).await,
    };
    if let Err(e) = res {
        log.line(format!("error: {e:#}"));
        agent.update(id, TaskStatus::Failed, format!("{e:#}"), &log, vec![], false);
    }
}

/// App center: install or remove a catalog application's packages, then report the new inventory.
async fn app(agent: &Agent, id: Uuid, install: bool, packages: &[String], log: &TaskLog) -> Result<()> {
    if packages.is_empty() || packages.len() > 50 {
        return Err(anyhow!("an application needs 1 to 50 packages"));
    }
    if let Some(bad) = packages.iter().find(|p| !is_valid_package_name(p) || p.contains(':')) {
        return Err(anyhow!("refusing invalid package name {bad:?}"));
    }
    let verb = if install { "installing" } else { "removing" };
    agent.update(id, TaskStatus::Running, format!("{verb} {}", packages.join(", ")), log, vec![], false);
    let before = pkg::installed_map(&agent.backend.inventory(&agent.runner).await?);
    if install {
        agent.backend.refresh(&agent.runner, log).await;
        agent.backend.install(&agent.runner, packages, log).await?;
    } else {
        agent.backend.remove(&agent.runner, packages, log).await?;
    }
    let after = agent.send_inventory().await?;
    let changes = pkg::diff(&before, &pkg::installed_map(&after.packages));
    agent.update(id, TaskStatus::Succeeded, format!("{} package(s) {}", changes.len(), if install { "installed" } else { "removed" }), log, changes, after.reboot_required);
    Ok(())
}

/// App center, command recipe: run the admin's command, then check the app is usable by whoever asked.
async fn app_command(agent: &Agent, id: Uuid, install: bool, cmd: &AppCommand, user: Option<&str>, log: &TaskLog) -> Result<()> {
    if !croustypatch_common::model::is_valid_check_path(&cmd.check) {
        return Err(anyhow!("refusing a recipe whose check is not an absolute path"));
    }
    let target = match user {
        Some(u) => crate::app_command::lookup(u).await?,
        None => crate::app_command::Target::everyone(),
    };
    agent.update(id, TaskStatus::Running, format!("{} with the catalog command (for {})", if install { "installing" } else { "removing" }, target.name), log, vec![], false);
    let summary = crate::app_command::run(&agent.runner, cmd, install, &target, log).await?;
    let inv = agent.send_inventory().await?;
    agent.update(id, TaskStatus::Succeeded, summary, log, vec![], inv.reboot_required);
    Ok(())
}

async fn compliance(agent: &Agent, id: Uuid, profile: &str, tailoring: Option<&Tailoring>, auto_install: bool, log: &TaskLog) -> Result<()> {
    use crate::modules::openscap;
    let facts = crate::facts::collect(&agent.cfg.server_url, agent.backend.kind(), vec![]).await;
    let kind = agent.backend.kind();
    let hint = format!("{}{}", facts.distro.to_ascii_lowercase().split_whitespace().next().unwrap_or(""), facts.os_version.split('.').next().unwrap_or(""));
    // OpenSCAP or its content missing: install them first (visible in this task's log), unless the administrator turned that off.
    let have_content = openscap::find_datastream(std::path::Path::new(openscap::CONTENT_DIR), &hint).is_some();
    if let openscap::InstallPlan::Install { packages, note } = openscap::install_plan(openscap::available(), have_content, auto_install, kind, &facts.distro) {
        agent.update(id, TaskStatus::Running, format!("installing OpenSCAP ({})", packages.join(", ")), log, vec![], false);
        agent.backend.refresh(&agent.runner, log).await;
        let names: Vec<String> = packages.iter().map(|s| s.to_string()).collect();
        agent.backend.install(&agent.runner, &names, log).await?;
        if let Some(n) = note {
            log.line(format!("note: {n}"));
        }
        let _ = agent.send_inventory().await;
    }
    agent.update(id, TaskStatus::Running, "compliance scan running (low priority)", log, vec![], false);
    let report = openscap::scan(&agent.runner, &agent.cfg.state_dir, profile, tailoring, &hint, kind, &facts.distro, log).await?;
    let msg = if report.passed + report.failed == 0 {
        format!("compliance: no rule applies to this system ({} not applicable): the SCAP content does not cover this release", report.not_applicable)
    } else {
        format!("compliance {:.1}% ({} passed, {} failed)", report.score, report.passed, report.failed)
    };
    let _ = agent.out.send(AgentMessage::Compliance { task_id: id, report });
    agent.update(id, TaskStatus::Succeeded, msg, log, vec![], false);
    Ok(())
}

async fn scan(agent: &Agent, id: Uuid, log: &TaskLog) -> Result<()> {
    agent.update(id, TaskStatus::Running, "scanning", log, vec![], false);
    agent.backend.refresh(&agent.runner, log).await;
    let inv = agent.send_inventory().await?;
    let pending = inv.packages.iter().filter(|p| p.candidate.is_some()).count();
    let sec = inv.packages.iter().filter(|p| p.candidate.is_some() && p.is_security).count();
    agent.update(id, TaskStatus::Succeeded, format!("{} packages, {pending} updates ({sec} security)", inv.packages.len()), log, vec![], inv.reboot_required);
    Ok(())
}

fn save_restore_point(cfg: &Config, rp: &RestorePoint) -> Result<()> {
    std::fs::create_dir_all(cfg.restore_dir())?;
    write_private(&cfg.restore_dir().join(format!("{}.json", rp.task_id)), &serde_json::to_vec_pretty(rp)?)?;
    Ok(())
}

fn load_restore_point(cfg: &Config, id: Uuid) -> Result<RestorePoint> {
    let p = cfg.restore_dir().join(format!("{id}.json"));
    serde_json::from_slice(&std::fs::read(&p).map_err(|e| anyhow!("no restore point for task {id}: {e}"))?).map_err(Into::into)
}

async fn run_probes(agent: &Agent, probes: &[HealthProbe]) -> Vec<Result<(), String>> {
    let mut out = Vec::with_capacity(probes.len());
    for p in probes {
        out.push(health::check(p, &agent.runner).await);
    }
    out
}

/// Rollback failed (or could not run): isolate the host, freeze evidence, tell the server.
async fn double_failure(agent: &Agent, id: Uuid, reason: &str, rp: Option<&RestorePoint>, log: &TaskLog) {
    log.line(format!("DOUBLE FAILURE: {reason}"));
    let ips = agent.server_ips();
    let isolated = isolate::quarantine(&agent.runner, &ips, &agent.isolation(), log).await;
    let extra: Vec<(&str, Vec<u8>)> = rp.map(|r| vec![("restore.json", serde_json::to_vec_pretty(r).unwrap_or_default())]).unwrap_or_default();
    let evidence = isolate::forensics_freeze(&agent.cfg, &agent.runner, id, log, &extra).await;
    let ev = evidence.map(|p| p.display().to_string()).unwrap_or_else(|e| format!("evidence capture failed: {e}"));
    match isolated {
        Ok(()) => agent.update(id, TaskStatus::Quarantined, format!("double failure ({reason}): host isolated from the network, evidence frozen in {ev}"), log, vec![], false),
        Err(e) => agent.update(id, TaskStatus::Failed, format!("double failure ({reason}) AND isolation failed ({e}); evidence: {ev}. Manual intervention required."), log, vec![], false),
    }
}

/// Restore, verify, and report. Used by auto-rollback and manual rollback.
async fn restore_and_verify(agent: &Agent, id: Uuid, rp: &RestorePoint, probes: &[HealthProbe], ignored: &[usize], reason: &str, log: &TaskLog) {
    agent.update(id, TaskStatus::RollingBack, format!("rolling back: {reason}"), log, vec![], false);
    if let Err(e) = agent.backend.restore(&agent.runner, rp, log).await {
        return double_failure(agent, id, &format!("rollback failed: {e:#}"), Some(rp), log).await;
    }
    // The service must be back; give it a moment to restart.
    for attempt in 0..6 {
        let results = run_probes(agent, probes).await;
        let bad: Vec<String> = results.iter().enumerate().filter(|(i, r)| r.is_err() && !ignored.contains(i)).map(|(i, _)| health::describe(&probes[i])).collect();
        if bad.is_empty() {
            let _ = agent.send_inventory().await;
            return agent.update(id, TaskStatus::RolledBack, format!("rolled back to the previous versions ({reason}); health restored"), log, vec![], false);
        }
        log.line(format!("after rollback, still failing: {} (attempt {})", bad.join(", "), attempt + 1));
        tokio::time::sleep(Duration::from_secs(agent.cfg.probe_interval_secs.min(10))).await;
    }
    double_failure(agent, id, "services still unhealthy after rollback", Some(rp), log).await
}

async fn apply_updates(agent: &Agent, id: Uuid, requested: Vec<String>, retag: std::collections::BTreeMap<String, String>, exec: ExecPolicy, log: &TaskLog) -> Result<()> {
    if let Some(bad) = requested.iter().find(|p| !is_valid_package_name(p)) {
        return Err(anyhow!("refusing invalid package name {bad:?}"));
    }
    // Containers (docker:<name>) follow their own cycle first; system packages then go through the usual one.
    let (containers, requested): (Vec<String>, Vec<String>) = requested.into_iter().partition(|n| n.starts_with(crate::modules::docker::PREFIX));
    let mut container_changes = vec![];
    if !containers.is_empty() {
        match update_containers(agent, id, &containers, &retag, &exec, log).await? {
            Some(ch) => container_changes = ch,
            None => return Ok(()), // rolled back or failed: the task is already closed
        }
        if requested.is_empty() {
            let n = container_changes.len();
            agent.update(id, TaskStatus::Succeeded, format!("{n} container(s) updated"), log, container_changes, false);
            return Ok(());
        }
        log.line(format!("{} container(s) updated; now the system packages", container_changes.len()));
    }
    agent.update(id, TaskStatus::Running, "preparing", log, vec![], false);
    agent.backend.refresh(&agent.runner, log).await;
    let inv = agent.backend.inventory(&agent.runner).await?;
    let before = pkg::installed_map(&inv);
    let candidates: HashSet<&str> = inv.iter().filter(|p| p.candidate.is_some()).map(|p| p.name.as_str()).collect();
    let names: Vec<String> = if agent.backend.full_upgrade_only() { candidates.iter().map(|s| s.to_string()).collect() } else { requested.iter().filter(|n| candidates.contains(n.as_str())).cloned().collect() };
    if agent.backend.full_upgrade_only() {
        log.line("this package manager only supports full upgrades: every pending update will be applied".into());
    }
    if names.is_empty() {
        agent.update(id, TaskStatus::Succeeded, "nothing to update (already current)", log, vec![], false);
        return Ok(());
    }

    // Refuse before anything changes if package scripts could not run properly from here (see preflight.rs).
    if !agent.runner.dry_run && !matches!(agent.backend, Backend::Mock(_)) {
        let problems = crate::preflight::current_problems();
        if !problems.is_empty() {
            for p in &problems {
                log.line(format!("pre-flight: {p}"));
            }
            return Err(anyhow!("refusing to patch: {}. Nothing was changed. Re-run the agent installer to replace the service unit (it no longer sandboxes the agent), then retry", problems.join("; ")));
        }
    }

    let baseline = run_probes(agent, &exec.probes).await;
    let ignored: Vec<usize> = baseline.iter().enumerate().filter(|(_, r)| r.is_err()).map(|(i, _)| i).collect();
    for i in &ignored {
        log.line(format!("probe '{}' was already failing before the patch: it will not be held against it", health::describe(&exec.probes[*i])));
    }

    let rp = agent.backend.snapshot(&agent.runner, id, &names, &before, log, &agent.cfg.state_dir).await?;
    if !rp.unrestorable.is_empty() && exec.snapshot_required {
        // On apt the usual cause is a package whose old version has left the repositories: dpkg-repack solves it.
        let hint = if agent.backend.kind() == croustypatch_common::model::PackageManager::Apt { ". Install dpkg-repack on this host (apt install dpkg-repack), or switch off “restore point required” in the policy" } else { "" };
        return Err(anyhow!("no safe restore point (cannot restore: {}); refusing to patch{hint}", rp.unrestorable.join(", ")));
    }
    save_restore_point(&agent.cfg, &rp)?;

    agent.update(id, TaskStatus::Running, format!("applying {} update(s)", names.len()), log, vec![], false);
    if let Err(e) = agent.backend.apply(&agent.runner, &names, log).await {
        log.line(format!("apply failed: {e:#}"));
        if exec.auto_rollback {
            restore_and_verify(agent, id, &rp, &exec.probes, &ignored, &format!("update failed: {e:#}"), log).await;
            return Ok(());
        }
        return Err(anyhow!("update failed and auto-rollback is disabled: {e:#}"));
    }

    let after_inv = agent.backend.inventory(&agent.runner).await?;
    let changes = pkg::diff(&before, &pkg::installed_map(&after_inv));
    let reboot = agent.backend.reboot_required(&agent.runner).await;
    let _ = agent.out.send(AgentMessage::Inventory(Inventory { facts: crate::facts::collect(&agent.cfg.server_url, agent.backend.kind(), vec![]).await, packages: after_inv, reboot_required: reboot, collected_at: Utc::now() }));

    let active: Vec<usize> = (0..exec.probes.len()).filter(|i| !ignored.contains(i)).collect();
    if exec.soak_minutes == 0 || active.is_empty() {
        if exec.soak_minutes > 0 {
            log.line("no healthy probe to watch during the soak window: soak skipped".into());
        }
        agent.update(id, TaskStatus::Succeeded, format!("{} package(s) updated", changes.len()), log, changes, reboot);
        return Ok(());
    }

    agent.update(id, TaskStatus::Soaking, format!("{} package(s) updated; observing for {} min", changes.len(), exec.soak_minutes), log, changes.clone(), reboot);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(exec.soak_minutes as u64 * 60);
    let mut tracker = ProbeTracker::new(&baseline);
    loop {
        tokio::time::sleep(Duration::from_secs(agent.cfg.probe_interval_secs.max(1))).await;
        let results = run_probes(agent, &exec.probes).await;
        if let Some(i) = tracker.record(&results) {
            let what = health::describe(&exec.probes[i]);
            let detail = results[i].clone().err().unwrap_or_default();
            log.line(format!("probe failed: {what}: {detail}"));
            if exec.auto_rollback {
                restore_and_verify(agent, id, &rp, &exec.probes, &ignored, &format!("health probe failed: {what}"), log).await;
            } else {
                agent.update(id, TaskStatus::Failed, format!("health probe failed after patch ({what}); auto-rollback is disabled"), log, changes, reboot);
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
    }
    agent.update(id, TaskStatus::Succeeded, format!("{} package(s) updated; soak window passed without incident", changes.len()), log, changes, reboot);
    Ok(())
}

/// Update compose containers with a restore point, health checks and automatic rollback.
/// Ok(Some(changes)) when done, Ok(None) when the task was closed here (rolled back or failed).
async fn update_containers(agent: &Agent, id: Uuid, names: &[String], retag: &std::collections::BTreeMap<String, String>, exec: &ExecPolicy, log: &TaskLog) -> Result<Option<Vec<PackageChange>>> {
    use crate::modules::{docker, retag as compose};
    agent.update(id, TaskStatus::Running, format!("preparing {} container(s)", names.len()), log, vec![], false);
    let pins = docker::snapshot(&agent.runner, names, log).await?;
    // A new version means changing the tag in the service's compose file. Everything is planned before anything is
    // written: one container that cannot be changed safely stops the whole task, with nothing touched.
    let requests = match docker::retag_requests(&pins, retag) {
        Ok(r) => r,
        Err(e) => {
            agent.update(id, TaskStatus::Failed, format!("{e:#}; nothing was changed"), log, vec![], false);
            return Ok(None);
        }
    };
    let (edits, outcomes) = match compose::plan_all(&requests, &compose::read_files) {
        Ok(x) => x,
        Err(why) => {
            agent.update(id, TaskStatus::Failed, format!("the new version cannot be applied: {why}; nothing was changed"), log, vec![], false);
            return Ok(None);
        }
    };
    for ((service, running, new, _), outcome) in requests.iter().zip(&outcomes) {
        match outcome {
            compose::Outcome::Changed { path } => log.line(format!("{service}: image {running} -> {new} in {path}")),
            compose::Outcome::AlreadyThere { path } => log.line(format!("{service}: {path} already says {new}; the container is recreated on it")),
        }
    }
    let stored: Vec<compose::StoredEdit> = edits.iter().map(compose::FileEdit::stored).collect();
    let rp = RestorePoint { task_id: id, created_at: Utc::now(), pins: vec![], extra: serde_json::json!({ "docker": pins, "compose_edits": stored }), unrestorable: vec![] };
    save_restore_point(&agent.cfg, &rp)?;
    let mut written: Vec<compose::StoredEdit> = vec![];
    for e in &edits {
        if agent.runner.dry_run {
            log.line(format!("[dry-run] would change {}", e.path));
            continue;
        }
        if let Err(err) = compose::write_file(&e.path, &e.edited) {
            for w in &written {
                let _ = compose::revert(w);
            }
            agent.update(id, TaskStatus::Failed, format!("cannot change {}: {err:#}; nothing was changed", e.path), log, vec![], false);
            return Ok(None);
        }
        log.line(format!("changed {}", e.path));
        written.push(e.stored());
    }
    let baseline = run_probes(agent, &exec.probes).await;
    let ignored: Vec<usize> = baseline.iter().enumerate().filter(|(_, r)| r.is_err()).map(|(i, _)| i).collect();
    agent.update(id, TaskStatus::Running, format!("updating {} container(s)", pins.len()), log, vec![], false);
    let mut touched = vec![];
    let applied = match docker::apply(&agent.runner, &pins, log, &mut touched).await {
        Ok(()) => docker::wait_healthy(&agent.runner, &pins, log).await,
        Err(e) => Err(e),
    };
    let probes_bad: Vec<String> = if applied.is_ok() { run_probes(agent, &exec.probes).await.iter().enumerate().filter(|(i, r)| r.is_err() && !ignored.contains(i)).map(|(i, _)| health::describe(&exec.probes[i])).collect() } else { vec![] };
    let failure = match (&applied, probes_bad.is_empty()) {
        (Err(e), _) => Some(format!("{e:#}")),
        (Ok(()), false) => Some(format!("health probe failed after the update ({})", probes_bad.join(", "))),
        _ => None,
    };
    if let Some(why) = failure {
        log.line(format!("container update failed: {why}"));
        // Only containers that were (or may have been) recreated need rolling back. A failure before any of them —
        // a pull that failed, a registry that is down — changed nothing: it is a plain failure, never a reason to
        // isolate the host.
        let changed: Vec<docker::Container> = touched.iter().map(|i| pins[*i].clone()).collect();
        if changed.is_empty() {
            // no container changed: the compose files go back as well, so the host is exactly as it was
            let undone = revert_compose(&written, log);
            let note = match undone { Ok(()) => String::new(), Err(e) => format!(" (the compose file could not be put back: {e:#})") };
            agent.update(id, TaskStatus::Failed, format!("{why}; nothing was changed{note}"), log, vec![], false);
            return Ok(None);
        }
        if !exec.auto_rollback {
            let note = if written.is_empty() { String::new() } else { " (the compose file keeps the new tag)".to_string() };
            agent.update(id, TaskStatus::Failed, format!("{why}; auto-rollback is disabled{note}"), log, vec![], false);
            return Ok(None);
        }
        agent.update(id, TaskStatus::RollingBack, format!("rolling back: {why}"), log, vec![], false);
        let names = changed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
        // the compose files must say the previous tags again before the previous images are put back
        let outcome = match revert_compose(&written, log) {
            Ok(()) => match docker::restore(&agent.runner, &changed, log).await {
                Ok(()) => docker::wait_healthy(&agent.runner, &changed, log).await,
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        match outcome {
            Ok(()) => {
                let _ = agent.send_inventory().await;
                agent.update(id, TaskStatus::RolledBack, format!("{names} back on the previous image ({why})"), log, vec![], false);
            }
            // What matters is the machine, not an exit code: if every container runs its previous image again,
            // the rollback worked even though a command complained.
            Err(e) if !agent.runner.dry_run && docker::running_on_pins(&agent.runner, &changed).await => {
                log.line(format!("rollback reported an error ({e:#}), but {names} run the previous image again"));
                let _ = agent.send_inventory().await;
                agent.update(id, TaskStatus::RolledBack, format!("{names} back on the previous image ({why})"), log, vec![], false);
            }
            Err(e) => double_failure(agent, id, &format!("rollback of {names} failed: {e:#}"), Some(&rp), log).await,
        }
        return Ok(None);
    }
    let now = docker::containers(&agent.runner).await.unwrap_or_default();
    let changes = pins
        .iter()
        .map(|p| PackageChange { name: format!("{}{}", docker::PREFIX, p.name), from: Some(p.image_id.chars().take(19).collect()), to: now.iter().find(|c| c.name == p.name).map(|c| c.image_id.chars().take(19).collect()) })
        .collect();
    let _ = agent.send_inventory().await;
    Ok(Some(changes))
}

/// Put the compose files back as they were (each only if nobody changed it since the update).
fn revert_compose(written: &[crate::modules::retag::StoredEdit], log: &TaskLog) -> Result<()> {
    let mut failed = vec![];
    for w in written {
        match crate::modules::retag::revert(w) {
            Ok(()) => log.line(format!("compose file {} put back as it was", w.path)),
            Err(e) => {
                log.line(format!("{e:#}"));
                failed.push(format!("{e:#}"));
            }
        }
    }
    if failed.is_empty() { Ok(()) } else { Err(anyhow!(failed.join("; "))) }
}

async fn manual_rollback(agent: &Agent, id: Uuid, target: Uuid, log: &TaskLog) -> Result<()> {
    let rp = load_restore_point(&agent.cfg, target)?;
    if let Some(pins) = rp.extra.get("docker").and_then(|v| serde_json::from_value::<Vec<crate::modules::docker::Container>>(v.clone()).ok()) {
        agent.update(id, TaskStatus::Running, format!("rolling back task {target} (containers)"), log, vec![], false);
        // a version change also changed compose files: they say the previous tags again before the containers are recreated
        let edits: Vec<crate::modules::retag::StoredEdit> = rp.extra.get("compose_edits").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        if let Err(e) = revert_compose(&edits, log) {
            agent.update(id, TaskStatus::Failed, format!("cannot put the compose file back: {e:#}; the containers were not touched"), log, vec![], false);
            return Ok(());
        }
        match crate::modules::docker::restore(&agent.runner, &pins, log).await {
            Ok(()) => {
                let _ = agent.send_inventory().await;
                agent.update(id, TaskStatus::Succeeded, format!("{} container(s) back on their previous image", pins.len()), log, vec![], false);
            }
            Err(e) => double_failure(agent, id, &format!("rollback failed: {e:#}"), Some(&rp), log).await,
        }
        return Ok(());
    }
    agent.update(id, TaskStatus::Running, format!("rolling back task {target}"), log, vec![], false);
    let before = pkg::installed_map(&agent.backend.inventory(&agent.runner).await?);
    match agent.backend.restore(&agent.runner, &rp, log).await {
        Ok(()) => {
            let after = agent.send_inventory().await?;
            let changes = pkg::diff(&before, &pkg::installed_map(&after.packages));
            agent.update(id, TaskStatus::Succeeded, format!("rolled back {} package(s)", changes.len()), log, changes, after.reboot_required);
        }
        Err(e) => double_failure(agent, id, &format!("rollback failed: {e:#}"), Some(&rp), log).await,
    }
    Ok(())
}
