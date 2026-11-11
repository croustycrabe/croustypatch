//! FreeBSD `pkg(8)`; base-system updates (`freebsd-update`) are out of scope for milestone 1.
use super::{Pin, RestorePoint};
use crate::exec::{self, Cmd, Runner};
use crate::runner::TaskLog;
use anyhow::{bail, Result};
use chrono::Utc;
use croustypatch_common::protocol::PackageEntry;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use uuid::Uuid;

pub fn parse_installed(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| l.split_once('\t').map(|(n, v)| (n.to_string(), v.trim().to_string()))).collect()
}

/// `pkg upgrade -n` lines: `\tcurl: 8.4.0 -> 8.5.0`
pub fn parse_upgrade_plan(text: &str) -> Vec<(String, String, String)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^\s+(\S+): (\S+) -> (\S+)\s*$").unwrap());
    text.lines().filter_map(|l| re.captures(l).map(|c| (c[1].to_string(), c[2].to_string(), c[3].to_string()))).collect()
}

/// `pkg audit` output → package name -> CVEs.
/// ```text
/// curl-8.4.0 is vulnerable:
///   curl -- multiple vulnerabilities
///   CVE: CVE-2023-46218
///   WWW: https://vuxml.FreeBSD.org/freebsd/xxxx.html
/// ```
pub fn parse_audit(text: &str) -> HashMap<String, Vec<String>> {
    static HEAD: OnceLock<Regex> = OnceLock::new();
    static CVE: OnceLock<Regex> = OnceLock::new();
    let head = HEAD.get_or_init(|| Regex::new(r"^(\S+)-[^-\s]+ is vulnerable:").unwrap());
    let cve = CVE.get_or_init(|| Regex::new(r"CVE-\d{4}-\d{4,}").unwrap());
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    for l in text.lines() {
        if let Some(c) = head.captures(l) {
            current = Some(c[1].to_string());
            out.entry(c[1].to_string()).or_default();
        } else if let (Some(cur), true) = (&current, l.trim_start().starts_with("CVE:")) {
            out.entry(cur.clone()).or_default().extend(cve.find_iter(l).map(|m| m.as_str().to_string()));
        }
    }
    out
}

pub async fn refresh(r: &Runner, log: &TaskLog) -> Result<()> {
    let out = r.mutate(Cmd::new("pkg", &["update", "-q"]).timeout(600), log).await?;
    if !out.ok() {
        bail!("pkg update exited with {}", out.code);
    }
    // Best effort: fetch the latest vulnerability database for `pkg audit`.
    let _ = r.mutate(Cmd::new("pkg", &["audit", "-F", "-q"]).timeout(300), log).await;
    Ok(())
}

pub async fn inventory(r: &Runner) -> Result<Vec<PackageEntry>> {
    let q = r.read(Cmd::new("pkg", &["query", "%n\t%v"]).timeout(120)).await?;
    if !q.ok() {
        bail!("pkg query failed: {}", q.stderr.trim());
    }
    let plan = r.read(Cmd::new("pkg", &["upgrade", "-n"]).timeout(300)).await?;
    let updates: HashMap<String, String> = parse_upgrade_plan(&format!("{}\n{}", plan.stdout, plan.stderr)).into_iter().map(|(n, _, new)| (n, new)).collect();
    let audit = r.read(Cmd::new("pkg", &["audit"]).timeout(120)).await.ok().map(|o| parse_audit(&o.stdout)).unwrap_or_default();
    Ok(parse_installed(&q.stdout)
        .into_iter()
        .map(|(name, version)| {
            let candidate = updates.get(&name).cloned();
            let cves = audit.get(&name).cloned().unwrap_or_default();
            PackageEntry { is_security: candidate.is_some() && audit.contains_key(&name), cves, candidate, source: "system".into(), meta: Default::default(), name, version }
        })
        .collect())
}

pub async fn snapshot(r: &Runner, task: Uuid, names: &[String], installed: &HashMap<String, String>, log: &TaskLog, state_dir: &Path) -> Result<RestorePoint> {
    let dir = state_dir.join("pkgbackup").join(task.to_string());
    let mut pins = vec![];
    let mut files = vec![];
    let mut missing = vec![];
    // A ZFS boot environment is a cheap whole-system safety net where available.
    let mut be = None;
    if exec::has_program("bectl") && !r.dry_run {
        let name = format!("croustypatch-{}", &task.to_string()[..8]);
        if r.mutate(Cmd::new("bectl", &["create", &name]), log).await.map(|o| o.ok()).unwrap_or(false) {
            be = Some(name);
        }
    }
    if !r.dry_run {
        std::fs::create_dir_all(&dir)?;
    }
    for n in names {
        let Some(v) = installed.get(n) else { continue };
        pins.push(Pin { name: n.clone(), version: v.clone() });
        // `pkg create` rebuilds a package archive from the installed files: our rollback source.
        let out = r.mutate(Cmd::new("pkg", &["create", "-o", &dir.to_string_lossy(), n]).timeout(600), log).await;
        let file = dir.join(format!("{n}-{v}.pkg"));
        if out.map(|o| o.ok()).unwrap_or(false) && (r.dry_run || file.exists() || dir.join(format!("{n}-{v}.txz")).exists()) {
            files.push(file.to_string_lossy().into_owned());
        } else {
            missing.push(format!("{n}={v}"));
        }
    }
    Ok(RestorePoint { task_id: task, created_at: Utc::now(), pins, extra: serde_json::json!({ "files": files, "boot_environment": be }), unrestorable: missing })
}

pub async fn apply(r: &Runner, names: &[String], log: &TaskLog) -> Result<()> {
    let out = r.mutate(Cmd::new("pkg", &["upgrade", "-y"]).args(names.iter().cloned()).timeout(3600), log).await?;
    if !out.ok() {
        bail!("pkg upgrade exited with {}", out.code);
    }
    Ok(())
}

pub async fn restore(r: &Runner, rp: &RestorePoint, log: &TaskLog) -> Result<()> {
    let files: Vec<String> = rp.extra.get("files").and_then(|f| f.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    if files.is_empty() {
        bail!("no package backups to restore from");
    }
    for f in files {
        let out = r.mutate(Cmd::new("pkg", &["add", "-f", &f]).timeout(600), log).await?;
        if !out.ok() {
            bail!("pkg add {f} exited with {}", out.code);
        }
    }
    Ok(())
}

pub async fn reboot_required(r: &Runner) -> bool {
    let (Ok(inst), Ok(run)) = (r.read(Cmd::new("freebsd-version", &["-k"])).await, r.read(Cmd::new("uname", &["-r"])).await) else { return false };
    let (i, u) = (inst.stdout.trim(), run.stdout.trim());
    !i.is_empty() && !u.is_empty() && i != u
}
