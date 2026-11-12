//! Docker module (spec docker-pve §3): containers are reported as packages named `docker:<container>`, so they go
//! through the normal pipeline (policy, risk, approval, window, restore point, health checks, rollback).
//!
//! Updates are applied to **Docker Compose** services only, the way their owner would: `docker compose pull` then
//! `up -d` for that one service, which keeps every volume, variable, network and port of the compose file. The
//! restore point is the previous image ID; rollback re-tags it and runs `up -d` again. Containers started by hand
//! are reported (with their available update) but never recreated: the agent cannot know which of their settings
//! are intentional.
use super::{registry, retag};
use crate::exec::{Cmd, Runner};
use crate::runner::TaskLog;
use anyhow::{anyhow, bail, Result};
use croustypatch_common::protocol::PackageEntry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub const PREFIX: &str = "docker:";

pub fn available() -> bool {
    std::path::Path::new("/var/run/docker.sock").exists() && ["/usr/bin/docker", "/usr/local/bin/docker", "/bin/docker"].iter().any(|p| std::path::Path::new(p).exists())
}

/// What the agent knows about one running container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Container {
    pub name: String,
    pub image_ref: String,
    pub image_id: String,
    pub repo_digests: Vec<String>,
    pub compose: Option<Compose>,
    /// (host source, path inside the container) of its mounts: used to find the compose files of stacks deployed
    /// by a manager running in a container (Arcane, Dockge, Komodo…), whose labels hold the manager's own paths.
    #[serde(default)]
    pub mounts: Vec<(String, String)>,
    /// running, restarting, exited…
    #[serde(default)]
    pub state: String,
    /// healthy / unhealthy / starting, empty without a health check.
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Compose {
    pub project: String,
    pub service: String,
    pub working_dir: String,
    pub config_files: Vec<String>,
}

/// From one `docker inspect <container>` object and its image's RepoDigests.
pub fn container_from_inspect(v: &Value, repo_digests: Vec<String>) -> Option<Container> {
    let name = v.get("Name")?.as_str()?.trim_start_matches('/').to_string();
    let image_ref = v.pointer("/Config/Image")?.as_str()?.to_string();
    let image_id = v.get("Image")?.as_str()?.to_string();
    let label = |k: &str| v.pointer("/Config/Labels").and_then(|l| l.get(k)).and_then(|x| x.as_str()).map(str::to_owned);
    let compose = match (label("com.docker.compose.project"), label("com.docker.compose.service"), label("com.docker.compose.project.working_dir")) {
        (Some(project), Some(service), Some(working_dir)) => Some(Compose { project, service, working_dir, config_files: label("com.docker.compose.project.config_files").map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default() }),
        _ => None,
    };
    let mounts = v
        .get("Mounts")
        .and_then(|m| m.as_array())
        .map(|a| a.iter().filter_map(|m| Some((m.get("Source")?.as_str()?.to_string(), m.get("Destination")?.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let state = v.pointer("/State/Status").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let health = v.pointer("/State/Health/Status").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let started_at = v.pointer("/State/StartedAt").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Some(Container { name, image_ref, image_id, repo_digests, compose, mounts, state, health, started_at })
}

/// A path from a compose label, as it exists on the host. If it does not exist as written, it may be a path inside a
/// manager's container: map it through that container's mounts (the longest matching mount first) and keep the
/// first translation that exists. None when nothing on the host matches.
pub fn host_path(path: &str, mounts: &[(String, String)], exists: impl Fn(&std::path::Path) -> bool) -> Option<String> {
    use std::path::Path;
    if exists(Path::new(path)) {
        return Some(path.to_string());
    }
    let mut ordered: Vec<&(String, String)> = mounts.iter().filter(|(src, dst)| !src.is_empty() && dst.starts_with('/') && dst != "/").collect();
    ordered.sort_by_key(|(_, d)| std::cmp::Reverse(d.len()));
    ordered.into_iter().find_map(|(src, dst)| {
        let rest = Path::new(path).strip_prefix(dst).ok()?;
        let cand = Path::new(src).join(rest);
        (!cand.components().any(|c| matches!(c, std::path::Component::ParentDir)) && exists(&cand)).then(|| cand.to_string_lossy().into_owned())
    })
}

/// The compose project with host paths, or a clear reason why the agent cannot drive it.
pub fn resolve_compose(c: &Compose, all_mounts: &[(String, String)], exists: impl Fn(&std::path::Path) -> bool + Copy) -> Result<Compose> {
    let files = if c.config_files.is_empty() { vec![format!("{}/compose.yaml", c.working_dir.trim_end_matches('/'))] } else { c.config_files.clone() };
    let mut out = c.clone();
    out.config_files = vec![];
    for f in &files {
        let h = host_path(f, all_mounts, exists).ok_or_else(|| anyhow!("compose file {f} is not on this host (the stack was probably deployed by a tool running in a container whose files are not mounted from the host): update it from that tool; nothing was changed"))?;
        out.config_files.push(h);
    }
    out.working_dir = host_path(&c.working_dir, all_mounts, exists).ok_or_else(|| anyhow!("compose project directory {} is not on this host; nothing was changed", c.working_dir))?;
    Ok(out)
}

fn short(d: &str) -> String {
    d.rsplit(':').next().unwrap_or(d).chars().take(12).collect()
}

/// Is `remote` (a registry digest) the image this container runs?
pub fn is_current(c: &Container, remote: &str) -> bool {
    c.repo_digests.iter().any(|d| d.ends_with(&format!("@{remote}")))
}

pub fn entry(c: &Container, remote: Option<&str>) -> PackageEntry {
    entry_with(c, remote, None, None)
}

/// The inventory entry of a container. `newer` is a newer version tag the registry offers for the image (the running
/// tag's own digest is what `candidate` tracks): it is informative, because moving to another tag means changing the
/// compose file, which CroustyPatch does not do.
pub fn entry_with(c: &Container, remote: Option<&str>, newer: Option<&registry::Newer>, apply: Option<Value>) -> PackageEntry {
    let local = c.repo_digests.first().map(|d| short(d.split('@').nth(1).unwrap_or(d))).unwrap_or_else(|| short(&c.image_id));
    let candidate = remote.filter(|r| !is_current(c, r)).map(|r| format!("{} @{}", c.image_ref, short(r)));
    let mut meta = serde_json::json!({
        "image": c.image_ref, "project": c.compose.as_ref().map(|x| x.project.as_str()), "service": c.compose.as_ref().map(|x| x.service.as_str()),
        "state": c.state, "health": c.health, "started_at": c.started_at,
    });
    if let Some(n) = newer {
        meta["latest"] = serde_json::json!({ "tag": n.tag, "major": n.major, "same_major": n.same_major });
        if let Some(a) = apply {
            meta["latest"]["apply"] = a;
        }
    }
    PackageEntry { name: format!("{PREFIX}{}", c.name), version: format!("{} @{local}", c.image_ref), candidate, is_security: false, cves: vec![], source: "docker".into(), meta }
}

/// What a version change needs for each pinned container that has one: (service, image it runs, image to move to,
/// compose files), in the order of `names`. A request for a container that is not among the pins is an error.
pub fn retag_requests(pins: &[Container], retag: &std::collections::BTreeMap<String, String>) -> Result<Vec<(String, String, String, Vec<String>)>> {
    for k in retag.keys() {
        if !pins.iter().any(|p| &format!("{PREFIX}{}", p.name) == k) {
            bail!("a new version was asked for {k}, which is not part of this update");
        }
    }
    pins.iter()
        .filter_map(|p| retag.get(&format!("{PREFIX}{}", p.name)).map(|new| (p, new)))
        .map(|(p, new)| {
            let cp = p.compose.as_ref().ok_or_else(|| anyhow!("{} is not a compose service", p.name))?;
            Ok((cp.service.clone(), p.image_ref.clone(), new.clone(), cp.config_files.clone()))
        })
        .collect()
}

/// For the inventory: can the suggested version be applied to this container, and by changing which file? Planned
/// on the real files without writing anything.
pub fn retag_hint(c: &Container, all_mounts: &[(String, String)], new_ref: &str, major: bool) -> Value {
    let manual = |reason: String| serde_json::json!({ "mode": "manual", "reason": reason });
    if major && croustypatch_common::model::is_database_image(&c.image_ref) {
        return manual(croustypatch_common::model::DATABASE_MAJOR_REASON.to_string());
    }
    let Some(cp) = &c.compose else { return manual("the container was not started by Docker Compose".into()) };
    let resolved = match resolve_compose(cp, all_mounts, |p| p.exists()) {
        Ok(r) => r,
        Err(e) => return manual(format!("{e:#}")),
    };
    match retag::plan_all(&[(resolved.service.clone(), c.image_ref.clone(), new_ref.to_string(), resolved.config_files.clone())], &retag::read_files) {
        Ok((_, outcomes)) => match outcomes.first() {
            Some(retag::Outcome::AlreadyThere { path }) => serde_json::json!({ "mode": "file", "file": path, "already": true }),
            Some(retag::Outcome::Changed { path }) => serde_json::json!({ "mode": "file", "file": path }),
            None => manual("nothing to plan".into()),
        },
        Err(reason) => manual(reason),
    }
}

type NewerCache = std::sync::Mutex<std::collections::HashMap<(String, String, String), (std::time::Instant, Option<registry::Newer>, bool)>>;

/// The newer version of `img`'s tag, from the registry's tag list. Asked at most every 6 hours per image (a failed
/// lookup again after 20 minutes): versions do not appear by the minute, and a registry should not be queried for
/// every inventory. No request at all when the tag is not a version (`latest`, `release`).
pub async fn newer_version(img: &registry::ImageRef) -> Option<registry::Newer> {
    static CACHE: std::sync::OnceLock<NewerCache> = std::sync::OnceLock::new();
    registry::tag_shape(&img.tag)?;
    let cache = CACHE.get_or_init(Default::default);
    let key = (img.registry.clone(), img.repository.clone(), img.tag.clone());
    if let Some((at, found, ok)) = cache.lock().ok()?.get(&key) {
        if at.elapsed() < Duration::from_secs(if *ok { 6 * 3600 } else { 20 * 60 }) {
            return found.clone();
        }
    }
    let (found, ok) = match registry::list_tags(img).await {
        Ok(tags) => (registry::newer_version(&img.tag, &tags), true),
        Err(e) => {
            tracing::debug!(image = %format!("{}:{}", img.repository, img.tag), "tag list: {e:#}");
            (None, false)
        }
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(key, (std::time::Instant::now(), found.clone(), ok));
    }
    found
}

pub async fn containers(r: &Runner) -> Result<Vec<Container>> {
    let ids = r.read(Cmd::new("docker", &["ps", "-q", "--no-trunc"])).await?;
    if !ids.ok() {
        bail!("docker ps failed: {}", ids.stderr.trim());
    }
    let mut out = vec![];
    for id in ids.stdout.split_whitespace().take(500) {
        let i = r.read(Cmd::new("docker", &["inspect", id])).await?;
        let Ok(Value::Array(a)) = serde_json::from_str::<Value>(&i.stdout) else { continue };
        let Some(v) = a.first() else { continue };
        let image_id = v.get("Image").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let im = r.read(Cmd::new("docker", &["image", "inspect", "--format", "{{json .RepoDigests}}", &image_id])).await?;
        let digests: Vec<String> = serde_json::from_str(im.stdout.trim()).unwrap_or_default();
        if let Some(c) = container_from_inspect(v, digests) {
            out.push(c);
        }
    }
    Ok(out)
}

/// Inventory entries for every running container, with the registry's current digest and the newest version tag
/// when they can be checked (four registry lookups at a time).
/// `online` false (an air-gapped installation): the containers are reported, no registry is asked.
pub async fn inventory(r: &Runner, online: bool) -> Vec<PackageEntry> {
    let Ok(list) = containers(r).await.inspect_err(|e| tracing::warn!("docker inventory: {e:#}")) else { return vec![] };
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let mut jobs = tokio::task::JoinSet::new();
    for (i, c) in list.iter().enumerate() {
        let image = c.image_ref.clone();
        let name = c.name.clone();
        let gate = gate.clone();
        jobs.spawn(async move {
            let _slot = gate.acquire().await;
            match registry::parse(&image) {
                // pinned by digest: never updated, nothing to look for
                Ok(img) if img.digest.is_none() && online => {
                    let remote = registry::remote_digest(&img).await.inspect_err(|e| tracing::debug!(container = %name, "registry check: {e:#}")).ok();
                    (i, remote, newer_version(&img).await)
                }
                _ => (i, None, None),
            }
        });
    }
    let mut found: Vec<(Option<String>, Option<registry::Newer>)> = vec![(None, None); list.len()];
    while let Some(Ok((i, remote, newer))) = jobs.join_next().await {
        found[i] = (remote, newer);
    }
    let all_mounts: Vec<(String, String)> = list.iter().flat_map(|c| c.mounts.iter().cloned()).collect();
    list.iter()
        .zip(found)
        .map(|(c, (remote, newer))| {
            // can the suggested version be applied (and by changing which file)? Planned on the real compose files.
            let apply = newer.as_ref().and_then(|n| registry::parse(&c.image_ref).ok().map(|_| n)).map(|n| retag_hint(c, &all_mounts, &format!("{}:{}", retag::split_tag(&c.image_ref).map_or(c.image_ref.as_str(), |t| t.0), n.tag), n.major));
            entry_with(c, remote.as_deref(), newer.as_ref(), apply)
        })
        .collect()
}

fn compose_cmd(c: &Compose, tail: &[&str]) -> Cmd {
    let mut args: Vec<String> = vec!["compose".into(), "--project-directory".into(), c.working_dir.clone()];
    for f in &c.config_files {
        args.push("-f".into());
        args.push(f.clone());
    }
    args.extend(["-p".to_string(), c.project.clone()]);
    args.extend(tail.iter().map(|s| s.to_string()));
    Cmd::new("docker", &[]).args(args).timeout(900)
}

/// Restore point: the image each container runs now. Every check happens here, before anything changes: a container
/// that is not a compose service, or whose compose files cannot be found on the host, fails the task cleanly.
pub async fn snapshot(r: &Runner, names: &[String], log: &TaskLog) -> Result<Vec<Container>> {
    let all = containers(r).await?;
    let all_mounts: Vec<(String, String)> = all.iter().flat_map(|c| c.mounts.iter().cloned()).collect();
    let mut pins = vec![];
    for n in names {
        let name = n.trim_start_matches(PREFIX);
        let c = all.iter().find(|c| c.name == name).ok_or_else(|| anyhow!("container {name} is not running"))?;
        let Some(cp) = &c.compose else {
            bail!("container {name} was not started by Docker Compose: update it by hand (CroustyPatch only recreates compose services, so their settings stay exactly as written); nothing was changed");
        };
        let resolved = resolve_compose(cp, &all_mounts, |p| p.exists()).map_err(|e| anyhow!("{name}: {e:#}"))?;
        if resolved.config_files != cp.config_files {
            log.line(format!("{name}: compose file {} found on the host as {}", cp.config_files.join(", "), resolved.config_files.join(", ")));
        }
        log.line(format!("restore point: {name} runs image {}", short(&c.image_id)));
        pins.push(Container { compose: Some(resolved), ..c.clone() });
    }
    Ok(pins)
}

/// Pull and recreate each service. `touched` receives the index of every container that was (or may have been)
/// recreated: a failure before that point changed nothing for it, so it needs no rollback.
pub async fn apply(r: &Runner, pins: &[Container], log: &TaskLog, touched: &mut Vec<usize>) -> Result<()> {
    for (i, c) in pins.iter().enumerate() {
        let cp = c.compose.as_ref().expect("checked at snapshot");
        let pull = r.mutate(compose_cmd(cp, &["pull", "--quiet", &cp.service]), log).await?;
        if !pull.ok() {
            bail!("pull of {} failed: {}", c.name, pull.stderr.trim());
        }
        touched.push(i);
        let up = r.mutate(compose_cmd(cp, &["up", "-d", "--no-deps", &cp.service]), log).await?;
        if !up.ok() {
            bail!("restart of {} failed: {}", c.name, up.stderr.trim());
        }
    }
    Ok(())
}

/// Do these containers run their restore-point image again? Checked after a rollback whose commands reported an
/// error: what counts is the state of the machine, not the exit code.
pub async fn running_on_pins(r: &Runner, pins: &[Container]) -> bool {
    for c in pins {
        let Ok(o) = r.read(Cmd::new("docker", &["inspect", "--format", "{{.Image}} {{.State.Status}}", &c.name])).await else { return false };
        if !o.ok() || o.stdout.trim() != format!("{} running", c.image_id) {
            return false;
        }
    }
    true
}

/// Wait until every updated container is running (and healthy, when it has a health check).
pub async fn wait_healthy(r: &Runner, pins: &[Container], log: &TaskLog) -> Result<()> {
    if r.dry_run {
        return Ok(());
    }
    for c in pins {
        let mut last = String::new();
        let mut ok = false;
        for _ in 0..24 {
            let o = r.read(Cmd::new("docker", &["inspect", "--format", "{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}", &c.name])).await?;
            last = o.stdout.trim().to_string();
            if last == "running none" || last == "running healthy" {
                ok = true;
                break;
            }
            if last.starts_with("exited") || last.ends_with("unhealthy") {
                break;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        if !ok {
            bail!("{} is not healthy after the update ({last})", c.name);
        }
        log.line(format!("{} is up ({last})", c.name));
    }
    Ok(())
}

/// Put the previous images back.
pub async fn restore(r: &Runner, pins: &[Container], log: &TaskLog) -> Result<()> {
    for c in pins {
        let cp = c.compose.as_ref().ok_or_else(|| anyhow!("{} is not a compose service", c.name))?;
        let tag = r.mutate(Cmd::new("docker", &["tag", &c.image_id, &c.image_ref]), log).await?;
        if !tag.ok() {
            bail!("cannot re-tag the previous image of {}: {}", c.name, tag.stderr.trim());
        }
        let up = r.mutate(compose_cmd(cp, &["up", "-d", "--no-deps", "--pull", "never", &cp.service]), log).await?;
        if !up.ok() {
            bail!("restart of {} on its previous image failed: {}", c.name, up.stderr.trim());
        }
    }
    Ok(())
}
