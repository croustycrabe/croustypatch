//! Debian / Ubuntu.
use super::{Pin, RestorePoint};
use crate::exec::{self, Cmd, Runner};
use crate::runner::TaskLog;
use anyhow::{bail, Result};
use chrono::Utc;
use croustypatch_common::protocol::PackageEntry;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use uuid::Uuid;

pub fn parse_dpkg_installed(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            let (name, ver, status) = (it.next()?, it.next()?, it.next()?);
            status.starts_with("ii").then(|| (name.to_string(), ver.to_string()))
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct Upgrade {
    pub name: String,
    pub old: String,
    pub new: String,
    pub security: bool,
}

/// Parse `apt-get -s dist-upgrade` "Inst" lines.
/// `Inst libssl3 [3.0.2-1] (3.0.2-2 Ubuntu:22.04/jammy-updates, Ubuntu:22.04/jammy-security [amd64])`
pub fn parse_simulated_upgrades(text: &str) -> Vec<Upgrade> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^Inst (?P<n>\S+) \[(?P<old>[^\]]+)\] \((?P<new>\S+) (?P<origin>[^\)]*?)\s*(?:\[[^\]]*\])?\)").unwrap());
    text.lines()
        .filter_map(|l| {
            let c = re.captures(l)?;
            Some(Upgrade { name: c["n"].into(), old: c["old"].into(), new: c["new"].into(), security: c["origin"].to_ascii_lowercase().contains("security") })
        })
        .collect()
}

pub async fn inventory(r: &Runner) -> Result<Vec<PackageEntry>> {
    let installed = r.read(Cmd::new("dpkg-query", &["-W", "-f=${Package}\t${Version}\t${db:Status-Abbrev}\n"]).timeout(120)).await?;
    if !installed.ok() {
        bail!("dpkg-query failed: {}", installed.stderr.trim());
    }
    let sim = r.read(Cmd::new("apt-get", &["-s", "-o", "Debug::NoLocking=1", "dist-upgrade"]).timeout(180)).await?;
    let ups: HashMap<String, Upgrade> = parse_simulated_upgrades(&sim.stdout).into_iter().map(|u| (u.name.clone(), u)).collect();
    Ok(parse_dpkg_installed(&installed.stdout)
        .into_iter()
        .map(|(name, version)| {
            let up = ups.get(&name);
            PackageEntry { candidate: up.map(|u| u.new.clone()), is_security: up.is_some_and(|u| u.security), cves: vec![], source: "system".into(), meta: Default::default(), name, version }
        })
        .collect())
}

/// apt drops to its `_apt` user for downloads, which needs CAP_SETUID and CAP_SETGID (bits 7 and 6 of CapEff).
/// Inside some containers and sandboxed services those are missing and apt dies with "seteuid 42 failed".
pub fn has_cap_setuid(proc_status: &str) -> bool {
    proc_status
        .lines()
        .find_map(|l| l.strip_prefix("CapEff:"))
        .and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
        .map(|m| m & 0xC0 == 0xC0)
        .unwrap_or(true) // unknown: keep apt's default behaviour
}

/// Extra apt options for this process. Without the capability the sandbox user cannot be assumed, so tell apt to
/// download as root (the agent is root anyway); with it, apt keeps its sandbox.
/// For callers outside this module (app center).
pub fn sandbox_opts_pub(log: &TaskLog) -> Vec<String> {
    sandbox_opts(Some(log))
}

fn sandbox_opts(log: Option<&TaskLog>) -> Vec<String> {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    if has_cap_setuid(&status) {
        return vec![];
    }
    if let Some(l) = log {
        l.line("this process lacks CAP_SETUID/CAP_SETGID: apt's download sandbox is disabled (APT::Sandbox::User=root)".into());
    }
    vec!["-o".into(), "APT::Sandbox::User=root".into()]
}

pub async fn refresh(r: &Runner, log: &TaskLog) -> Result<()> {
    let out = r.mutate(Cmd::new("apt-get", &["update", "-qq"]).args(sandbox_opts(Some(log))).env("DEBIAN_FRONTEND", "noninteractive").timeout(600), log).await?;
    if !out.ok() {
        bail!("apt-get update exited with {}", out.code);
    }
    Ok(())
}

/// Can `name=version` still be installed? True if apt knows it, or a cached .deb exists.
pub fn madison_has(text: &str, version: &str) -> bool {
    text.lines().any(|l| l.split('|').nth(1).map(|v| v.trim() == version).unwrap_or(false))
}

/// dpkg names package files `<name>_<version, ':' written as %3a>_<arch>.deb`.
pub fn deb_file_matches(file: &str, name: &str, version: &str) -> bool {
    file.ends_with(".deb") && file.starts_with(&format!("{name}_{}_", version.replace(':', "%3a")))
}

fn find_deb(dir: &Path, name: &str, version: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find(|e| deb_file_matches(&e.file_name().to_string_lossy(), name, version)).map(|e| e.path())
}

/// Build a restore point, per package, from the most reliable source available:
///  1. a copy of the installed version's .deb (apt's cache),
///  2. the repositories, while they still list that exact version,
///  3. `dpkg-repack`, which rebuilds the package from the files that are installed right now.
/// The old version of an upgraded package is usually *gone* from the repositories (the index only keeps the newest
/// per pocket), so (2) alone would refuse most real patches.
pub async fn snapshot(r: &Runner, task: Uuid, names: &[String], installed: &HashMap<String, String>, log: &TaskLog, state_dir: &Path) -> Result<RestorePoint> {
    let dir = state_dir.join("pkgbackup").join(task.to_string());
    let repack = exec::has_program("dpkg-repack");
    let (mut pins, mut files, mut repo, mut unrestorable) = (vec![], vec![], vec![], vec![]);
    for n in names {
        let Some(v) = installed.get(n) else { continue }; // new dependency: nothing to restore
        pins.push(Pin { name: n.clone(), version: v.clone() });
        if let Some(src) = find_deb(Path::new("/var/cache/apt/archives"), n, v) {
            if !r.dry_run {
                std::fs::create_dir_all(&dir)?;
                let dst = dir.join(src.file_name().unwrap_or_default());
                std::fs::copy(&src, &dst)?;
                files.push(dst.to_string_lossy().into_owned());
            }
            continue;
        }
        let madison = r.read(Cmd::new("apt-cache", &["madison", n])).await?;
        if madison_has(&madison.stdout, v) {
            repo.push(n.clone());
            continue;
        }
        if repack {
            if r.dry_run {
                continue; // rehearsal: nothing is written, but the real run could do it
            }
            std::fs::create_dir_all(&dir)?;
            let out = r.mutate(Cmd::new("dpkg-repack", &[n]).cwd(&dir).timeout(900), log).await;
            if out.map(|o| o.ok()).unwrap_or(false) {
                if let Some(f) = std::fs::read_dir(&dir)?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&format!("{n}_")) && f.to_string_lossy().ends_with(".deb"))) {
                    files.push(f.to_string_lossy().into_owned());
                    continue;
                }
            }
        }
        unrestorable.push(format!("{n}={v}"));
    }
    log.line(format!("restore point: {} packages pinned ({} saved as package files, {} available in the repositories), {} not restorable", pins.len(), files.len(), repo.len(), unrestorable.len()));
    if !unrestorable.is_empty() && !repack {
        log.line("tip: install dpkg-repack on this host (apt install dpkg-repack) so packages whose old version left the repositories can still be restored".into());
    }
    Ok(RestorePoint { task_id: task, created_at: Utc::now(), pins, extra: serde_json::json!({ "files": files, "repo": repo }), unrestorable })
}

pub async fn apply(r: &Runner, names: &[String], log: &TaskLog) -> Result<()> {
    let cmd = Cmd::new("apt-get", &["install", "--only-upgrade", "-y", "-o", "Dpkg::Options::=--force-confold", "-o", "Dpkg::Options::=--force-confdef"])
        .args(sandbox_opts(Some(log)))
        .args(names.iter().cloned())
        .env("DEBIAN_FRONTEND", "noninteractive")
        .timeout(3600);
    let out = r.mutate(cmd, log).await?;
    if !out.ok() {
        bail!("apt-get install exited with {}", out.code);
    }
    Ok(())
}

/// Put the previous versions back: package files with dpkg first, then repository-backed ones with apt.
pub async fn restore(r: &Runner, rp: &RestorePoint, log: &TaskLog) -> Result<()> {
    if rp.pins.is_empty() {
        return Ok(());
    }
    let strings = |k: &str| -> Option<Vec<String>> { rp.extra.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()) };
    let files = strings("files").unwrap_or_default();
    // Restore points written before this format existed carry no `repo` list: every pin then comes from the repositories.
    let repo_names = strings("repo");
    if !files.is_empty() {
        let cmd = Cmd::new("dpkg", &["-i", "--force-downgrade", "--force-confold"]).args(files.iter().cloned()).env("DEBIAN_FRONTEND", "noninteractive").timeout(3600);
        let out = r.mutate(cmd, log).await?;
        if !out.ok() {
            bail!("dpkg -i exited with {}", out.code);
        }
    }
    let from_repo: Vec<String> = rp
        .pins
        .iter()
        .filter(|p| match &repo_names {
            Some(list) => list.contains(&p.name),
            None => true,
        })
        .map(|p| format!("{}={}", p.name, p.version))
        .collect();
    if !from_repo.is_empty() {
        let cmd = Cmd::new("apt-get", &["install", "-y", "--allow-downgrades", "-o", "Dpkg::Options::=--force-confold"]).args(sandbox_opts(Some(log))).args(from_repo).env("DEBIAN_FRONTEND", "noninteractive").timeout(3600);
        let out = r.mutate(cmd, log).await?;
        if !out.ok() {
            bail!("apt-get downgrade exited with {}", out.code);
        }
    }
    Ok(())
}

pub fn reboot_required() -> bool {
    std::path::Path::new("/var/run/reboot-required").exists() || std::path::Path::new("/run/reboot-required").exists()
}
