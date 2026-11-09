use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "croustypatch-server", version, about = "CroustyPatch management server")]
pub struct Cli {
    #[command(flatten)]
    pub cfg: Config,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the server (default).
    Serve,
    /// Apply pending database migrations and exit.
    Migrate,
    /// Administrative operations on local accounts.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum AdminAction {
    /// Disable or re-enable the bootstrap fallback super-admin.
    Disable {
        #[arg(long)]
        super_admin_fallback: bool,
    },
    Enable {
        #[arg(long)]
        super_admin_fallback: bool,
    },
    /// Set a new password for a local user (prompted from the CROUSTYPATCH_NEW_PASSWORD variable).
    ResetPassword { username: String },
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum TlsMode {
    /// Serve HTTPS (TLS 1.3 only) ourselves.
    Direct,
    /// Plain HTTP behind a reverse proxy that terminates TLS.
    Proxy,
}

#[derive(Args, Debug, Clone)]
pub struct Config {
    #[arg(long, env = "DATABASE_URL", default_value = "postgres://croustypatch:croustypatch-dev@127.0.0.1:5433/croustypatch")]
    pub database_url: String,

    #[arg(long, env = "CROUSTYPATCH_LISTEN", default_value = "127.0.0.1:8443")]
    pub listen: String,

    #[arg(long, env = "CROUSTYPATCH_DATA_DIR", default_value = "./data")]
    pub data_dir: PathBuf,

    /// Base64 32-byte master key. If unset, `<data_dir>/master.key` is used (created at first start).
    #[arg(long, env = "CROUSTYPATCH_MASTER_KEY", hide_env_values = true)]
    pub master_key: Option<String>,

    #[arg(long, env = "CROUSTYPATCH_TLS_MODE", value_enum, default_value = "direct")]
    pub tls_mode: TlsMode,

    #[arg(long, env = "CROUSTYPATCH_TLS_CERT")]
    pub tls_cert: Option<PathBuf>,
    #[arg(long, env = "CROUSTYPATCH_TLS_KEY")]
    pub tls_key: Option<PathBuf>,

    /// Extra DNS names / IPs for the auto-generated self-signed certificate.
    #[arg(long, env = "CROUSTYPATCH_TLS_SAN", value_delimiter = ',', default_value = "localhost,127.0.0.1")]
    pub tls_san: Vec<String>,

    /// In proxy mode, honour X-Forwarded-* only from these peer IPs.
    #[arg(long, env = "CROUSTYPATCH_TRUSTED_PROXIES", value_delimiter = ',', default_value = "127.0.0.1,::1")]
    pub trusted_proxies: Vec<String>,

    /// Directory containing the exported web UI (Next.js `out/`). Served at `/` when present.
    #[arg(long, env = "CROUSTYPATCH_STATIC_DIR", default_value = "../frontend/out")]
    pub static_dir: PathBuf,

    /// Directories (comma separated, searched in order) holding `croustypatch-agent-<platform>` binaries that the
    /// "Add agent" installer hands to new hosts.
    #[arg(long, env = "CROUSTYPATCH_AGENT_DIR", value_delimiter = ',', default_value = "./data/agents")]
    pub agent_dir: Vec<PathBuf>,

    /// Allowed browser origin for the UI during development (e.g. http://localhost:3000).
    #[arg(long, env = "CROUSTYPATCH_CORS_ORIGIN")]
    pub cors_origin: Option<String>,
}
