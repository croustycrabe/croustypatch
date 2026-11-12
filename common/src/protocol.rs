//! Agent <-> server wire protocol.
//!
//! * Enrolment is a plain REST call authenticated by a one-time token.
//! * Afterwards every request carries an RS256 JWT signed with the agent's own key.
//! * Every command the server sends is signed with the server key; the agent verifies it,
//!   its validity window and its nonce before doing anything (see `SignedCommand`).
use crate::model::{HealthProbe, PackageManager, TaskStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostFacts {
    pub hostname: String,
    /// "linux" or "freebsd".
    pub os_family: String,
    pub distro: String,
    pub os_version: String,
    pub kernel: String,
    pub arch: String,
    pub package_manager: Option<PackageManager>,
    pub is_laptop: bool,
    pub on_battery: bool,
    /// A graphical desktop is installed (display manager or desktop sessions): a workstation unless it is a laptop.
    #[serde(default)]
    pub is_desktop: bool,
    /// The desktop app center (`croustypatch-portal-gui`) is installed.
    #[serde(default)]
    pub portal_gui: bool,
    pub virtualization: Option<String>,
    pub ip_addresses: Vec<String>,
    pub agent_version: String,
    /// Optional modules loaded in the agent (docker, openscap, ...).
    #[serde(default)]
    pub modules: Vec<String>,
    /// Protocol features this agent understands (see `capability`); empty on agents before 0.4.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// What the machine is made of (empty on agents before 0.6).
    #[serde(default)]
    pub hardware: Hardware,
    /// OpenSCAP as installed on the machine: version, the content found and the profiles it offers (agents 0.7+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openscap: Option<OpenScapInfo>,
}

/// OpenSCAP on a host: what an administrator can choose from when picking its audit profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenScapInfo {
    pub version: String,
    /// The SCAP Security Guide datastream used (empty when no content is installed).
    pub datastream: String,
    pub profiles: Vec<ScapProfile>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScapProfile {
    pub id: String,
    pub title: String,
}

/// A custom audit profile: a profile of the datastream with some rules switched off (tailoring).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tailoring {
    pub id: String,
    pub name: String,
    /// The datastream profile it extends.
    pub base: String,
    pub disabled_rules: Vec<String>,
}

/// Static description of the machine: CPU, memory and what the firmware says it is. No serial numbers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hardware {
    pub cpu_model: String,
    /// Logical processors.
    pub cpu_cores: u32,
    pub mem_total_mb: u64,
    pub swap_total_mb: u64,
    /// Manufacturer and model as the firmware reports them (a laptop, a server, "QEMU Standard PC"...).
    pub vendor: String,
    pub product: String,
    pub bios: String,
}

/// How busy the machine is right now (sent with the heartbeat).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    /// CPU use since the previous sample, 0-100 (None on the first sample or where it cannot be measured).
    pub cpu_pct: Option<f32>,
    pub mem_total_mb: u64,
    pub mem_used_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    pub load_5m: f32,
    pub load_15m: f32,
    pub disks: Vec<DiskUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub mount: String,
    pub fs: String,
    pub total_mb: u64,
    pub used_mb: u64,
}

/// Feature flags an agent advertises in its facts, so the server never sends an order an older agent would misread.
pub mod capability {
    /// Understands `ServerMessage::Settings` / `TelemetryResponse::settings` (per-host modules).
    pub const SETTINGS: &str = "settings";
    /// Runs app-center command recipes (`CommandAction::App { command, .. }`).
    pub const APP_COMMAND: &str = "app_command";
    /// Replaces its own binary on `CommandAction::UpdateAgent`.
    pub const SELF_UPDATE: &str = "self_update";
    /// Changes the tag of a container's image in its compose file (`ApplyUpdates::retag`). An agent without it would
    /// ignore the field and pull the tag it already has, so the server never sends it one.
    pub const DOCKER_RETAG: &str = "docker_retag";
    pub const ALL: &[&str] = &[SETTINGS, APP_COMMAND, SELF_UPDATE, DOCKER_RETAG];
}

/// Optional features switched on per host by the server (Hosts > host > Modules).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostModules {
    pub docker: bool,
    pub helper_script: bool,
    pub openscap: bool,
    pub portal: bool,
    pub eco: bool,
    /// Users of this machine may start its pending system updates from the app center, without approval.
    pub portal_updates: bool,
}

impl HostModules {
    pub const NAMES: &[&str] = &["docker", "helper_script", "openscap", "portal", "portal_updates", "eco"];
    pub fn get(&self, name: &str) -> bool {
        match name {
            "docker" => self.docker,
            "helper_script" | "helper-script" => self.helper_script,
            "openscap" => self.openscap,
            "portal" => self.portal,
            "portal_updates" => self.portal_updates,
            "eco" => self.eco,
            _ => false,
        }
    }
}

/// Server-side settings pushed to an agent (on connection and whenever they change).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentSettings {
    pub modules: HostModules,
    #[serde(default)]
    pub isolation: IsolationSettings,
    /// Air-gapped installation (Settings > Connectivity): no lookup on the internet (container registries, the AUR).
    /// The agent then reports what is installed, and what its own package manager offers through local mirrors.
    #[serde(default)]
    pub offline: bool,
}

/// What stays reachable while a host is isolated (double failure or manual quarantine), besides the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IsolationSettings {
    /// Keep inbound SSH open, so an administrator can still log in and repair the host.
    pub keep_ssh: bool,
    pub ssh_port: u16,
    /// Networks allowed to reach SSH (CIDR or address); empty = any source.
    pub ssh_sources: Vec<String>,
}

impl Default for IsolationSettings {
    fn default() -> Self {
        Self { keep_ssh: true, ssh_port: 22, ssh_sources: vec![] }
    }
}

/// `192.0.2.0/24`, `2001:db8::/32` or a single address, normalised; None when malformed (never reaches a firewall).
pub fn parse_cidr(s: &str) -> Option<(std::net::IpAddr, u8)> {
    let (ip, len) = match s.trim().split_once('/') {
        Some((ip, len)) => (ip.parse::<std::net::IpAddr>().ok()?, len.parse::<u8>().ok()?),
        None => {
            let ip = s.trim().parse::<std::net::IpAddr>().ok()?;
            (ip, if ip.is_ipv4() { 32 } else { 128 })
        }
    };
    (len <= if ip.is_ipv4() { 32 } else { 128 }).then_some((ip, len))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    /// Newer version available, if any.
    pub candidate: Option<String>,
    #[serde(default)]
    pub is_security: bool,
    #[serde(default)]
    pub cves: Vec<String>,
    /// "system" for OS packages; later "docker", "helper-script".
    #[serde(default = "system")]
    pub source: String,
    /// Extra facts about the entry (containers: image, stack, service, state, health, start time).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub meta: serde_json::Value,
}

fn system() -> String {
    "system".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub facts: HostFacts,
    pub packages: Vec<PackageEntry>,
    pub reboot_required: bool,
    pub collected_at: DateTime<Utc>,
}

// ------------------------------------------------------------------ enrolment

#[derive(Debug, Serialize, Deserialize)]
pub struct EnrollRequest {
    pub token: String,
    pub facts: HostFacts,
    /// PEM (SPKI) public key of the agent's freshly generated RSA keypair.
    pub public_key_pem: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EnrollResponse {
    pub agent_id: Uuid,
    /// The agent pins this key to verify every command signature.
    pub server_public_key_pem: String,
    pub heartbeat_secs: u64,
}

// --------------------------------------------------------------- agent -> server

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Hello { facts: HostFacts, protocol: u32 },
    Heartbeat(Heartbeat),
    Inventory(Inventory),
    TaskUpdate(TaskUpdate),
    /// Result of an OpenSCAP evaluation, sent before the task's final update.
    Compliance { task_id: Uuid, report: ComplianceReport },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailedRule {
    pub rule_id: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub profile: String,
    pub datastream: String,
    pub passed: u32,
    pub failed: u32,
    pub not_applicable: u32,
    /// Errors, unknown, not checked…
    pub other: u32,
    /// passed / (passed + failed), in percent.
    pub score: f64,
    pub failed_rules: Vec<FailedRule>,
    /// SHA-256 of the raw XCCDF result file kept on the host.
    pub results_sha256: String,
    pub collected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub on_battery: bool,
    /// The agent switched to the economy profile.
    pub eco: bool,
    pub load_1m: f32,
    pub uptime_secs: u64,
    /// Tasks the agent currently runs; lets the server detect lost work.
    #[serde(default)]
    pub running_tasks: Vec<Uuid>,
    /// Resource use (agents from 0.6).
    #[serde(default)]
    pub metrics: Option<Metrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskUpdate {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub message: String,
    #[serde(default)]
    pub log: Vec<String>,
    #[serde(default)]
    pub changes: Vec<PackageChange>,
    #[serde(default)]
    pub reboot_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageChange {
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

// --------------------------------------------------------------- server -> agent

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome { heartbeat_secs: u64 },
    /// Per-host settings; only sent to agents advertising `capability::SETTINGS`.
    Settings(AgentSettings),
    Command(SignedCommand),
    /// Ask for a fresh inventory right away.
    Ping,
}

/// Server-signed command envelope. `payload` is the exact JSON text that was signed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedCommand {
    pub payload: String,
    /// base64 RSASSA-PKCS1-v1_5/SHA-256 over `payload` bytes.
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPayload {
    pub task_id: Uuid,
    /// Binds the command to one agent so it cannot be replayed on another host.
    pub agent_id: Uuid,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub nonce: String,
    pub action: CommandAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandAction {
    Scan,
    ApplyUpdates {
        packages: Vec<String>,
        #[serde(default)]
        exec: ExecPolicy,
        /// `docker:<container>` -> the image reference to move it to (the same image with another tag). The agent
        /// changes the tag in the service's compose file, recreates it, and puts the file back if that fails.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        retag: std::collections::BTreeMap<String, String>,
    },
    /// Revert the packages changed by a previous patch task.
    Rollback { target_task: Uuid },
    Quarantine,
    Release,
    /// Restart the host. Users are warned `delay_minutes` ahead by the system's shutdown broadcast; the task completes
    /// when the agent reports again from the new boot.
    Reboot {
        #[serde(default)]
        delay_minutes: u32,
        #[serde(default)]
        message: String,
    },
    /// OpenSCAP evaluation with a profile id (empty: the SSG "standard" profile), or with a tailored (custom) profile.
    /// When OpenSCAP or its content is missing and `auto_install` is on, the agent installs them first.
    ComplianceScan {
        #[serde(default)]
        profile: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tailoring: Option<Tailoring>,
        #[serde(default = "yes")]
        auto_install: bool,
    },
    /// App center: install or remove a catalog application, from its packages or (agents with
    /// `capability::APP_COMMAND`) from an admin-written command recipe.
    App {
        install: bool,
        packages: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<AppCommand>,
        /// Local account that asked (TUI), so the result can be checked against its rights; None from the web UI.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
    },
    /// Replace the agent binary with the server's build for this platform, then restart. When the desktop app center
    /// is installed and the server has a build of it, `gui_sha256` refreshes it in the same operation.
    UpdateAgent {
        platform: String,
        sha256: String,
        version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gui_sha256: Option<String>,
    },
}

/// Command recipe of an app-center application (run as root with `sh -c`, umask 022).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCommand {
    pub install: String,
    pub uninstall: String,
    /// Absolute path that exists once the app is installed. It must be usable by the user who asked
    /// (or by every user when the request came from the web UI).
    pub check: String,
    /// Where to find out that a newer version exists. Software installed by a command is not in the package manager's
    /// update list, so without this the app center cannot say that an update is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<LatestVersion>,
}

/// An AUR package name as the AUR spells it (what may end up in a request URL and a pacman command).
pub fn valid_aur_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 100 && !n.starts_with(['-', '.']) && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '@' | '.' | '_' | '+' | '-'))
}

/// Where the latest version of a command-installed application is published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatestVersion {
    /// An AUR package (Arch): the installed version is what pacman says, the latest is what the AUR says.
    Aur(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecPolicy {
    /// Refuse to patch when no restore point could be made.
    #[serde(default = "yes")]
    pub snapshot_required: bool,
    #[serde(default)]
    pub soak_minutes: u32,
    #[serde(default)]
    pub probes: Vec<HealthProbe>,
    /// Roll back automatically when a probe fails during the soak window.
    #[serde(default = "yes")]
    pub auto_rollback: bool,
}

fn yes() -> bool {
    true
}

impl Default for ExecPolicy {
    fn default() -> Self {
        Self { snapshot_required: true, soak_minutes: 0, probes: vec![], auto_rollback: true }
    }
}

// ------------------------------------------------------------------- push mode

/// Push-mode agents POST this to `/agent/telemetry`; the reply carries pending commands.
#[derive(Debug, Serialize, Deserialize)]
pub struct TelemetryRequest {
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TelemetryResponse {
    pub commands: Vec<SignedCommand>,
    pub heartbeat_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<AgentSettings>,
}

/// Build a signed command envelope.
pub fn sign_command(server_private_pem: &str, payload: &CommandPayload) -> Result<SignedCommand, crate::crypto::CryptoError> {
    let text = serde_json::to_string(payload).map_err(|e| crate::crypto::CryptoError::Encoding(e.to_string()))?;
    let signature = crate::crypto::rsa_sign_b64(server_private_pem, text.as_bytes())?;
    Ok(SignedCommand { payload: text, signature })
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("signature verification failed")]
    BadSignature,
    #[error("malformed command payload")]
    Malformed,
    #[error("command is addressed to another agent")]
    WrongAgent,
    #[error("command expired or not yet valid")]
    Expired,
}

/// Verify signature, addressee and validity window. Nonce replay checks are the caller's job.
pub fn verify_command(
    server_public_pem: &str,
    cmd: &SignedCommand,
    my_agent_id: Uuid,
    now: DateTime<Utc>,
) -> Result<CommandPayload, CommandError> {
    crate::crypto::rsa_verify_b64(server_public_pem, cmd.payload.as_bytes(), &cmd.signature).map_err(|_| CommandError::BadSignature)?;
    let p: CommandPayload = serde_json::from_str(&cmd.payload).map_err(|_| CommandError::Malformed)?;
    if p.agent_id != my_agent_id {
        return Err(CommandError::WrongAgent);
    }
    if now > p.expires_at || now + chrono::Duration::seconds(60) < p.issued_at {
        return Err(CommandError::Expired);
    }
    Ok(p)
}
