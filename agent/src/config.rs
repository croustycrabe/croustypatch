use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_url: String,
    /// "pull" (persistent WebSocket tunnel) or "push" (periodic HTTPS telemetry).
    pub mode: String,
    /// PEM file pinned as the only trust anchor (self-signed or private CA). Empty = public roots.
    pub tls_ca_file: String,
    pub state_dir: PathBuf,
    pub heartbeat_secs: u64,
    /// Heartbeat interval used on battery (economy profile).
    pub eco_heartbeat_secs: u64,
    pub eco_enabled: bool,
    /// Seconds without any frame from the server (it pings every 30 s) before the tunnel is taken for dead and reopened.
    pub link_silence_secs: u64,
    /// Log what would be changed but never modify the system.
    pub dry_run: bool,
    /// auto | apt | pacman | mock
    pub package_manager: String,
    /// Test backend only: JSON file describing a fake package set.
    pub mock_file: String,
    pub inventory_interval_secs: u64,
    pub probe_interval_secs: u64,
    /// Unix socket of the local app center (TUI clients talk to it).
    pub portal_socket: String,
    /// Group allowed to use the app center socket; empty = every local user (the catalog is admin-approved).
    pub portal_group: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            mode: "pull".into(),
            tls_ca_file: String::new(),
            state_dir: PathBuf::from("/var/lib/croustypatch"),
            heartbeat_secs: 60,
            eco_heartbeat_secs: 900,
            eco_enabled: true,
            link_silence_secs: 90,
            dry_run: false,
            package_manager: "auto".into(),
            mock_file: String::new(),
            inventory_interval_secs: 3600,
            probe_interval_secs: 15,
            portal_socket: "/run/croustypatch/portal.sock".into(),
            portal_group: String::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        write_private(path, toml::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        let u = &self.server_url;
        if !(u.starts_with("https://") || (u.starts_with("http://") && is_loopback(u))) {
            anyhow::bail!("server_url must be https:// (plain http is only accepted for localhost during development)");
        }
        if !matches!(self.mode.as_str(), "pull" | "push") {
            anyhow::bail!("mode must be 'pull' or 'push'");
        }
        Ok(())
    }

    pub fn restore_dir(&self) -> PathBuf {
        self.state_dir.join("restore")
    }
    pub fn done_dir(&self) -> PathBuf {
        self.state_dir.join("done")
    }
    pub fn forensics_dir(&self) -> PathBuf {
        self.state_dir.join("forensics")
    }
}

fn is_loopback(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(data)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}
