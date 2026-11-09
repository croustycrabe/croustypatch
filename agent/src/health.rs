//! Post-patch health probes.
use crate::exec::{self, Cmd, Runner};
use croustypatch_common::model::HealthProbe;
use std::time::Duration;

fn unit_ok(u: &str) -> bool {
    !u.is_empty() && u.len() <= 128 && !u.starts_with('-') && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '@' | '-' | '\\'))
}

pub fn describe(p: &HealthProbe) -> String {
    match p {
        HealthProbe::Systemd { unit } => format!("service {unit} active"),
        HealthProbe::Tcp { host, port } => format!("tcp {host}:{port} reachable"),
        HealthProbe::Http { url, expect_status } => format!("GET {url} → {expect_status}"),
    }
}

pub async fn check(p: &HealthProbe, r: &Runner) -> Result<(), String> {
    match p {
        HealthProbe::Systemd { unit } => {
            if !unit_ok(unit) {
                return Err(format!("invalid unit name '{unit}'"));
            }
            let cmd = if exec::has_program("systemctl") { Cmd::new("systemctl", &["is-active", "--quiet", unit]) } else if exec::has_program("service") { Cmd::new("service", &[unit, "status"]) } else { return Err("neither systemctl nor service is available".into()) };
            match r.read(cmd.timeout(15)).await {
                Ok(o) if o.ok() => Ok(()),
                Ok(o) => Err(format!("{unit} is not active (exit {})", o.code)),
                Err(e) => Err(e.to_string()),
            }
        }
        HealthProbe::Tcp { host, port } => match tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect((host.as_str(), *port))).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("{host}:{port}: {e}")),
            Err(_) => Err(format!("{host}:{port}: connection timed out")),
        },
        HealthProbe::Http { url, expect_status } => {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err("only http(s) URLs are allowed".into());
            }
            let local = ["//localhost", "//127.0.0.1", "//[::1]"].iter().any(|h| url.contains(h));
            let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).redirect(reqwest::redirect::Policy::none()).danger_accept_invalid_certs(local).build().map_err(|e| e.to_string())?;
            match client.get(url).send().await {
                Ok(resp) if resp.status().as_u16() == *expect_status => Ok(()),
                Ok(resp) => Err(format!("{url} answered {} instead of {expect_status}", resp.status().as_u16())),
                Err(e) => Err(format!("{url}: {e}")),
            }
        }
    }
}
