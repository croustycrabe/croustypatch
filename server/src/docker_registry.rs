//! Docker registries (Settings > Docker registries): the places the agents look images up and pull them from, local
//! (on the local network) or distant (Docker Hub, GHCR, a private registry), with the login each one needs.
//!
//! Offline (air-gapped) installations have no way to reach a distant registry: the Docker features (newer-image
//! detection, container updates, new versions) need at least one local registry there, and are not switched on without
//! one. That rule is enforced where the features are switched on (the Docker module, per host or for the fleet), where
//! offline mode is chosen, and where the registries are changed.
//!
//! Only the passwords are secret: they are stored encrypted (like the AI key) and never returned; everything else is
//! kept in clear, so that the rule can be checked without the master key.
use crate::crypto::MasterKey;
use crate::error::{ApiError, ApiResult};
use crate::schema::{hosts, settings};
use crate::state::AppState;
use croustypatch_common::protocol::{AgentSettings, RegistryProfile};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

pub const KEY: &str = "docker_registries";
pub const MAX_REGISTRIES: usize = 20;

pub const OFFLINE_NEEDS_LOCAL: &str = "Offline mode has no access to the internet: the Docker features (newer-image detection, container updates, new versions) need a local Docker registry. Add one in Settings > Docker registries (kind: local) first, or keep the Docker module switched off.";

/// A registry as stored: the password only as ciphertext.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub id: String,
    pub name: String,
    /// `local` or `distant`.
    pub kind: String,
    pub host: String,
    pub mirror_of: Vec<String>,
    pub path_prefix: String,
    pub username: String,
    pub password_ct: String,
    pub insecure: bool,
}

/// A registry as the administrator edits it. `password` empty means "keep the stored one" for an existing registry.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Incoming {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub host: String,
    pub mirror_of: Vec<String>,
    pub path_prefix: String,
    pub username: String,
    pub password: String,
    pub insecure: bool,
}

pub fn load(c: &mut PgConnection) -> Vec<Stored> {
    let v: Option<Value> = settings::table.find(KEY).select(settings::value).first(c).optional().ok().flatten();
    v.and_then(|v| v.get("registries").cloned()).and_then(|r| serde_json::from_value(r).ok()).unwrap_or_default()
}

pub fn save_all(c: &mut PgConnection, regs: &[Stored]) -> ApiResult<()> {
    let v = json!({ "registries": regs });
    diesel::insert_into(settings::table)
        .values((settings::key.eq(KEY), settings::value.eq(&v), settings::encrypted.eq(false)))
        .on_conflict(settings::key)
        .do_update()
        .set((settings::value.eq(&v), settings::encrypted.eq(false), settings::updated_at.eq(chrono::Utc::now())))
        .execute(c)?;
    Ok(())
}

pub fn has_local(regs: &[Stored]) -> bool {
    regs.iter().any(|r| r.kind == "local")
}

/// Is the installation offline (Settings > Connectivity)?
pub fn offline(c: &mut PgConnection) -> bool {
    let v: Option<Value> = settings::table.find("connectivity").select(settings::value).first(c).optional().ok().flatten();
    v.and_then(|v| v.get("mode").and_then(|m| m.as_str()).map(|m| m == "air_gapped")).unwrap_or(false)
}

fn docker_flag(v: &Value) -> bool {
    v.get("docker").and_then(|d| d.as_bool()).unwrap_or(false)
}

/// Is the Docker module on for the fleet default or for any host?
pub fn docker_enabled(c: &mut PgConnection) -> bool {
    let fleet: Option<Value> = settings::table.find("modules").select(settings::value).first(c).optional().ok().flatten();
    if fleet.as_ref().is_some_and(docker_flag) {
        return true;
    }
    hosts::table.select(hosts::modules).load::<Value>(c).map(|all| all.iter().any(docker_flag)).unwrap_or(false)
}

/// Offline without a local registry: the Docker features cannot be on.
pub fn require_local_when_offline(c: &mut PgConnection) -> ApiResult<()> {
    if offline(c) && !has_local(&load(c)) {
        return Err(ApiError::bad(OFFLINE_NEEDS_LOCAL));
    }
    Ok(())
}

/// Called before a `connectivity` or `modules` setting is saved: refuse a change that would leave the Docker features
/// on in offline mode without a local registry.
pub fn check_setting_change(c: &mut PgConnection, key: &str, value: &Value) -> ApiResult<()> {
    match key {
        "connectivity" if value.get("mode").and_then(|m| m.as_str()) == Some("air_gapped") && docker_enabled(c) && !has_local(&load(c)) => Err(ApiError::bad(format!("{OFFLINE_NEEDS_LOCAL} (the Docker module is on)"))),
        "modules" if docker_flag(value) => require_local_when_offline(c),
        _ => Ok(()),
    }
}

// ------------------------------------------------------------------------------------------------ validation

/// `host` or `host:port`, as in an image reference: no scheme, no path.
pub fn valid_host(s: &str) -> bool {
    let (h, port) = match s.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (s, None),
    };
    !h.is_empty()
        && h.len() <= 253
        && !h.starts_with(['.', '-'])
        && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n)))
}

/// Docker Hub has three names; the one agents and the interface use is `docker.io`.
pub fn canonical_upstream(h: &str) -> String {
    let h = h.trim().to_ascii_lowercase();
    if matches!(h.as_str(), "docker.io" | "index.docker.io" | "registry-1.docker.io") { "docker.io".into() } else { h }
}

fn clean_line(s: &str, max: usize) -> Option<String> {
    let t = s.trim();
    (t.chars().count() <= max && !t.chars().any(|c| c.is_control())).then(|| t.to_string())
}

/// The registries as they will be stored (passwords encrypted; an empty one keeps the stored password of the same id),
/// or the first thing wrong with them.
pub fn check_set(incoming: &[Incoming], current: &[Stored], master: &MasterKey) -> Result<Vec<Stored>, String> {
    if incoming.len() > MAX_REGISTRIES {
        return Err(format!("at most {MAX_REGISTRIES} registries"));
    }
    let mut out: Vec<Stored> = vec![];
    for (i, r) in incoming.iter().enumerate() {
        let n = i + 1;
        let name = clean_line(&r.name, 60).filter(|s| !s.is_empty()).ok_or(format!("registry {n}: the name is 1 to 60 characters"))?;
        if !matches!(r.kind.as_str(), "local" | "distant") {
            return Err(format!("{name}: kind must be local or distant"));
        }
        let host = r.host.trim().to_ascii_lowercase();
        if !valid_host(&host) {
            return Err(format!("{name}: the address must be a host name with an optional port (registry.lan:5000), without https:// or a path"));
        }
        if out.iter().any(|o| o.host == host) {
            return Err(format!("{name}: {host} is already in the list"));
        }
        let mut mirror_of: Vec<String> = vec![];
        for m in &r.mirror_of {
            let m = canonical_upstream(m);
            if m.is_empty() {
                continue;
            }
            if !valid_host(&m) {
                return Err(format!("{name}: {m} is not a registry address (docker.io, ghcr.io…)"));
            }
            if !mirror_of.contains(&m) {
                mirror_of.push(m);
            }
        }
        if mirror_of.len() > 8 {
            return Err(format!("{name}: at most 8 mirrored registries"));
        }
        let prefix = r.path_prefix.trim().trim_start_matches('/');
        if prefix.len() > 100 || prefix.contains("..") || !prefix.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-')) {
            return Err(format!("{name}: the path prefix has letters, digits and . _ - / only (dockerhub-proxy/)"));
        }
        let path_prefix = if prefix.is_empty() || prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
        let username = clean_line(&r.username, 128).ok_or(format!("{name}: the user name is at most 128 characters"))?;
        if r.password.chars().count() > 512 || r.password.chars().any(|c| c.is_control()) {
            return Err(format!("{name}: the password is at most 512 printable characters"));
        }
        let id = if !r.id.is_empty() && r.id.len() <= 40 && r.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !out.iter().any(|o| o.id == r.id) { r.id.clone() } else { Uuid::new_v4().simple().to_string()[..12].to_string() };
        let previous = current.iter().find(|c| c.id == r.id && !r.id.is_empty());
        // no user name, no login: a password alone is dropped
        let password_ct = if username.is_empty() {
            String::new()
        } else if !r.password.is_empty() {
            master.encrypt(r.password.as_bytes())
        } else {
            previous.map(|p| p.password_ct.clone()).unwrap_or_default()
        };
        out.push(Stored { id, name, kind: r.kind.clone(), host, mirror_of, path_prefix, username, password_ct, insecure: r.insecure });
    }
    Ok(out)
}

/// What the interface shows: the passwords as a yes or no.
pub fn masked(regs: &[Stored]) -> Vec<Value> {
    regs.iter().map(|r| json!({ "id": r.id, "name": r.name, "kind": r.kind, "host": r.host, "mirror_of": r.mirror_of, "path_prefix": r.path_prefix, "username": r.username, "password_set": !r.password_ct.is_empty(), "insecure": r.insecure })).collect()
}

// ------------------------------------------------------------------------------------------------ agents

/// The registries an agent may use. Offline, only the local ones (a distant one cannot be reached).
pub fn profiles(regs: &[Stored], offline: bool, master: &MasterKey) -> Vec<RegistryProfile> {
    regs.iter()
        .filter(|r| !offline || r.kind == "local")
        .map(|r| RegistryProfile {
            name: r.name.clone(),
            kind: r.kind.clone(),
            host: r.host.clone(),
            mirror_of: r.mirror_of.clone(),
            path_prefix: r.path_prefix.clone(),
            username: r.username.clone(),
            password: if r.password_ct.is_empty() { String::new() } else { master.decrypt_string(&r.password_ct).map(|p| p.to_string()).unwrap_or_default() },
            insecure: r.insecure,
        })
        .collect()
}

/// `settings_for` plus the registries, for hosts with the Docker module on.
pub fn agent_settings(c: &mut PgConnection, st: &AppState, host_id: Uuid) -> ApiResult<Option<AgentSettings>> {
    let Some(mut s) = crate::modules::settings_for(c, host_id)? else { return Ok(None) };
    if s.modules.docker {
        s.registries = profiles(&load(c), s.offline, &st.master);
    }
    Ok(Some(s))
}

// ------------------------------------------------------------------------------------------ image on a registry

/// The image reference of `image` as held by registry `r`: the registry's address, its path prefix, the repository and
/// the tag (`ghcr.io/immich-app/immich-server:v3.4.1` on `registry.lan:5000` giving
/// `registry.lan:5000/immich-app/immich-server:v3.4.1`). Docker Hub official images get their `library/` only on a
/// registry that mirrors Docker Hub, as a pull-through cache keeps them. None for a digest or an unusable reference.
pub fn image_on(r: &Stored, image: &str) -> Option<String> {
    if image.contains('@') || image.is_empty() {
        return None;
    }
    let (path, tag) = match image.rsplit_once(':') {
        Some((p, t)) if !t.contains('/') => (p, t),
        _ => (image, "latest"),
    };
    let (upstream, repo) = match path.split_once('/') {
        Some((first, rest)) if first.contains('.') || first.contains(':') || first == "localhost" => (canonical_upstream(first), rest.to_string()),
        _ => ("docker.io".to_string(), path.to_string()),
    };
    let repo = if upstream == "docker.io" && !repo.contains('/') && r.mirror_of.iter().any(|m| m == "docker.io") { format!("library/{repo}") } else { repo };
    Some(format!("{}/{}{repo}:{tag}", r.host, r.path_prefix))
}
