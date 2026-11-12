
use actix_cors::Cors;
use actix_web::http::header;
use actix_web::middleware::DefaultHeaders;
use actix_web::{web, App, HttpServer};
use anyhow::{Context, Result};
use clap::Parser;
use croustypatch_server::config::{AdminAction, Cli, Command, Config, TlsMode};
use croustypatch_server::crypto::MasterKey;
use croustypatch_server::state::{AppState, PgPool};
use croustypatch_server::{api, auth, db, scheduler, schema, tls};
use diesel::prelude::*;
use serde_json::json;
use std::sync::Mutex;
use zeroize::Zeroizing;

fn connect(cfg: &Config) -> Result<PgPool> {
    db::connect(cfg)
}

fn migrate(pool: &PgPool) -> Result<()> {
    let n = db::migrate(pool)?;
    if n > 0 {
        tracing::info!(count = n, "applied database migrations");
    }
    Ok(())
}

/// The server's RSA key signs every command sent to agents. Stored encrypted in the database.
fn load_or_create_server_key(pool: &PgPool, master: &MasterKey) -> Result<(Zeroizing<String>, String)> {
    use schema::settings;
    let mut conn = pool.get()?;
    let row: Option<serde_json::Value> = settings::table.find("server_key").select(settings::value).first(&mut conn).optional()?;
    if let Some(v) = row {
        let ct = v.get("ct").and_then(|x| x.as_str()).context("malformed server_key setting")?;
        let plain = master.decrypt_string(ct).map_err(|e| anyhow::anyhow!("cannot decrypt the server key (wrong master key?): {e}"))?;
        let j: serde_json::Value = serde_json::from_str(&plain)?;
        let (p, q) = (j["private_pem"].as_str().context("private_pem")?.to_string(), j["public_pem"].as_str().context("public_pem")?.to_string());
        return Ok((Zeroizing::new(p), q));
    }
    tracing::info!("generating the server signing key (RSA 3072)…");
    let kp = croustypatch_common::crypto::generate_rsa(3072)?;
    let blob = json!({ "private_pem": kp.private_pem, "public_pem": kp.public_pem }).to_string();
    diesel::insert_into(settings::table)
        .values((settings::key.eq("server_key"), settings::value.eq(json!({ "ct": master.encrypt(blob.as_bytes()) })), settings::encrypted.eq(true)))
        .on_conflict_do_nothing()
        .execute(&mut conn)?;
    // Another instance may have won the race; always read back what is stored.
    drop(conn);
    load_or_create_server_key(pool, master)
}

fn print_bootstrap_banner(password: Option<&str>) {
    const W: usize = 62;
    let row = |t: &str| eprintln!("  │{t:<W$}│");
    eprintln!("\n  ┌{}┐", "─".repeat(W));
    row(" First start: fallback super-admin created.");
    row(" username: admin");
    match password {
        Some(p) => {
            row(&format!(" password: {p}"));
            row(" Shown once; you must change it at first login.");
        }
        None => {
            row(" password: taken from CROUSTYPATCH_BOOTSTRAP_PASSWORD");
            row(" You must change it at first login.");
        }
    }
    row(" Disable it once SSO/admins are set up:");
    row("   croustypatch-server admin disable --super-admin-fallback");
    eprintln!("  └{}┘\n", "─".repeat(W));
}

fn run_admin(cfg: &Config, action: AdminAction) -> Result<()> {
    let pool = connect(cfg)?;
    migrate(&pool)?;
    let mut conn = pool.get()?;
    match action {
        AdminAction::Disable { super_admin_fallback } | AdminAction::Enable { super_admin_fallback } if !super_admin_fallback => {
            anyhow::bail!("specify --super-admin-fallback");
        }
        AdminAction::Disable { .. } => {
            let n = auth::set_fallback_disabled(&mut conn, true).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("fallback super-admin disabled ({n} account)");
        }
        AdminAction::Enable { .. } => {
            let n = auth::set_fallback_disabled(&mut conn, false).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("fallback super-admin enabled ({n} account)");
        }
        AdminAction::ResetPassword { username } => {
            let pw = std::env::var("CROUSTYPATCH_NEW_PASSWORD").context("set CROUSTYPATCH_NEW_PASSWORD (a password never goes on the command line)")?;
            auth::reset_password(&mut conn, &username, &pw).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("password updated for {username}; they must change it at next login");
        }
    }
    Ok(())
}

#[actix_web::main]
async fn main() -> Result<()> {
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,actix_server=warn".into());
        // standard output as before, plus a copy of the lines that matter for Administration > Logs
        tracing_subscriber::registry().with(filter).with(tracing_subscriber::fmt::layer()).with(croustypatch_server::logs::CaptureLayer).init();
    }
    // rustls needs a process-wide provider for the (TLS 1.3-only) server config.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cli = Cli::parse();
    let cfg = cli.cfg;
    match cli.command.unwrap_or(Command::Serve) {
        Command::Migrate => {
            migrate(&connect(&cfg)?)?;
            println!("migrations up to date");
            return Ok(());
        }
        Command::Admin { action } => return run_admin(&cfg, action),
        Command::Serve => {}
    }

    std::fs::create_dir_all(&cfg.data_dir)?;
    let master = MasterKey::load_or_create(cfg.master_key.as_deref(), &cfg.data_dir.join("master.key"))?;
    let pool = connect(&cfg)?;
    migrate(&pool)?;
    {
        let mut conn = pool.get()?;
        auth::ensure_roles(&mut conn).map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(b) = auth::ensure_fallback_admin(&mut conn, std::env::var("CROUSTYPATCH_BOOTSTRAP_PASSWORD").ok()).map_err(|e| anyhow::anyhow!("{e}"))? {
            print_bootstrap_banner(b.password.as_deref());
        }
    }
    let (server_private_pem, server_public_pem) = load_or_create_server_key(&pool, &master)?;
    let session_secret = master.derive("ui-session-v1");
    let tls_direct = cfg.tls_mode == TlsMode::Direct;

    let state = web::Data::new(AppState { pool, cfg: cfg.clone(), master, server_private_pem, server_public_pem, session_secret, hub: Default::default(), tls_direct, seen_jti: Mutex::new(Default::default()) });
    scheduler::spawn(state.clone());
    croustypatch_server::notify::worker(state.clone());

    let static_dir = cfg.static_dir.clone().canonicalize().ok().filter(|p| p.join("index.html").exists());
    let cors_origin = cfg.cors_origin.clone();
    let st = state.clone();
    let factory = move || {
        let mut app_cors = Cors::default();
        if let Some(o) = &cors_origin {
            app_cors = Cors::default().allowed_origin(o).allowed_methods(["GET", "POST", "PUT", "PATCH", "DELETE"]).allowed_headers([header::CONTENT_TYPE, header::AUTHORIZATION]).supports_credentials().max_age(600);
        }
        let mut headers = DefaultHeaders::new()
            .add(("X-Content-Type-Options", "nosniff"))
            .add(("X-Frame-Options", "DENY"))
            .add(("Referrer-Policy", "no-referrer"))
            .add(("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"));
        if tls_direct {
            headers = headers.add(("Strict-Transport-Security", "max-age=31536000"));
        }
        let mut app = App::new()
            .app_data(st.clone())
            .app_data(web::JsonConfig::default().limit(512 * 1024))
            .wrap(headers)
            .wrap(app_cors)
            .configure(api::configure);
        if let Some(dir) = &static_dir {
            app = app.service(actix_files::Files::new("/", dir).index_file("index.html").prefer_utf8(true));
        }
        app
    };

    let server = HttpServer::new(factory);
    let server = if tls_direct {
        let files = tls::ensure_files(cfg.tls_cert.as_deref(), cfg.tls_key.as_deref(), &cfg.data_dir, &cfg.tls_san)?;
        let rustls_cfg = tls::server_config(&files)?;
        tracing::info!(listen = %cfg.listen, cert = %files.cert.display(), self_signed = files.generated, "serving HTTPS (TLS 1.3 only)");
        server.bind_rustls_0_23(&cfg.listen, rustls_cfg)?
    } else {
        tracing::warn!(listen = %cfg.listen, "TLS mode 'proxy': serving plain HTTP, a reverse proxy must terminate TLS 1.3");
        server.bind(&cfg.listen)?
    };
    server.run().await?;
    Ok(())
}
