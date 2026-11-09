//! Container registry lookups (OCI distribution API): what digest does a tag point to right now?
//! Anonymous token flow, which covers Docker Hub, GHCR, Quay and most self-hosted registries for public images.
use anyhow::{anyhow, bail, Result};

/// A parsed image reference.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    /// Registry host for the API (docker.io -> registry-1.docker.io).
    pub registry: String,
    pub repository: String,
    pub tag: String,
    /// Pinned by digest: never "updated".
    pub digest: Option<String>,
}

pub fn parse(reference: &str) -> Result<ImageRef> {
    let r = reference.trim();
    if r.is_empty() || r.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("bad image reference {reference:?}");
    }
    let (rest, digest) = match r.split_once('@') {
        Some((a, d)) => (a, Some(d.to_string())),
        None => (r, None),
    };
    // The first path part is a registry if it has a dot or a port, or is "localhost".
    let (registry, path) = match rest.split_once('/') {
        Some((first, more)) if first.contains('.') || first.contains(':') || first == "localhost" => (first.to_string(), more.to_string()),
        _ => ("docker.io".to_string(), rest.to_string()),
    };
    let (repo, tag) = match path.rsplit_once(':') {
        Some((p, t)) if !t.contains('/') => (p.to_string(), t.to_string()),
        _ => (path.clone(), "latest".to_string()),
    };
    let repository = if registry == "docker.io" && !repo.contains('/') { format!("library/{repo}") } else { repo };
    let registry = if registry == "docker.io" { "registry-1.docker.io".to_string() } else { registry };
    Ok(ImageRef { registry, repository, tag, digest })
}

const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json";

/// Parse `Bearer realm="…",service="…",scope="…"`.
pub fn bearer_challenge(h: &str) -> Option<(String, Vec<(String, String)>)> {
    let rest = h.strip_prefix("Bearer ").or_else(|| h.strip_prefix("bearer "))?;
    let mut realm = None;
    let mut params = vec![];
    for part in rest.split(',') {
        let (k, v) = part.trim().split_once('=')?;
        let v = v.trim_matches('"').to_string();
        if k == "realm" { realm = Some(v) } else { params.push((k.to_string(), v)) }
    }
    Some((realm?, params))
}

/// Current digest of `image` in its registry (the manifest list digest for multi-arch images, which is what
/// `RepoDigests` records after a pull).
pub async fn remote_digest(image: &ImageRef) -> Result<String> {
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent(concat!("croustypatch-agent/", env!("CARGO_PKG_VERSION"))).build()?;
    let url = format!("https://{}/v2/{}/manifests/{}", image.registry, image.repository, image.tag);
    let mut r = http.head(&url).header("accept", ACCEPT).send().await?;
    if r.status() == reqwest::StatusCode::UNAUTHORIZED {
        let ch = r.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).and_then(bearer_challenge).ok_or_else(|| anyhow!("registry wants an authentication this agent does not support"))?;
        let mut q: Vec<(String, String)> = ch.1;
        if !q.iter().any(|(k, _)| k == "scope") {
            q.push(("scope".into(), format!("repository:{}:pull", image.repository)));
        }
        let qs: Vec<String> = q.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect();
        let sep = if ch.0.contains('?') { '&' } else { '?' };
        let resp = http.get(format!("{}{sep}{}", ch.0, qs.join("&"))).send().await?;
        if !resp.status().is_success() {
            bail!("token request answered {}", resp.status());
        }
        let tok: serde_json::Value = resp.json().await?;
        let token = tok.get("token").or_else(|| tok.get("access_token")).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("no token from the registry"))?;
        r = http.head(&url).header("accept", ACCEPT).bearer_auth(token).send().await?;
    }
    if !r.status().is_success() {
        bail!("registry answered {} for {}:{}", r.status(), image.repository, image.tag);
    }
    r.headers().get("docker-content-digest").and_then(|v| v.to_str().ok()).map(str::to_owned).ok_or_else(|| anyhow!("registry gave no digest"))
}

/// Percent-encode a query component (RFC 3986 unreserved characters stay).
pub fn pct(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
