//! Container registry lookups (OCI distribution API): what digest does a tag point to right now, and is there a newer
//! version tag (`v3.3.1` running, `v3.4.0` published)?
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

/// `https://host`, except for a registry on this machine (`localhost`, `127.0.0.1`), which commonly speaks plain HTTP.
fn base_url(registry: &str) -> String {
    let host = registry.rsplit_once(':').map(|(h, _)| h).unwrap_or(registry);
    if matches!(host, "localhost" | "127.0.0.1") { format!("http://{registry}") } else { format!("https://{registry}") }
}

/// An HTTP client that remembers the registry's anonymous bearer token between requests.
struct Session {
    http: reqwest::Client,
    token: Option<String>,
}

impl Session {
    fn new() -> Result<Self> {
        Ok(Self { http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent(concat!("croustypatch-agent/", env!("CARGO_PKG_VERSION"))).build()?, token: None })
    }

    async fn send(&mut self, method: reqwest::Method, url: &str, repository: &str, accept: Option<&str>) -> Result<reqwest::Response> {
        let build = |this: &Self| {
            let mut r = this.http.request(method.clone(), url);
            if let Some(a) = accept {
                r = r.header("accept", a);
            }
            if let Some(t) = &this.token {
                r = r.bearer_auth(t);
            }
            r
        };
        let r = build(self).send().await?;
        if r.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(r);
        }
        let ch = r.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).and_then(bearer_challenge).ok_or_else(|| anyhow!("registry wants an authentication this agent does not support"))?;
        let mut q: Vec<(String, String)> = ch.1;
        if !q.iter().any(|(k, _)| k == "scope") {
            q.push(("scope".into(), format!("repository:{repository}:pull")));
        }
        let qs: Vec<String> = q.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect();
        let sep = if ch.0.contains('?') { '&' } else { '?' };
        let resp = self.http.get(format!("{}{sep}{}", ch.0, qs.join("&"))).send().await?;
        if !resp.status().is_success() {
            bail!("token request answered {}", resp.status());
        }
        let tok: serde_json::Value = resp.json().await?;
        let token = tok.get("token").or_else(|| tok.get("access_token")).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("no token from the registry"))?;
        self.token = Some(token.to_string());
        Ok(build(self).send().await?)
    }
}

/// Current digest of `image` in its registry (the manifest list digest for multi-arch images, which is what
/// `RepoDigests` records after a pull).
pub async fn remote_digest(image: &ImageRef) -> Result<String> {
    let url = format!("{}/v2/{}/manifests/{}", base_url(&image.registry), image.repository, image.tag);
    let r = Session::new()?.send(reqwest::Method::HEAD, &url, &image.repository, Some(ACCEPT)).await?;
    if !r.status().is_success() {
        bail!("registry answered {} for {}:{}", r.status(), image.repository, image.tag);
    }
    r.headers().get("docker-content-digest").and_then(|v| v.to_str().ok()).map(str::to_owned).ok_or_else(|| anyhow!("registry gave no digest"))
}

/// Pages of 1000 tags read at most from one repository, and the time allowed for all of them. GHCR lists tags in no
/// particular order and immich-server has more than 33 000 (34 pages, about 10 s): the newest version can be on any page.
const MAX_TAG_PAGES: usize = 100;
const TAG_LISTING_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

/// The `rel="next"` target of a `Link` header, as an absolute URL on the registry itself (a link to another host is
/// ignored: the agent only talks to the registry the image comes from).
pub fn next_link(header: &str, registry: &str) -> Option<String> {
    let base = base_url(registry);
    header.split(',').find(|p| p.contains("rel=\"next\"") || p.contains("rel=next")).and_then(|p| {
        let target = p.split(';').next()?.trim().trim_start_matches('<').trim_end_matches('>');
        if target.starts_with('/') && !target.starts_with("//") {
            Some(format!("{base}{target}"))
        } else if target.starts_with(&format!("{base}/")) {
            Some(target.to_string())
        } else {
            None
        }
    })
}

/// Is `tag` past every tag that begins with `prefix` (alphabetically)?
fn past_prefix(tag: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        tag.chars().next().is_some_and(|c| c > '9')
    } else {
        tag > prefix && !tag.starts_with(prefix)
    }
}

/// The tags of `image`'s repository that have the shape of its own tag (`v3.3.1` and `v3.4.0`, not `v3.4.0-rc.1` or
/// `pr-123`), read page by page and filtered as they arrive. When a page shows that the registry lists tags
/// alphabetically (Docker Hub does) and has gone past the tag's prefix, the rest is not read.
pub async fn list_tags(image: &ImageRef) -> Result<Vec<String>> {
    let (template, _) = tag_shape(&image.tag).ok_or_else(|| anyhow!("{} is not a version tag", image.tag))?;
    let prefix = tag_prefix(&image.tag);
    tokio::time::timeout(TAG_LISTING_BUDGET, async {
        let mut s = Session::new()?;
        let mut url = format!("{}/v2/{}/tags/list?n=1000", base_url(&image.registry), image.repository);
        let mut kept = vec![];
        for _ in 0..MAX_TAG_PAGES {
            let r = s.send(reqwest::Method::GET, &url, &image.repository, None).await?;
            if !r.status().is_success() {
                bail!("registry answered {} listing the tags of {}", r.status(), image.repository);
            }
            let next = r.headers().get("link").and_then(|v| v.to_str().ok()).and_then(|h| next_link(h, &image.registry));
            let body: serde_json::Value = r.json().await?;
            let page: Vec<&str> = body.get("tags").and_then(|t| t.as_array()).into_iter().flatten().filter_map(|t| t.as_str()).collect();
            let done = page.last().is_some_and(|t| past_prefix(t, &prefix)) && page.windows(2).all(|w| w[0] <= w[1]);
            kept.extend(page.iter().filter(|t| tag_shape(t).is_some_and(|(tt, _)| tt == template)).map(|t| t.to_string()));
            match next {
                Some(n) if !done => url = n,
                _ => break,
            }
        }
        Ok(kept)
    })
    .await
    .map_err(|_| anyhow!("listing the tags of {} took more than {} s", image.repository, TAG_LISTING_BUDGET.as_secs()))?
}

/// The text before the first number of a tag: `v` for `v3.3.1`, empty for `17-alpine`.
pub fn tag_prefix(tag: &str) -> String {
    tag.chars().take_while(|c| !c.is_ascii_digit()).collect()
}

/// A tag split into its numbers and the text around them: `v3.3.1` is the template `v#.#.#` with `[3, 3, 1]`, and
/// `16-vectorchord1.1.1` is `#-vectorchord#.#.#` with `[16, 1, 1, 1]`. None when there is no number to compare, or
/// when the tag carries a build hash (`2026.10.12-sha-3699278`), whose digits mean nothing as a version.
pub fn tag_shape(tag: &str) -> Option<(String, Vec<u64>)> {
    let (mut template, mut nums) = (String::new(), vec![]);
    let mut chars = tag.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut n = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                n.push(d);
                chars.next();
            }
            nums.push(n.parse::<u64>().ok()?);
            template.push('#');
        } else {
            template.push(c);
            chars.next();
        }
    }
    if nums.is_empty() || nums.len() > 6 || template.to_ascii_lowercase().contains("sha") {
        return None;
    }
    Some((template, nums))
}

/// A published version newer than the one running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Newer {
    pub tag: String,
    /// The first number went up (`v2.5.0` to `v3.0.0`, `17-alpine` to `18-alpine`): a change that deserves a read of
    /// the release notes (and, for a database, a migration).
    pub major: bool,
    /// When the newest is a new major: the newest of the major that is running, if it is newer than the running tag.
    pub same_major: Option<String>,
}

/// The highest tag with the same shape as `current` and a greater version: only a `v#.#.#` tag is compared with
/// other `v#.#.#` tags, so release candidates, flavours (`-alpine`) and build tags are never offered by mistake.
pub fn newer_version(current: &str, tags: &[String]) -> Option<Newer> {
    let (template, cur) = tag_shape(current)?;
    let (mut best, mut within): (Option<(Vec<u64>, &String)>, Option<(Vec<u64>, &String)>) = (None, None);
    for t in tags {
        let Some((tt, n)) = tag_shape(t) else { continue };
        // A first number that jumps by dozens is another numbering scheme (a date tag next to `4`), not a release.
        if tt != template || n <= cur || n[0].saturating_sub(cur[0]) > 50 {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| n > *b) {
            best = Some((n.clone(), t));
        }
        if n[0] == cur[0] && within.as_ref().is_none_or(|(b, _)| n > *b) {
            within = Some((n, t));
        }
    }
    let (n, t) = best?;
    let major = n[0] > cur[0];
    Some(Newer { tag: t.clone(), major, same_major: if major { within.map(|(_, w)| w.clone()) } else { None } })
}

/// Percent-encode a query component (RFC 3986 unreserved characters stay).
pub fn pct(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
