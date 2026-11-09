//! TLS 1.3-only client plumbing shared by the REST client and the WebSocket tunnel.
use crate::config::Config;
use anyhow::{bail, Context, Result};
use std::sync::Arc;

fn roots(cfg: &Config) -> Result<rustls::RootCertStore> {
    let mut store = rustls::RootCertStore::empty();
    if cfg.tls_ca_file.is_empty() {
        store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    } else {
        // A pinned CA/certificate replaces the public roots entirely.
        let pem = std::fs::read(&cfg.tls_ca_file).with_context(|| format!("reading {}", cfg.tls_ca_file))?;
        let mut n = 0;
        for cert in rustls_pemfile::certs(&mut &pem[..]) {
            store.add(cert?)?;
            n += 1;
        }
        if n == 0 {
            bail!("{} contains no certificate", cfg.tls_ca_file);
        }
    }
    Ok(store)
}

pub fn tls_config(cfg: &Config) -> Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("TLS 1.3 unavailable")?
        .with_root_certificates(roots(cfg)?)
        .with_no_client_auth())
}

pub fn http_client(cfg: &Config) -> Result<reqwest::Client> {
    reqwest::Client::builder().use_preconfigured_tls(tls_config(cfg)?).timeout(std::time::Duration::from_secs(60)).user_agent(concat!("croustypatch-agent/", env!("CARGO_PKG_VERSION"))).build().context("building HTTP client")
}

pub fn api_url(cfg: &Config, path: &str) -> String {
    format!("{}/api/v1{}", cfg.server_url.trim_end_matches('/'), path)
}

pub fn ws_url(cfg: &Config) -> String {
    let base = api_url(cfg, "/agent/ws");
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else {
        base.replacen("http://", "ws://", 1)
    }
}

// ------------------------------------------------------------------ pinned server addresses
//
// A quarantined host only reaches the server's IP addresses: DNS is cut like everything else. So the agent keeps the
// last addresses it resolved (in memory and in the state directory) and connects through them, with the URL's host
// name still used for TLS (SNI and certificate check) and the Host header. A restart of the server or of the agent
// therefore never strands an isolated host, and "restore the network" always reaches it.

/// Host name and port of a server URL (`https://name[:port]/...`).
pub fn server_host_port(url: &str) -> (String, u16) {
    let default_port = if url.starts_with("https") { 443 } else { 80 };
    let hp = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
    if let Some(rest) = hp.strip_prefix('[') {
        // [v6]:port
        let (h, p) = rest.split_once(']').unwrap_or((rest, ""));
        return (h.to_string(), p.trim_start_matches(':').parse().unwrap_or(default_port));
    }
    match hp.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().unwrap_or(default_port)),
        _ => (hp.to_string(), default_port),
    }
}

fn ips_file(cfg: &Config) -> std::path::PathBuf {
    cfg.state_dir.join("server_ips.json")
}

/// Last server addresses saved by [`save_server_ips`]; empty if none.
pub fn load_server_ips(cfg: &Config) -> Vec<std::net::IpAddr> {
    std::fs::read(ips_file(cfg)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_server_ips(cfg: &Config, ips: &[std::net::IpAddr]) {
    if let Ok(b) = serde_json::to_vec(ips) {
        let _ = crate::config::write_private(&ips_file(cfg), &b);
    }
}

/// REST client whose server name resolves to `ips` (no DNS needed); plain [`http_client`] when the list is empty.
pub fn http_client_pinned(cfg: &Config, ips: &[std::net::IpAddr]) -> Result<reqwest::Client> {
    if ips.is_empty() {
        return http_client(cfg);
    }
    let (host, port) = server_host_port(&cfg.server_url);
    let addrs: Vec<std::net::SocketAddr> = ips.iter().map(|ip| std::net::SocketAddr::new(*ip, port)).collect();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls_config(cfg)?)
        .resolve_to_addrs(&host, &addrs)
        .timeout(std::time::Duration::from_secs(60))
        .user_agent(concat!("croustypatch-agent/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building HTTP client")
}

/// TCP connection to the first of `ips` that answers on the server's port.
pub async fn connect_pinned(cfg: &Config, ips: &[std::net::IpAddr]) -> Result<tokio::net::TcpStream> {
    let (_, port) = server_host_port(&cfg.server_url);
    let mut last = None;
    for ip in ips {
        match tokio::time::timeout(std::time::Duration::from_secs(10), tokio::net::TcpStream::connect((*ip, port))).await {
            Ok(Ok(s)) => return Ok(s),
            Ok(Err(e)) => last = Some(anyhow::anyhow!("{ip}:{port}: {e}")),
            Err(_) => last = Some(anyhow::anyhow!("{ip}:{port}: timed out")),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no server address known")))
}
