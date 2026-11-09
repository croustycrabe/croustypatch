//! Arch Linux. Arch does not support partial upgrades, so patching is always a full `-Syu`.
use super::{parse_arrow_updates, Pin, RestorePoint};
use crate::exec::{self, Cmd, Runner};
use crate::runner::TaskLog;
use anyhow::{bail, Result};
use chrono::Utc;
use croustypatch_common::protocol::PackageEntry;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use uuid::Uuid;

pub fn parse_installed(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| l.split_once(' ').map(|(n, v)| (n.to_string(), v.trim().to_string()))).collect()
}

/// `arch-audit` lines: `Package openssl is affected by CVE-2022-4450, CVE-2022-4304. High risk! Update to 3.0.8-1!`
/// Returns name -> (CVEs, fix available).
pub fn parse_arch_audit(text: &str) -> HashMap<String, (Vec<String>, bool)> {
    static LINE: OnceLock<Regex> = OnceLock::new();
    static CVE: OnceLock<Regex> = OnceLock::new();
    let line = LINE.get_or_init(|| Regex::new(r"^Package (\S+) is affected by (.*)$").unwrap());
    let cve = CVE.get_or_init(|| Regex::new(r"CVE-\d{4}-\d{4,}").unwrap());
    let mut out: HashMap<String, (Vec<String>, bool)> = HashMap::new();
    for l in text.lines() {
        let Some(c) = line.captures(l.trim()) else { continue };
        let e = out.entry(c[1].to_string()).or_default();
        e.0.extend(cve.find_iter(&c[2]).map(|m| m.as_str().to_string()));
        e.1 |= c[2].contains("Update to");
    }
    out
}

pub async fn inventory(r: &Runner) -> Result<Vec<PackageEntry>> {
    let q = r.read(Cmd::new("pacman", &["-Q"])).await?;
    if !q.ok() {
        bail!("pacman -Q failed: {}", q.stderr.trim());
    }
    // checkupdates uses a private DB copy, so it never leaves the system half-synced.
    let upd = if exec::has_program("checkupdates") { r.read(Cmd::new("checkupdates", &[]).timeout(300)).await? } else { r.read(Cmd::new("pacman", &["-Qu"])).await? };
    let updates: HashMap<String, String> = parse_arrow_updates(&upd.stdout).into_iter().map(|(n, _, new)| (n, new)).collect();
    let audit = if exec::has_program("arch-audit") { r.read(Cmd::new("arch-audit", &[]).timeout(120)).await.ok().map(|o| parse_arch_audit(&o.stdout)).unwrap_or_default() } else { HashMap::new() };
    Ok(parse_installed(&q.stdout)
        .into_iter()
        .map(|(name, version)| {
            let candidate = updates.get(&name).cloned();
            let (cves, fixed) = audit.get(&name).cloned().unwrap_or_default();
            PackageEntry { is_security: candidate.is_some() && fixed, cves, candidate, source: "system".into(), name, version }
        })
        .collect())
}

pub fn cache_file(dir: &Path, name: &str, version: &str) -> Option<String> {
    let prefix = format!("{name}-{version}-");
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let f = e.file_name().to_string_lossy().into_owned();
        // "<name>-<version>-<arch>.pkg.tar.<ext>": the remainder must be exactly an arch + extension
        if let Some(rest) = f.strip_prefix(&prefix) {
            if !rest.contains('-') && rest.contains(".pkg.tar") && !rest.ends_with(".sig") {
                return Some(e.path().to_string_lossy().into_owned());
            }
        }
    }
    None
}

pub fn snapshot(task: Uuid, names: &[String], installed: &HashMap<String, String>, log: &TaskLog) -> Result<RestorePoint> {
    // A full upgrade touches everything with a candidate; the pending list is `names`.
    let dir = Path::new("/var/cache/pacman/pkg");
    let (mut pins, mut files, mut missing) = (vec![], vec![], vec![]);
    let wanted: HashSet<&String> = names.iter().collect();
    for (n, v) in installed.iter().filter(|(n, _)| wanted.contains(n)) {
        pins.push(Pin { name: n.clone(), version: v.clone() });
        match cache_file(dir, n, v) {
            Some(f) => files.push(f),
            None => missing.push(format!("{n}={v}")),
        }
    }
    log.line(format!("restore point: {} packages pinned, {} found in the pacman cache, {} missing", pins.len(), files.len(), missing.len()));
    Ok(RestorePoint { task_id: task, created_at: Utc::now(), pins, extra: serde_json::json!({ "files": files }), unrestorable: missing })
}

pub async fn apply(r: &Runner, log: &TaskLog) -> Result<()> {
    let out = r.mutate(Cmd::new("pacman", &["-Syu", "--noconfirm"]).timeout(3600), log).await?;
    if !out.ok() {
        bail!("pacman -Syu exited with {}", out.code);
    }
    Ok(())
}

pub async fn restore(r: &Runner, rp: &RestorePoint, log: &TaskLog) -> Result<()> {
    let files: Vec<String> = rp.extra.get("files").and_then(|f| f.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    if files.is_empty() {
        bail!("no cached package files to restore from");
    }
    let out = r.mutate(Cmd::new("pacman", &["-U", "--noconfirm"]).args(files).timeout(3600), log).await?;
    if !out.ok() {
        bail!("pacman -U exited with {}", out.code);
    }
    Ok(())
}

/// The running kernel's modules disappear when the kernel package is upgraded.
pub async fn reboot_required(r: &Runner) -> bool {
    let Ok(out) = r.read(Cmd::new("uname", &["-r"])).await else { return false };
    let running = out.stdout.trim();
    !running.is_empty() && !Path::new("/usr/lib/modules").join(running).exists()
}
