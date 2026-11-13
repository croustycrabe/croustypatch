
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use croustypatch_agent::config::Config;
use croustypatch_agent::identity::Identity;
use croustypatch_agent::{exec, facts, launcher, link, net, pkg, runner};
use croustypatch_common::crypto;
use croustypatch_common::protocol::{EnrollRequest, EnrollResponse};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "croustypatch-agent", version, about = "CroustyPatch agent (Linux / FreeBSD)")]
struct Cli {
    #[arg(long, global = true, env = "CROUSTYPATCH_AGENT_CONFIG", default_value = "/etc/croustypatch/agent.toml")]
    config: PathBuf,
    #[arg(long, global = true, env = "CROUSTYPATCH_STATE_DIR")]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Register this host with a server using a one-time enrolment token.
    Enroll {
        #[arg(long)]
        server: String,
        /// Prefer the environment variable: command-line arguments are visible to other local users.
        #[arg(long, env = "CROUSTYPATCH_ENROLL_TOKEN", hide_env_values = true)]
        token: String,
        #[arg(long, default_value = "pull")]
        mode: String,
        /// PEM certificate/CA to trust for the server (self-signed or private PKI).
        #[arg(long)]
        ca_file: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// Run the agent.
    Run {
        /// Never modify the system; log what would be done.
        #[arg(long)]
        dry_run: bool,
    },
    /// Print this host's package inventory as JSON and exit (no server needed).
    Scan,
    /// Rehearse the safety checks of a patch run without changing the system: refresh the repositories, then build a
    /// restore point for every pending update in a scratch directory and report what could not be restored.
    Doctor,
    /// The app center for any local user (also installed as `croustypatch-portal`): the desktop window when a
    /// graphical session is available and it is installed, the terminal one otherwise.
    Portal {
        /// The agent's app center socket.
        #[arg(long, env = "CROUSTYPATCH_PORTAL_SOCKET", default_value = "/run/croustypatch/portal.sock")]
        socket: String,
        /// Always open the desktop window (falls back to the terminal if it is not installed).
        #[arg(long, conflicts_with = "tui")]
        gui: bool,
        /// Always open the terminal app center.
        #[arg(long)]
        tui: bool,
    },
    /// Print this machine's type as detected by the agent: laptop, workstation or server.
    MachineType,
}

fn load_config(cli: &Cli) -> Result<Config> {
    let mut c = Config::load(&cli.config)?;
    if let Some(d) = &cli.state_dir {
        c.state_dir = d.clone();
    }
    Ok(c)
}

fn backend_for(cfg: &Config) -> Result<pkg::Backend> {
    let kind = if cfg.package_manager == "mock" { croustypatch_common::model::PackageManager::Unknown } else { facts::detect_package_manager(&cfg.package_manager) };
    pkg::Backend::select(kind, &cfg.mock_file)
}

async fn enroll(cli: &Cli, server: String, token: String, mode: String, ca_file: Option<String>, force: bool) -> Result<()> {
    let mut cfg = load_config(cli)?;
    cfg.server_url = server.trim_end_matches('/').to_string();
    cfg.mode = mode;
    if let Some(c) = ca_file {
        cfg.tls_ca_file = c;
    }
    cfg.validate()?;
    if Identity::exists(&cfg) && !force {
        bail!("this host is already enrolled (use --force to enrol again)");
    }
    std::fs::create_dir_all(&cfg.state_dir)?;
    println!("generating RSA-2048 identity key…");
    let kp = crypto::generate_rsa(2048)?;
    let pm = facts::detect_package_manager(&cfg.package_manager);
    let facts = facts::collect(&cfg.server_url, pm, vec![]).await;
    let resp = net::http_client(&cfg)?
        .post(net::api_url(&cfg, "/agent/enroll"))
        .json(&EnrollRequest { token, facts, public_key_pem: kp.public_pem.clone() })
        .send()
        .await
        .context("contacting the server")?;
    if !resp.status().is_success() {
        bail!("enrolment refused ({}): token invalid, expired, revoked or exhausted", resp.status());
    }
    let r: EnrollResponse = resp.json().await.context("reading enrolment response")?;
    Identity { agent_id: r.agent_id, server_public_key_pem: r.server_public_key_pem, private_key_pem: kp.private_pem }.save(&cfg)?;
    cfg.heartbeat_secs = r.heartbeat_secs;
    cfg.save(&cli.config)?;
    println!("enrolled as {} — config written to {}", r.agent_id, cli.config.display());
    Ok(())
}

async fn run(cli: &Cli, dry_run: bool) -> Result<()> {
    let mut cfg = load_config(cli)?;
    cfg.dry_run |= dry_run;
    cfg.validate()?;
    let identity = Identity::load(&cfg)?;
    let backend = backend_for(&cfg)?;
    #[cfg(unix)]
    if !cfg.dry_run && unsafe { libc_geteuid() } != 0 {
        tracing::warn!("not running as root: package operations will fail (use --dry-run to rehearse)");
    }
    if cfg.dry_run {
        tracing::warn!("DRY-RUN: the system will not be modified");
    }
    std::fs::create_dir_all(&cfg.state_dir)?;
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let agent = Arc::new(runner::Agent { runner: exec::Runner { dry_run: cfg.dry_run }, cfg: cfg.clone(), identity, backend, out: out_tx, server_ips: Mutex::new(net::load_server_ips(&cfg)), running: Mutex::new(Default::default()), settings: Mutex::new(runner::load_settings(&cfg)) });
    tracing::info!(agent = %agent.identity.agent_id, mode = %cfg.mode, server = %cfg.server_url, "agent starting");
    croustypatch_agent::reboot::resume(&agent);
    croustypatch_agent::selfupdate::resume(&agent);
    tokio::spawn(runner::worker(agent.clone(), cmd_rx));
    let link = Arc::new(link::Link::new(agent.clone(), cmd_tx));
    {
        let a = agent.clone();
        let relay = std::sync::Arc::new(croustypatch_agent::portal::Relay { agent: Some(agent.clone()), cfg: cfg.clone(), identity: agent.identity.clone(), server_ips: std::sync::Arc::new(move || a.server_ips()) });
        tokio::spawn(async move {
            if let Err(e) = croustypatch_agent::portal::serve(relay).await {
                tracing::warn!("app center socket unavailable: {e:#}");
            }
        });
    }
    let task = if cfg.mode == "push" { tokio::spawn(link::run_push(link, out_rx)) } else { tokio::spawn(link::run_pull(link, out_rx)) };
    wait_for_shutdown().await;
    tracing::info!("shutting down");
    task.abort();
    Ok(())
}

#[cfg(unix)]
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

async fn scan(cli: &Cli) -> Result<()> {
    let cfg = load_config(cli)?;
    let backend = backend_for(&cfg)?;
    let r = exec::Runner { dry_run: true };
    let mut packages = backend.inventory(&r).await?;
    if croustypatch_agent::modules::docker::available() {
        packages.extend(croustypatch_agent::modules::docker::inventory(&r, &Default::default()).await);
    }
    packages.extend(croustypatch_agent::modules::helper_scripts::detect(std::path::Path::new("/opt")));
    let facts = facts::collect(&cfg.server_url, backend.kind(), croustypatch_agent::modules::detected()).await;
    let reboot_required = backend.reboot_required(&r).await;
    println!("{}", serde_json::to_string_pretty(&croustypatch_common::protocol::Inventory { facts, packages, reboot_required, collected_at: chrono::Utc::now(), app_updates: vec![] })?);
    Ok(())
}

async fn doctor(cli: &Cli) -> Result<()> {
    // Every check below is about what root can do; as another user they only report the missing privileges.
    #[cfg(unix)]
    if unsafe { libc_geteuid() } != 0 {
        bail!("doctor checks what the agent can do as root: run it with sudo (sudo croustypatch-agent doctor)");
    }
    let cfg = load_config(cli)?;
    let backend = backend_for(&cfg)?;
    let r = exec::Runner { dry_run: false };
    let log = runner::TaskLog::default();
    println!("package manager: {:?}", backend.kind());
    let problems = croustypatch_agent::preflight::current_problems();
    for p in &problems {
        println!("NOT SAFE: {p}");
    }
    if problems.is_empty() {
        println!("environment: plain root, package scripts can run");
    }
    backend.refresh(&r, &log).await;
    let inv = backend.inventory(&r).await?;
    let names: Vec<String> = inv.iter().filter(|p| p.candidate.is_some()).map(|p| p.name.clone()).collect();
    println!("{} packages installed, {} with a pending update", inv.len(), names.len());
    let installed = pkg::installed_map(&inv);
    let scratch = cfg.state_dir.join(format!("doctor-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;
    let res = backend.snapshot(&r, uuid::Uuid::new_v4(), &names, &installed, &log, &scratch).await;
    let _ = std::fs::remove_dir_all(&scratch);
    for l in log.snapshot() {
        println!("  {l}");
    }
    match res {
        Ok(rp) if rp.unrestorable.is_empty() => println!("OK: all {} pending updates have a restore point", rp.pins.len()),
        Ok(rp) => println!("NOT SAFE: {} pinned, cannot restore: {}", rp.pins.len(), rp.unrestorable.join(", ")),
        Err(e) => println!("FAILED: {e}"),
    }
    Ok(())
}

async fn portal(socket: &str, force: launcher::Force) -> Result<()> {
    if launcher::decide(force) == launcher::Client::Gui {
        let e = launcher::exec_gui(socket);
        eprintln!("cannot open the desktop app center ({e}); opening the terminal one");
    } else if force == launcher::Force::Gui {
        eprintln!("the desktop app center is not installed on this machine; opening the terminal one");
    }
    croustypatch_agent::tui::run(socket).await
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).with_writer(std::io::stderr).init();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    // Installed as `croustypatch-portal` (a link to this binary): open the app center directly, no root needed.
    if std::env::args().next().is_some_and(|a| a.ends_with("croustypatch-portal")) {
        let socket = std::env::var("CROUSTYPATCH_PORTAL_SOCKET").unwrap_or_else(|_| "/run/croustypatch/portal.sock".into());
        return portal(&socket, launcher::force_from_args(std::env::args().skip(1))).await;
    }
    let cli = Cli::parse();
    match &cli.command {
        Cmd::Enroll { server, token, mode, ca_file, force } => enroll(&cli, server.clone(), token.clone(), mode.clone(), ca_file.clone(), *force).await,
        Cmd::Run { dry_run } => run(&cli, *dry_run).await,
        Cmd::Scan => scan(&cli).await,
        Cmd::Doctor => doctor(&cli).await,
        Cmd::Portal { socket, gui, tui } => portal(socket, if *gui { launcher::Force::Gui } else if *tui { launcher::Force::Tui } else { launcher::Force::Auto }).await,
        Cmd::MachineType => {
            let (is_laptop, _) = facts::power_state().await;
            println!("{}", facts::machine_type(is_laptop, facts::desktop_detected()));
            Ok(())
        }
    }
}
