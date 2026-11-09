//! Network quarantine and forensics freeze (the "double failure" protocol).
use crate::config::{write_private, Config};
use crate::exec::{self, Cmd, Runner};
use crate::runner::TaskLog;
use anyhow::{anyhow, bail, Result};
use croustypatch_common::protocol::{parse_cidr, IsolationSettings};
use std::net::IpAddr;
use std::path::PathBuf;

const NFT_TABLE: &str = "croustypatch_quarantine";

/// Inbound SSH rules kept during isolation: (nft lines, pf lines). Malformed sources are dropped, and if none is
/// left from a non-empty list, SSH stays closed rather than open to everyone.
fn ssh_rules(iso: &IsolationSettings) -> (String, String) {
    if !iso.keep_ssh || iso.ssh_port == 0 {
        return (String::new(), String::new());
    }
    let port = iso.ssh_port;
    if iso.ssh_sources.is_empty() {
        return (format!("    tcp dport {port} accept\n"), format!("pass in quick proto tcp from any to any port {port} keep state\n"));
    }
    let nets: Vec<(IpAddr, u8)> = iso.ssh_sources.iter().filter_map(|s| parse_cidr(s)).collect();
    let fmt = |v: &[&(IpAddr, u8)]| v.iter().map(|(ip, l)| format!("{ip}/{l}")).collect::<Vec<_>>().join(", ");
    let v4: Vec<&(IpAddr, u8)> = nets.iter().filter(|(i, _)| i.is_ipv4()).collect();
    let v6: Vec<&(IpAddr, u8)> = nets.iter().filter(|(i, _)| i.is_ipv6()).collect();
    let mut nft = String::new();
    if !v4.is_empty() {
        nft += &format!("    ip saddr {{ {} }} tcp dport {port} accept\n", fmt(&v4));
    }
    if !v6.is_empty() {
        nft += &format!("    ip6 saddr {{ {} }} tcp dport {port} accept\n", fmt(&v6));
    }
    let all: Vec<&(IpAddr, u8)> = nets.iter().collect();
    let pf = if all.is_empty() { String::new() } else { format!("pass in quick proto tcp from {{ {} }} to any port {port} keep state\n", fmt(&all)) };
    (nft, pf)
}

/// nftables ruleset: drop everything except loopback, established flows, the management server and (by default)
/// inbound SSH.
pub fn nft_ruleset(server_ips: &[IpAddr], iso: &IsolationSettings) -> String {
    let v4: Vec<String> = server_ips.iter().filter(|i| i.is_ipv4()).map(|i| i.to_string()).collect();
    let v6: Vec<String> = server_ips.iter().filter(|i| i.is_ipv6()).map(|i| i.to_string()).collect();
    let rule = |dir: &str, field: &str| -> String {
        let mut s = String::new();
        if !v4.is_empty() {
            s += &format!("    ip {field} {{ {} }} accept\n", v4.join(", "));
        }
        if !v6.is_empty() {
            s += &format!("    ip6 {field} {{ {} }} accept\n", v6.join(", "));
        }
        let _ = dir;
        s
    };
    format!(
        "add table inet {NFT_TABLE}\ndelete table inet {NFT_TABLE}\ntable inet {NFT_TABLE} {{\n  chain input {{\n    type filter hook input priority -300; policy drop;\n    iifname \"lo\" accept\n    ct state established,related accept\n{}{}  }}\n  chain output {{\n    type filter hook output priority -300; policy drop;\n    oifname \"lo\" accept\n    ct state established,related accept\n{}  }}\n}}\n",
        rule("in", "saddr"),
        ssh_rules(iso).0,
        rule("out", "daddr"),
    )
}

/// pf anchor rules (FreeBSD). `anchor "croustypatch_quarantine"` must be referenced from pf.conf.
pub fn pf_rules(server_ips: &[IpAddr], iso: &IsolationSettings) -> String {
    let list = server_ips.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
    format!("block drop all\npass quick on lo0 all\npass quick from {{ {list} }} to any keep state\npass quick from any to {{ {list} }} keep state\n{}", ssh_rules(iso).1)
}

/// One line for logs: what stays reachable.
pub fn describe(server_ips: &[IpAddr], iso: &IsolationSettings) -> String {
    let servers = server_ips.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
    match (iso.keep_ssh && iso.ssh_port != 0, iso.ssh_sources.is_empty()) {
        (false, _) => format!("only {servers} stays reachable"),
        (true, true) => format!("only {servers} and inbound SSH (port {}) stay reachable", iso.ssh_port),
        (true, false) => format!("only {servers} and inbound SSH (port {}) from {} stay reachable", iso.ssh_port, iso.ssh_sources.join(", ")),
    }
}

pub async fn quarantine(r: &Runner, server_ips: &[IpAddr], iso: &IsolationSettings, log: &TaskLog) -> Result<()> {
    if server_ips.is_empty() {
        // Cutting the host off from its own management server would make recovery impossible.
        bail!("cannot quarantine: the management server address is unknown");
    }
    log.line(format!("isolating host: {}", describe(server_ips, iso)));
    if cfg!(target_os = "freebsd") {
        let sr = r.read(Cmd::new("pfctl", &["-sr"])).await?;
        if !sr.stdout.contains(&format!("anchor \"{NFT_TABLE}\"")) && !r.dry_run {
            bail!("pf.conf does not reference anchor \"{NFT_TABLE}\"; add it so quarantine can take effect");
        }
        let _ = r.mutate(Cmd::new("pfctl", &["-e"]), log).await;
        let out = r.mutate(Cmd::new("pfctl", &["-a", NFT_TABLE, "-f", "-"]).stdin(pf_rules(server_ips, iso).into_bytes()), log).await?;
        if !out.ok() {
            bail!("pfctl failed ({})", out.code);
        }
    } else {
        let out = r.mutate(Cmd::new("nft", &["-f", "-"]).stdin(nft_ruleset(server_ips, iso).into_bytes()), log).await?;
        if !out.ok() {
            bail!("nft failed ({}): {}", out.code, out.stderr.trim());
        }
    }
    Ok(())
}

pub async fn release(r: &Runner, log: &TaskLog) -> Result<()> {
    if cfg!(target_os = "freebsd") {
        let out = r.mutate(Cmd::new("pfctl", &["-a", NFT_TABLE, "-F", "all"]), log).await?;
        if !out.ok() {
            bail!("pfctl flush failed ({})", out.code);
        }
    } else {
        let out = r.mutate(Cmd::new("nft", &["delete", "table", "inet", NFT_TABLE]), log).await?;
        // "No such file or directory": already released.
        if !out.ok() && !out.stderr.contains("No such file") {
            bail!("nft failed ({}): {}", out.code, out.stderr.trim());
        }
    }
    Ok(())
}

/// Preserve evidence: package state, recent system logs, the restore point and the task log.
pub async fn forensics_freeze(cfg: &Config, r: &Runner, task: uuid::Uuid, log: &TaskLog, extra: &[(&str, Vec<u8>)]) -> Result<PathBuf> {
    let dir = cfg.forensics_dir().join(task.to_string());
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut files: Vec<(String, Vec<u8>)> = vec![("task.log".into(), log.snapshot().join("\n").into_bytes())];
    files.extend(extra.iter().map(|(n, d)| (n.to_string(), d.clone())));
    for (name, cmd) in [
        ("journal.txt", Cmd::new("journalctl", &["-n", "3000", "--no-pager"])),
        ("dmesg.txt", Cmd::new("dmesg", &[])),
        ("processes.txt", Cmd::new("ps", &["aux"])),
    ] {
        if exec::has_program(cmd.program) {
            if let Ok(o) = r.read(cmd.timeout(30)).await {
                files.push((name.into(), o.stdout.into_bytes()));
            }
        }
    }
    let mut manifest = String::new();
    for (name, data) in &files {
        write_private(&dir.join(name), data)?;
        manifest += &format!("{}  {}\n", croustypatch_common::crypto::sha256_hex(data), name);
    }
    write_private(&dir.join("MANIFEST.sha256"), manifest.as_bytes())?;
    // Make the evidence immutable where the filesystem supports it (best effort).
    if exec::has_program("chattr") && !r.dry_run {
        let _ = r.read(Cmd::new("chattr", &["-R", "+i", &dir.to_string_lossy()])).await;
    }
    log.line(format!("evidence frozen in {}", dir.display()));
    Ok(dir)
}

pub async fn resolve_server(url: &str) -> Result<Vec<IpAddr>> {
    let hp = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
    let default_port = if url.starts_with("https") { 443 } else { 80 };
    let target = if hp.contains(':') { hp.to_string() } else { format!("{hp}:{default_port}") };
    let ips: Vec<IpAddr> = tokio::net::lookup_host(target).await.map_err(|e| anyhow!("cannot resolve {hp}: {e}"))?.map(|a| a.ip()).collect();
    Ok(ips)
}
