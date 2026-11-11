//! Proxmox VE Helper-Scripts (spec docker-pve §2): applications installed by the community scripts leave
//! `/opt/<app>_version.txt` (and an `update` command). They are reported as `helper:<app>` so they show up in the
//! inventory next to the system packages. Their updates stay with the script's own `update` command, which is
//! interactive; CroustyPatch reports, it does not run it.
use croustypatch_common::protocol::PackageEntry;
use std::path::Path;

pub fn detect(dir: &Path) -> Vec<PackageEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<PackageEntry> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let app = name.strip_suffix("_version.txt")?.to_string();
            if app.is_empty() || !app.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
                return None;
            }
            let v = std::fs::read_to_string(e.path()).ok()?.lines().next()?.trim().chars().take(64).collect::<String>();
            Some(PackageEntry { name: format!("helper:{app}"), version: if v.is_empty() { "unknown".into() } else { v }, candidate: None, is_security: false, cves: vec![], source: "helper-script".into(), meta: Default::default() })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn available() -> bool {
    !detect(Path::new("/opt")).is_empty()
}
