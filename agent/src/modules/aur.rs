//! Newer versions of AUR packages, for applications the app center installs by building them from the AUR (Microsoft's
//! Visual Studio Code on Arch): pacman's own update check only knows the official repositories, so an update of such
//! a package is invisible to it. The installed version is what pacman says, the latest is what the AUR says.
use crate::exec::{self, Cmd};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use croustypatch_common::protocol::valid_aur_name as valid_name;

/// The `Version` of the package in an AUR RPC `info` answer.
pub fn parse_version(rpc: &Value) -> Option<String> {
    rpc.get("results")?.as_array()?.first()?.get("Version")?.as_str().map(str::to_owned).filter(|v| !v.is_empty() && v.len() <= 64)
}

static CACHE: Mutex<Option<HashMap<String, (Instant, String)>>> = Mutex::new(None);

/// The version the AUR publishes now (asked at most once an hour per package).
pub async fn latest(pkg: &str) -> Result<String> {
    if !valid_name(pkg) {
        bail!("invalid AUR package name");
    }
    if let Some((at, v)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(pkg)) {
        if at.elapsed() < Duration::from_secs(3600) {
            return Ok(v.clone());
        }
    }
    let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent(concat!("croustypatch-agent/", env!("CARGO_PKG_VERSION"))).build()?;
    let r = http.get(format!("https://aur.archlinux.org/rpc/v5/info?arg%5B%5D={}", super::registry::pct(pkg))).send().await?;
    if !r.status().is_success() {
        bail!("the AUR answered {}", r.status());
    }
    let body: Value = r.json().await?;
    let v = parse_version(&body).ok_or_else(|| anyhow!("{pkg} is not in the AUR"))?;
    CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(pkg.to_string(), (Instant::now(), v.clone()));
    Ok(v)
}

/// The version of an installed package, from `pacman -Q`.
pub async fn installed(pkg: &str) -> Option<String> {
    let o = exec::run(&Cmd::new("pacman", &["-Q", pkg]).timeout(20)).await.ok().filter(|o| o.ok())?;
    let mut it = o.stdout.split_whitespace();
    (it.next() == Some(pkg)).then(|| it.next().map(str::to_owned)).flatten()
}

/// Is `latest` newer than `installed`? Decided by pacman's own `vercmp`, so epochs and pkgrel count as pacman counts them.
pub async fn is_newer(latest: &str, installed: &str) -> bool {
    exec::run(&Cmd::new("vercmp", &[latest, installed]).timeout(10)).await.ok().filter(|o| o.ok()).is_some_and(|o| o.stdout.trim() == "1")
}
