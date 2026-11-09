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
    pub virtualization: Option<String>,
    pub ip_addresses: Vec<String>,
    pub agent_version: String,
    /// Optional modules loaded in the agent (docker, openscap, ...).
    #[serde(default)]
    pub modules: Vec<String>,
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
    /// OpenSCAP evaluation with a profile id (empty: the SSG "standard" profile).
    ComplianceScan {
        #[serde(default)]
        profile: String,
    },
    /// App center: install or remove the packages of a catalog application.
    App { install: bool, packages: Vec<String> },
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
