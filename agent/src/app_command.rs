//! App-center command recipes: admin-written install / uninstall commands for software that is not in the system's
//! repositories (internal tools, vendor installers).
//!
//! The command comes inside a server-signed order and runs as root through `sh -c`, with a cleared environment, a
//! `umask 022` (so what it creates is readable by everyone) and a timeout. Nothing from the requesting user is ever
//! interpolated into it: the user's identity is passed as environment variables only (`CROUSTYPATCH_USER`,
//! `CROUSTYPATCH_USER_HOME`, `CROUSTYPATCH_USER_UID`, `CROUSTYPATCH_USER_GID`), for scripts that set up per-user
//! files. After an install the agent checks that the recipe's `check` path exists **and is usable by the user who
//! asked** (or by every user when the request came from the web UI), so an app installed only for root is reported as
//! a failure instead of a success.
use crate::exec::{self, Cmd};
use crate::runner::TaskLog;
use anyhow::{anyhow, bail, Result};
use croustypatch_common::protocol::AppCommand;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Commands may download and build things: give them time.
pub const TIMEOUT_SECS: u64 = 30 * 60;

/// Who must be able to use the installed application.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
    pub home: String,
}

impl Target {
    /// "Every user": only the permission bits for others count.
    pub fn everyone() -> Self {
        Self { name: "every user".into(), uid: u32::MAX - 1, gid: u32::MAX - 1, groups: vec![], home: String::new() }
    }
}

fn valid_user_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) && !s.starts_with('-')
}

/// Look a local account up through the system's name service (`getent`, `id`), so LDAP / SSSD users work too.
pub async fn lookup(name: &str) -> Result<Target> {
    if !valid_user_name(name) {
        bail!("invalid user name {name:?}");
    }
    let pw = exec::run(&Cmd::new("getent", &["passwd", name])).await?;
    let line = pw.stdout.lines().next().filter(|_| pw.ok()).ok_or_else(|| anyhow!("unknown local user {name}"))?;
    let f: Vec<&str> = line.split(':').collect();
    if f.len() < 7 {
        bail!("unreadable account entry for {name}");
    }
    let uid = f[2].parse().map_err(|_| anyhow!("bad uid for {name}"))?;
    let gid = f[3].parse().map_err(|_| anyhow!("bad gid for {name}"))?;
    let ids = exec::run(&Cmd::new("id", &["-G", name])).await.ok().filter(|o| o.ok()).map(|o| o.stdout).unwrap_or_default();
    let mut groups: Vec<u32> = ids.split_whitespace().filter_map(|g| g.parse().ok()).collect();
    if !groups.contains(&gid) {
        groups.push(gid);
    }
    Ok(Target { name: name.to_string(), uid, gid, groups, home: f[5].to_string() })
}

/// Do the permission bits let this user use `want` (r=4, w=2, x=1) on a file with this owner / group / mode?
pub fn mode_allows(mode: u32, owner: u32, group: u32, t: &Target, want: u32) -> bool {
    if t.uid == 0 {
        return true;
    }
    let bits = if owner == t.uid {
        (mode >> 6) & 7
    } else if t.groups.contains(&group) {
        (mode >> 3) & 7
    } else {
        mode & 7
    };
    bits & want == want
}

/// Can the user reach and use this path? Every directory on the way must be searchable; a directory must be
/// readable and searchable, an executable file executable, any other file readable. (ACLs are not considered.)
pub fn usable_by(path: &Path, t: &Target) -> std::result::Result<(), String> {
    let real = std::fs::canonicalize(path).map_err(|_| format!("{} does not exist", path.display()))?;
    let mut dir = PathBuf::from("/");
    for comp in real.parent().map(|p| p.components().skip(1).collect::<Vec<_>>()).unwrap_or_default() {
        dir.push(comp);
        let md = std::fs::metadata(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if !mode_allows(md.mode(), md.uid(), md.gid(), t, 1) {
            return Err(format!("{} cannot open the directory {} (mode {:o})", t.name, dir.display(), md.mode() & 0o7777));
        }
    }
    let md = std::fs::metadata(&real).map_err(|e| format!("{}: {e}", real.display()))?;
    let (want, what) = if md.is_dir() {
        (5, "read")
    } else if md.mode() & 0o111 != 0 {
        (1, "run")
    } else {
        (4, "read")
    };
    if mode_allows(md.mode(), md.uid(), md.gid(), t, want) {
        Ok(())
    } else {
        Err(format!("{} cannot {what} {} (mode {:o}, owner uid {})", t.name, real.display(), md.mode() & 0o7777, md.uid()))
    }
}

/// The shell invocation for a recipe step.
pub fn command(script: &str, t: &Target) -> Cmd {
    let mut c = Cmd::new("sh", &["-c", &format!("umask 022\n{script}")]).timeout(TIMEOUT_SECS).env("HOME", "/root").env("DEBIAN_FRONTEND", "noninteractive").cwd("/");
    if t.uid != Target::everyone().uid {
        c = c.env("CROUSTYPATCH_USER", &t.name).env("CROUSTYPATCH_USER_HOME", &t.home).env("CROUSTYPATCH_USER_UID", &t.uid.to_string()).env("CROUSTYPATCH_USER_GID", &t.gid.to_string());
    }
    c
}

/// Run the install or uninstall step and verify its result. Returns a one-line summary.
pub async fn run(runner: &exec::Runner, cmd: &AppCommand, install: bool, t: &Target, log: &TaskLog) -> Result<String> {
    let script = if install { &cmd.install } else { &cmd.uninstall };
    let check = Path::new(&cmd.check);
    let out = runner.mutate(command(script, t), log).await?;
    if !out.ok() {
        let why = out.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no error output");
        bail!("the {} command failed (exit {}): {why}", if install { "install" } else { "uninstall" }, out.code);
    }
    if runner.dry_run {
        return Ok(format!("[dry-run] {} command not executed", if install { "install" } else { "uninstall" }));
    }
    if install {
        usable_by(check, t).map_err(|e| anyhow!("the install command succeeded, but the application is not usable: {e}"))?;
        Ok(format!("installed; {} is usable by {}", cmd.check, t.name))
    } else if check.exists() {
        bail!("the uninstall command succeeded, but {} is still there", cmd.check)
    } else {
        Ok("uninstalled".to_string())
    }
}
