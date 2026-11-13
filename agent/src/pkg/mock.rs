//! Test backend: a JSON file stands in for the package database, so the full patch pipeline
//! (approval, signed command, snapshot, soak, rollback, quarantine) can be exercised on any machine.
//! Only selectable by explicit configuration (`mock_file`); never auto-detected.
use super::{Pin, RestorePoint};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use croustypatch_common::protocol::PackageEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MockDb {
    pub installed: Vec<MockPkg>,
    #[serde(default)]
    pub reboot_required: bool,
    /// Make `apply` fail, to test the failure path.
    #[serde(default)]
    pub fail_apply: bool,
    /// Make `restore` fail, to test the double-failure protocol.
    #[serde(default)]
    pub fail_restore: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockPkg {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub candidate: Option<String>,
    #[serde(default)]
    pub is_security: bool,
    #[serde(default)]
    pub cves: Vec<String>,
}

pub struct Mock {
    path: PathBuf,
}

impl Mock {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    fn load(&self) -> Result<MockDb> {
        serde_json::from_str(&std::fs::read_to_string(&self.path).with_context(|| format!("reading {}", self.path.display()))?).context("mock db is not valid JSON")
    }
    fn save(&self, db: &MockDb) -> Result<()> {
        std::fs::write(&self.path, serde_json::to_string_pretty(db)?)?;
        Ok(())
    }

    pub async fn inventory(&self) -> Result<Vec<PackageEntry>> {
        Ok(self.load()?.installed.into_iter().map(|p| PackageEntry { name: p.name, version: p.version, candidate: p.candidate, is_security: p.is_security, cves: p.cves, source: "system".into(), meta: Default::default() }).collect())
    }

    pub fn snapshot(&self, task: Uuid, names: &[String], installed: &HashMap<String, String>) -> Result<RestorePoint> {
        let pins = names.iter().filter_map(|n| installed.get(n).map(|v| Pin { name: n.clone(), version: v.clone() })).collect();
        Ok(RestorePoint { task_id: task, created_at: Utc::now(), pins, extra: serde_json::Value::Null, unrestorable: vec![] })
    }

    pub fn install(&self, names: &[String]) -> Result<()> {
        let mut db = self.load()?;
        for n in names {
            if !db.installed.iter().any(|p| &p.name == n) {
                db.installed.push(MockPkg { name: n.clone(), version: "1.0".into(), candidate: None, is_security: false, cves: vec![] });
            }
        }
        self.save(&db)
    }

    pub fn remove(&self, names: &[String]) -> Result<()> {
        let mut db = self.load()?;
        db.installed.retain(|p| !names.contains(&p.name));
        self.save(&db)
    }

    pub async fn apply(&self, names: &[String]) -> Result<()> {
        let mut db = self.load()?;
        if db.fail_apply {
            bail!("mock: apply failed on purpose");
        }
        for p in db.installed.iter_mut().filter(|p| names.contains(&p.name)) {
            if let Some(c) = p.candidate.take() {
                p.version = c;
                p.is_security = false;
                if p.name.starts_with("linux") || p.name.starts_with("kernel") {
                    db.reboot_required = true;
                }
            }
        }
        // `db.installed` borrow ended above; persist.
        let flag = db.reboot_required;
        let _ = flag;
        self.save(&db)
    }

    pub async fn restore(&self, rp: &RestorePoint) -> Result<()> {
        let mut db = self.load()?;
        if db.fail_restore {
            bail!("mock: restore failed on purpose");
        }
        for pin in &rp.pins {
            if let Some(p) = db.installed.iter_mut().find(|p| p.name == pin.name) {
                p.candidate = Some(p.version.clone());
                p.version = pin.version.clone();
            }
        }
        self.save(&db)
    }

    pub async fn reboot_required(&self) -> bool {
        self.load().map(|d| d.reboot_required).unwrap_or(false)
    }
}
