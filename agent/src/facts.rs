//! Host facts: OS release, kernel, power state, addresses.
use crate::exec::{self, Cmd};
use croustypatch_common::model::PackageManager;
use croustypatch_common::protocol::HostFacts;
use std::collections::HashMap;

pub fn parse_os_release(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            Some((k.to_string(), v.trim().trim_matches('"').trim_matches('\'').to_string()))
        })
        .collect()
}

pub fn detect_package_manager(configured: &str) -> PackageManager {
    match configured {
        "apt" => PackageManager::Apt,
        "pacman" => PackageManager::Pacman,
        "dnf" => PackageManager::Dnf,
        _ => {
            if exec::has_program("apt-get") && exec::has_program("dpkg-query") {
                PackageManager::Apt
            } else if exec::has_program("pacman") {
                PackageManager::Pacman
            } else {
                PackageManager::Unknown
            }
        }
    }
}

/// Where the desktop app center is installed.
pub const PORTAL_GUI: &str = "/usr/local/bin/croustypatch-portal-gui";

/// Is a graphical desktop installed? A display manager service, or desktop sessions offered at login
/// (`xsessions` / `wayland-sessions`). The systemd default target is not used: distributions ship
/// `graphical.target` as the default even on servers. `root` is "/" except in tests.
pub fn desktop_detected_in(root: &std::path::Path) -> bool {
    let non_empty = |p: &str| std::fs::read_dir(root.join(p)).map(|mut d| d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".desktop")))).unwrap_or(false);
    root.join("etc/systemd/system/display-manager.service").exists()
        || ["usr/share/xsessions", "usr/share/wayland-sessions"].iter().any(|p| non_empty(p))
}

pub fn desktop_detected() -> bool {
    desktop_detected_in(std::path::Path::new("/"))
}

/// laptop (system battery) > workstation (graphical desktop) > server.
pub fn machine_type(is_laptop: bool, is_desktop: bool) -> &'static str {
    if is_laptop {
        "laptop"
    } else if is_desktop {
        "workstation"
    } else {
        "server"
    }
}

/// (is_laptop, on_battery)
pub async fn power_state() -> (bool, bool) {
    #[cfg(target_os = "linux")]
    {
        let mut has_battery = false;
        let mut discharging = false;
        let mut ac_online = None;
        if let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") {
            for e in rd.flatten() {
                let p = e.path();
                let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
                match kind.trim() {
                    "Battery" => {
                        // Skip peripherals (mice, headsets) that expose a battery with scope=Device.
                        if std::fs::read_to_string(p.join("scope")).map(|s| s.trim() == "Device").unwrap_or(false) {
                            continue;
                        }
                        has_battery = true;
                        discharging |= std::fs::read_to_string(p.join("status")).map(|s| s.trim() == "Discharging").unwrap_or(false);
                    }
                    "Mains" => ac_online = Some(std::fs::read_to_string(p.join("online")).map(|s| s.trim() == "1").unwrap_or(false)),
                    _ => {}
                }
            }
        }
        (has_battery, has_battery && (discharging || ac_online == Some(false)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        (false, false)
    }
}

pub async fn local_ip(server_url: &str) -> Vec<String> {
    // Connecting a UDP socket sends nothing; it only makes the kernel pick the outgoing interface.
    let host_port = server_url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
    let target = if host_port.contains(':') { host_port.to_string() } else { format!("{host_port}:443") };
    let mut out = vec![];
    if let Ok(mut addrs) = tokio::net::lookup_host(target).await {
        if let Some(a) = addrs.next() {
            if let Ok(s) = std::net::UdpSocket::bind(if a.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" }) {
                if s.connect(a).is_ok() {
                    if let Ok(l) = s.local_addr() {
                        out.push(l.ip().to_string());
                    }
                }
            }
        }
    }
    out
}

pub async fn collect(server_url: &str, pm: PackageManager, modules: Vec<String>) -> HostFacts {
    let (is_laptop, on_battery) = power_state().await;
    let (os_family, distro, os_version) = {
        let rel = parse_os_release(&std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).unwrap_or_default());
        ("linux".to_string(), rel.get("NAME").cloned().unwrap_or_else(|| "Linux".into()), rel.get("VERSION_ID").or(rel.get("BUILD_ID")).cloned().unwrap_or_default())
    };
    let kernel = if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default()
    } else {
        exec::run(&Cmd::new("uname", &["-r"])).await.ok().map(|o| o.stdout.trim().to_string()).unwrap_or_default()
    };
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default();
    let hostname = if hostname.is_empty() { exec::run(&Cmd::new("hostname", &[])).await.ok().map(|o| o.stdout.trim().to_string()).unwrap_or_else(|| "unknown".into()) } else { hostname };
    let is_desktop = desktop_detected();
    HostFacts {
        hostname,
        os_family,
        distro,
        os_version,
        kernel,
        arch: std::env::consts::ARCH.to_string(),
        package_manager: Some(pm),
        is_laptop,
        is_desktop,
        portal_gui: std::path::Path::new(PORTAL_GUI).is_file(),
        on_battery,
        virtualization: crate::sysinfo::virtualization(),
        ip_addresses: local_ip(server_url).await,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        modules,
        capabilities: croustypatch_common::protocol::capability::ALL.iter().map(|s| s.to_string()).collect(),
        hardware: crate::sysinfo::hardware(),
    }
}
