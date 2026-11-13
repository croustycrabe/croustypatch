//! Package-manager backends. Each backend is a small module of parsers (pure, unit-tested) and
//! command wrappers; `Backend` dispatches between them.
use crate::exec::Runner;
use crate::runner::TaskLog;
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use croustypatch_common::model::PackageManager;
use croustypatch_common::protocol::{PackageChange, PackageEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub mod apt;
pub mod mock;
pub mod pacman;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pin {
    pub name: String,
    pub version: String,
}

/// Everything needed to put the packages back the way they were.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestorePoint {
    pub task_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub pins: Vec<Pin>,
    /// Backend-specific data (cached package files, boot environment name…).
    #[serde(default)]
    pub extra: serde_json::Value,
    /// Pins whose old version can no longer be fetched or found locally.
    #[serde(default)]
    pub unrestorable: Vec<String>,
}

pub enum Backend {
    Apt,
    Pacman,
    Mock(mock::Mock),
}

impl Backend {
    pub fn select(kind: PackageManager, mock_file: &str) -> Result<Self> {
        Ok(match kind {
            PackageManager::Apt => Backend::Apt,
            PackageManager::Pacman => Backend::Pacman,
            _ if !mock_file.is_empty() => Backend::Mock(mock::Mock::new(mock_file.into())),
            _ => bail!("no supported package manager found (apt and pacman are supported)"),
        })
    }

    pub fn kind(&self) -> PackageManager {
        match self {
            Backend::Apt => PackageManager::Apt,
            Backend::Pacman => PackageManager::Pacman,
            Backend::Mock(_) => PackageManager::Unknown,
        }
    }

    /// Installed packages with pending candidates.
    pub async fn inventory(&self, r: &Runner) -> Result<Vec<PackageEntry>> {
        match self {
            Backend::Apt => apt::inventory(r).await,
            Backend::Pacman => pacman::inventory(r).await,
            Backend::Mock(m) => m.inventory().await,
        }
    }

    /// Refresh repository metadata (needs root; failures are logged, not fatal).
    pub async fn refresh(&self, r: &Runner, log: &TaskLog) {
        let res = match self {
            Backend::Apt => apt::refresh(r, log).await,
            Backend::Pacman => Ok(()), // pacman's sync DB is refreshed as part of the full upgrade
            Backend::Mock(_) => Ok(()),
        };
        if let Err(e) = res {
            log.line(format!("repository refresh failed (continuing with cached metadata): {e}"));
        }
    }

    pub async fn snapshot(&self, r: &Runner, task: Uuid, names: &[String], installed: &HashMap<String, String>, log: &TaskLog, state_dir: &std::path::Path) -> Result<RestorePoint> {
        match self {
            Backend::Apt => apt::snapshot(r, task, names, installed, log, state_dir).await,
            Backend::Pacman => pacman::snapshot(task, names, installed, log),
            Backend::Mock(m) => m.snapshot(task, names, installed),
        }
    }

    pub async fn apply(&self, r: &Runner, names: &[String], log: &TaskLog) -> Result<()> {
        match self {
            Backend::Apt => apt::apply(r, names, log).await,
            Backend::Pacman => pacman::apply(r, log).await,
            Backend::Mock(m) => m.apply(names).await,
        }
    }

    /// Install packages (app center). Names are validated by the caller.
    pub async fn install(&self, r: &Runner, names: &[String], log: &TaskLog) -> Result<()> {
        use crate::exec::Cmd;
        let cmd = match self {
            Backend::Apt => Cmd::new("apt-get", &["install", "-y", "--no-install-recommends", "-o", "Dpkg::Options::=--force-confold", "-o", "Dpkg::Options::=--force-confdef"]).args(apt::sandbox_opts_pub(log)).env("DEBIAN_FRONTEND", "noninteractive"),
            Backend::Pacman => Cmd::new("pacman", &["-S", "--needed", "--noconfirm"]),
            Backend::Mock(m) => return m.install(names),
        };
        let out = r.mutate(cmd.args(names.iter().cloned()).timeout(3600), log).await?;
        if !out.ok() {
            bail!("installation exited with {}: {}", out.code, out.stderr.lines().last().unwrap_or("").trim());
        }
        Ok(())
    }

    /// Remove packages (app center).
    pub async fn remove(&self, r: &Runner, names: &[String], log: &TaskLog) -> Result<()> {
        use crate::exec::Cmd;
        let cmd = match self {
            Backend::Apt => Cmd::new("apt-get", &["remove", "-y"]).args(apt::sandbox_opts_pub(log)).env("DEBIAN_FRONTEND", "noninteractive"),
            Backend::Pacman => Cmd::new("pacman", &["-R", "--noconfirm"]),
            Backend::Mock(m) => return m.remove(names),
        };
        let out = r.mutate(cmd.args(names.iter().cloned()).timeout(1800), log).await?;
        if !out.ok() {
            bail!("removal exited with {}: {}", out.code, out.stderr.lines().last().unwrap_or("").trim());
        }
        Ok(())
    }

    pub async fn restore(&self, r: &Runner, rp: &RestorePoint, log: &TaskLog) -> Result<()> {
        match self {
            Backend::Apt => apt::restore(r, rp, log).await,
            Backend::Pacman => pacman::restore(r, rp, log).await,
            Backend::Mock(m) => m.restore(rp).await,
        }
    }

    pub async fn reboot_required(&self, r: &Runner) -> bool {
        match self {
            Backend::Apt => apt::reboot_required(),
            Backend::Pacman => pacman::reboot_required(r).await,
            Backend::Mock(m) => m.reboot_required().await,
        }
    }

    /// Does the backend always upgrade the whole system regardless of the selection?
    pub fn full_upgrade_only(&self) -> bool {
        matches!(self, Backend::Pacman)
    }
}

pub fn installed_map(inv: &[PackageEntry]) -> HashMap<String, String> {
    inv.iter().map(|p| (p.name.clone(), p.version.clone())).collect()
}

/// Compare two installed-version maps.
pub fn diff(before: &HashMap<String, String>, after: &HashMap<String, String>) -> Vec<PackageChange> {
    let mut out: Vec<PackageChange> = after
        .iter()
        .filter_map(|(n, v)| match before.get(n) {
            Some(old) if old == v => None,
            old => Some(PackageChange { name: n.clone(), from: old.cloned(), to: Some(v.clone()) }),
        })
        .collect();
    out.extend(before.iter().filter(|(n, _)| !after.contains_key(*n)).map(|(n, v)| PackageChange { name: n.clone(), from: Some(v.clone()), to: None }));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Parse "name old -> new" lines (pacman `-Qu` / `checkupdates`).
pub fn parse_arrow_updates(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (n, o, arrow, new) = (it.next()?, it.next()?, it.next()?, it.next()?);
            (arrow == "->").then(|| (n.to_string(), o.to_string(), new.to_string()))
        })
        .collect()
}
