//! Domain enums shared by server, agent and (via JSON) the web UI.
use serde::{Deserialize, Serialize};

macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $( #[serde(rename = $s)] $variant ),+ }
        impl $name {
            pub fn as_str(&self) -> &'static str { match self { $( Self::$variant => $s ),+ } }
            pub fn parse(s: &str) -> Option<Self> { match s { $( $s => Some(Self::$variant), )+ _ => None } }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
        }
    };
}

str_enum!(
    /// Lifecycle of a remediation task.
    TaskStatus {
        PendingApproval => "pending_approval",
        Queued => "queued",
        Sent => "sent",
        Running => "running",
        Soaking => "soaking",
        Succeeded => "succeeded",
        Failed => "failed",
        RollingBack => "rolling_back",
        RolledBack => "rolled_back",
        Quarantined => "quarantined",
        Blocked => "blocked",
        Rejected => "rejected",
        Cancelled => "cancelled",
        Expired => "expired",
    }
);

impl TaskStatus {
    /// No further transition is expected.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::RolledBack | Self::Quarantined | Self::Blocked | Self::Rejected | Self::Cancelled | Self::Expired
        )
    }
    /// The agent currently owns the task.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Sent | Self::Running | Self::Soaking | Self::RollingBack)
    }
}

str_enum!(
    TaskKind { Scan => "scan", Patch => "patch", Rollback => "rollback", Quarantine => "quarantine", Release => "release", Reboot => "reboot", Compliance => "compliance", App => "app", AgentUpdate => "agent_update" }
);

str_enum!(
    /// Contextual or stability risk level (ordered Low < Medium < High < Critical).
    RiskLevel { Low => "Low", Medium => "Medium", High => "High", Critical => "Critical" }
);

impl RiskLevel {
    pub fn rank(&self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }
    pub fn from_cvss(score: f64) -> Self {
        match score {
            s if s >= 9.0 => Self::Critical,
            s if s >= 7.0 => Self::High,
            s if s >= 4.0 => Self::Medium,
            _ => Self::Low,
        }
    }
}

str_enum!(
    /// Policy decision for a candidate update.
    PolicyAction { AutoDeploy => "AUTO_DEPLOY", RequireApproval => "REQUIRE_APPROVAL", Block => "BLOCK" }
);

str_enum!(
    HostStatus {
        Online => "online",
        Offline => "offline",
        Eco => "eco",
        Maintenance => "maintenance",
        Alert => "alert",
        Quarantined => "quarantined",
        Pending => "pending",
    }
);

str_enum!(
    PackageManager { Apt => "apt", Pacman => "pacman", Pkg => "pkg", Dnf => "dnf", Unknown => "unknown" }
);

/// Health probe the agent runs after a patch (soak window).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HealthProbe {
    Systemd { unit: String },
    Tcp { host: String, port: u16 },
    Http { url: String, #[serde(default = "default_status")] expect_status: u16 },
}

fn default_status() -> u16 {
    200
}

/// Package names are passed to package managers: only accept a conservative charset.
pub fn is_valid_package_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with(['-', '.'])
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '_' | '-' | ':' | '~' | '@'))
}

/// Keys of an app-center recipe map: one per package manager, or `*` for every system.
pub const RECIPE_KEYS: &[&str] = &["apt", "pacman", "pkg", "dnf", "*"];

/// The recipe for a host: its package manager's entry, else the `*` entry.
pub fn pick_recipe<'a>(recipes: &'a serde_json::Value, package_manager: &str) -> Option<&'a serde_json::Value> {
    recipes.get(package_manager).or_else(|| recipes.get("*")).filter(|v| v.is_object())
}

/// An absolute path without `..`, control characters or shell-hostile surprises (used for app checks).
pub fn is_valid_check_path(s: &str) -> bool {
    s.starts_with('/') && s.len() <= 255 && !s.split('/').any(|c| c == "..") && s.chars().all(|c| !c.is_control()) && !s.contains('\0')
}
