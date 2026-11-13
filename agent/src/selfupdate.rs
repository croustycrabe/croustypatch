//! Agent self-update (Hosts > host > Update agent).
//!
//! The signed order names the platform build and its SHA-256. The agent downloads it from its own server, checks
//! the hash, makes sure the new binary starts and reports the expected version, swaps it in place atomically and
//! exits with `EXIT_RESTART`; the service manager (systemd `Restart=always` or `on-failure`)
//! starts the new one, which closes the task. A failure at any step leaves the running binary untouched.
use crate::config::{write_private, Config};
use crate::net;
use crate::runner::{Agent, TaskLog};
use anyhow::{anyhow, bail, Context, Result};
use croustypatch_common::crypto::sha256_hex;
use croustypatch_common::model::TaskStatus;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

const PLATFORMS: &[&str] = &["linux-x86_64", "linux-aarch64"];
const MAX_SIZE: usize = 200 * 1024 * 1024;
/// Non-zero on purpose (EX_TEMPFAIL): restarted by `Restart=always` and by `Restart=on-failure` units alike.
pub const EXIT_RESTART: i32 = 75;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingUpdate {
    pub task_id: Uuid,
    pub from: String,
    pub to: String,
}

fn pending_file(cfg: &Config) -> std::path::PathBuf {
    cfg.state_dir.join("pending_update.json")
}

pub fn check_order(platform: &str, sha256: &str, version: &str) -> Result<()> {
    if !PLATFORMS.contains(&platform) {
        bail!("unknown platform {platform:?}");
    }
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("malformed SHA-256");
    }
    if version.is_empty() || version.len() > 40 || !version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')) {
        bail!("malformed version");
    }
    Ok(())
}

/// Download one build from this agent's server and check its SHA-256.
async fn fetch(agent: &Agent, id: &str, sha256: &str) -> Result<Vec<u8>> {
    let client = net::http_client_pinned(&agent.cfg, &agent.server_ips())?;
    let resp = client.get(net::api_url(&agent.cfg, &format!("/agent/download/{id}"))).timeout(Duration::from_secs(600)).send().await.with_context(|| format!("downloading {id}"))?;
    if !resp.status().is_success() {
        bail!("download of {id} refused: {}", resp.status());
    }
    let bytes = resp.bytes().await.with_context(|| format!("downloading {id}"))?;
    if bytes.len() > MAX_SIZE {
        bail!("{id}: download too large ({} bytes)", bytes.len());
    }
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(sha256) {
        bail!("{id}: checksum mismatch (expected {sha256}, got {got}): nothing was changed");
    }
    Ok(bytes.to_vec())
}

/// Write an executable next to its final place (same directory, so the later rename is atomic).
fn stage(target: &std::path::Path, id: Uuid, bytes: &[u8]) -> Result<std::path::PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = target.with_file_name(format!(".{name}.{id}.new"));
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o755).open(&staged).with_context(|| format!("writing {}", staged.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(staged)
}

/// Login entry of the update notifier (the installer writes the same file: keep the two in step).
pub const NOTIFIER_ENTRY: &str = "/etc/xdg/autostart/croustypatch-update-notifier.desktop";
const NOTIFIER_DESKTOP: &str = "[Desktop Entry]\nType=Application\nName=CroustyPatch update notifier\nName[fr]=CroustyPatch – notification des mises à jour\nComment=Tells you when updates can be installed\nExec=/usr/local/bin/croustypatch-portal-gui --notifier\nIcon=croustypatch-portal\nNoDisplay=true\nX-GNOME-Autostart-Delay=20\n";

/// Laptops enrolled before 0.7 have the desktop app center but no notifier: only the installer adds its login entry,
/// and *Update agent* does not run the installer. Written after the desktop binary is replaced (an older binary would
/// open its window for `--notifier`), and never over an entry that is already there. Returns whether it was written.
pub fn ensure_notifier_entry(path: &std::path::Path) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, NOTIFIER_DESKTOP).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

pub async fn start(agent: &Agent, id: Uuid, platform: &str, sha256: &str, version: &str, gui_sha256: Option<&str>, log: &TaskLog) -> Result<()> {
    check_order(platform, sha256, version)?;
    if let Some(g) = gui_sha256 {
        check_order(platform, g, version)?;
    }
    let current = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe().context("locating the running agent")?;
    let gui_path = std::path::Path::new(crate::facts::PORTAL_GUI);
    let gui = gui_sha256.filter(|_| gui_path.is_file());
    if agent.runner.dry_run {
        log.line(format!("[dry-run] would replace {} ({current}) with {version}{}", exe.display(), if gui.is_some() { " and the desktop app center" } else { "" }));
        agent.update(id, TaskStatus::Succeeded, format!("[dry-run] agent {current} would be updated to {version}"), log, vec![], false);
        return Ok(());
    }
    agent.update(id, TaskStatus::Running, format!("downloading agent {version} for {platform}"), log, vec![], false);
    // Everything is downloaded and verified before anything is replaced.
    let bytes = fetch(agent, platform, sha256).await?;
    log.line(format!("agent: {} bytes, SHA-256 verified", bytes.len()));
    let gui_bytes = match gui {
        Some(g) => {
            let b = fetch(agent, &format!("portal-gui-{platform}"), g).await?;
            log.line(format!("desktop app center: {} bytes, SHA-256 verified", b.len()));
            Some(b)
        }
        None => None,
    };

    let staged = stage(&exe, id, &bytes)?;
    // The new binary must start on this machine and be the announced version (wrong architecture, truncated file…).
    let probe = tokio::time::timeout(Duration::from_secs(20), tokio::process::Command::new(&staged).arg("--version").env_clear().output()).await;
    let ok = match &probe {
        Ok(Ok(o)) if o.status.success() => String::from_utf8_lossy(&o.stdout).split_whitespace().any(|w| w == version),
        _ => false,
    };
    if !ok {
        let _ = std::fs::remove_file(&staged);
        let detail = match probe {
            Ok(Ok(o)) => format!("it printed {:?}", String::from_utf8_lossy(&o.stdout).trim()),
            Ok(Err(e)) => e.to_string(),
            Err(_) => "it did not answer".into(),
        };
        bail!("the downloaded agent does not run as version {version} on this host ({detail}): nothing was changed");
    }
    let gui_staged = match &gui_bytes {
        Some(b) => match stage(gui_path, id, b) {
            Ok(p) => Some(p),
            Err(e) => {
                let _ = std::fs::remove_file(&staged);
                return Err(e);
            }
        },
        None => None,
    };
    if let Some(g) = &gui_staged {
        std::fs::rename(g, gui_path).map_err(|e| {
            let _ = std::fs::remove_file(g);
            let _ = std::fs::remove_file(&staged);
            anyhow!("replacing {}: {e}", gui_path.display())
        })?;
        log.line(format!("{} replaced", gui_path.display()));
        match ensure_notifier_entry(std::path::Path::new(NOTIFIER_ENTRY)) {
            Ok(true) => log.line("update notifier: login entry added (desktop notifications about updates start at the next login)".to_string()),
            Ok(false) => {}
            Err(e) => log.line(format!("update notifier: could not add its login entry: {e:#}")),
        }
    }
    std::fs::rename(&staged, &exe).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        anyhow!("replacing {}: {e}", exe.display())
    })?;
    write_private(&pending_file(&agent.cfg), &serde_json::to_vec(&PendingUpdate { task_id: id, from: current.into(), to: version.into() })?)?;
    log.line(format!("{} replaced; restarting", exe.display()));
    agent.update(id, TaskStatus::Running, format!("agent {version} installed{}; restarting", if gui_staged.is_some() { " with the desktop app center" } else { "" }), log, vec![], false);
    // Give the link a moment to deliver the update, then let the service manager start the new binary.
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        tracing::info!("exiting to restart on the updated binary");
        std::process::exit(EXIT_RESTART);
    });
    Ok(())
}

/// After a restart: close the update task that caused it.
pub fn resume(agent: &Agent) {
    let path = pending_file(&agent.cfg);
    let Some(p) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<PendingUpdate>(&b).ok()) else { return };
    let _ = std::fs::remove_file(&path);
    let now = env!("CARGO_PKG_VERSION");
    let log = TaskLog::default();
    if now == p.to {
        agent.update(p.task_id, TaskStatus::Succeeded, format!("agent updated from {} to {now}", p.from), &log, vec![], false);
    } else {
        agent.update(p.task_id, TaskStatus::Failed, format!("the agent restarted as {now}, expected {}", p.to), &log, vec![], false);
    }
}
