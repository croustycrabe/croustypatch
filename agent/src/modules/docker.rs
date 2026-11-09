//! Docker module (spec docker-pve §3): containers are reported as packages named `docker:<container>`, so they go
//! through the normal pipeline (policy, risk, approval, window, restore point, health checks, rollback).
//!
//! Updates are applied to **Docker Compose** services only, the way their owner would: `docker compose pull` then
//! `up -d` for that one service, which keeps every volume, variable, network and port of the compose file. The
//! restore point is the previous image ID; rollback re-tags it and runs `up -d` again. Containers started by hand
//! are reported (with their available update) but never recreated: the agent cannot know which of their settings
//! are intentional.
use super::registry;
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
    Some(Container { name, image_ref, image_id, repo_digests, compose })
}

fn short(d: &str) -> String {
    d.rsplit(':').next().unwrap_or(d).chars().take(12).collect()
}

/// Is `remote` (a registry digest) the image this container runs?
pub fn is_current(c: &Container, remote: &str) -> bool {
    c.repo_digests.iter().any(|d| d.ends_with(&format!("@{remote}")))
}

pub fn entry(c: &Container, remote: Option<&str>) -> PackageEntry {
    let local = c.repo_digests.first().map(|d| short(d.split('@').nth(1).unwrap_or(d))).unwrap_or_else(|| short(&c.image_id));
    let candidate = remote.filter(|r| !is_current(c, r)).map(|r| format!("{} @{}", c.image_ref, short(r)));
    PackageEntry { name: format!("{PREFIX}{}", c.name), version: format!("{} @{local}", c.image_ref), candidate, is_security: false, cves: vec![], source: "docker".into() }
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

/// Inventory entries for every running container, with the registry's current digest when it can be checked.
pub async fn inventory(r: &Runner) -> Vec<PackageEntry> {
    let Ok(list) = containers(r).await.inspect_err(|e| tracing::warn!("docker inventory: {e:#}")) else { return vec![] };
    let mut out = vec![];
    for c in &list {
        let remote = match registry::parse(&c.image_ref) {
            Ok(img) if img.digest.is_none() => registry::remote_digest(&img).await.inspect_err(|e| tracing::debug!(container = %c.name, "registry check: {e:#}")).ok(),
            _ => None,
        };
        out.push(entry(c, remote.as_deref()));
    }
    out
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

/// Restore point: the image each container runs now.
pub async fn snapshot(r: &Runner, names: &[String], log: &TaskLog) -> Result<Vec<Container>> {
    let all = containers(r).await?;
    let mut pins = vec![];
    for n in names {
        let name = n.trim_start_matches(PREFIX);
        let c = all.iter().find(|c| c.name == name).ok_or_else(|| anyhow!("container {name} is not running"))?;
        if c.compose.is_none() {
            bail!("container {name} was not started by Docker Compose: update it by hand (CroustyPatch only recreates compose services, so their settings stay exactly as written)");
        }
        log.line(format!("restore point: {name} runs image {}", short(&c.image_id)));
        pins.push(c.clone());
    }
    Ok(pins)
}

pub async fn apply(r: &Runner, pins: &[Container], log: &TaskLog) -> Result<()> {
    for c in pins {
        let cp = c.compose.as_ref().expect("checked at snapshot");
        let pull = r.mutate(compose_cmd(cp, &["pull", "--quiet", &cp.service]), log).await?;
        if !pull.ok() {
            bail!("pull of {} failed: {}", c.name, pull.stderr.trim());
        }
        let up = r.mutate(compose_cmd(cp, &["up", "-d", "--no-deps", &cp.service]), log).await?;
        if !up.ok() {
            bail!("restart of {} failed: {}", c.name, up.stderr.trim());
        }
    }
    Ok(())
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
