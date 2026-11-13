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
    /// Plain HTTP (a configured registry on the local network without TLS).
    pub insecure: bool,
    /// The login of a configured registry; otherwise what `docker login` recorded for the host is used.
    pub auth: Option<(String, String)>,
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
    Ok(ImageRef { registry, repository, tag, digest, insecure: false, auth: None })
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

/// `https://host`, except for a registry on this machine (`localhost`, `127.0.0.1`) or one configured as plain HTTP.
fn base_url_with(registry: &str, insecure: bool) -> String {
    let host = registry.rsplit_once(':').map(|(h, _)| h).unwrap_or(registry);
    if insecure || matches!(host, "localhost" | "127.0.0.1") { format!("http://{registry}") } else { format!("https://{registry}") }
}

fn base_url(image: &ImageRef) -> String {
    base_url_with(&image.registry, image.insecure)
}

/// The user and password `docker login` stored for a registry, from the `auths` of a Docker config file. Credential
/// helpers (`credsStore`, `credHelpers`) are not run: the agent only reads what is written in the file.
pub fn credentials_from_config(config: &str, registry: &str) -> Option<(String, String)> {
    use base64::Engine;
    let v: serde_json::Value = serde_json::from_str(config).ok()?;
    // `registry-1.docker.io` is Docker Hub: `docker login` records it as https://index.docker.io/v1/
    let hub = |h: &str| matches!(h, "registry-1.docker.io" | "index.docker.io" | "docker.io");
    let host_of = |k: &str| k.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("").to_ascii_lowercase();
    let want = registry.to_ascii_lowercase();
    let (key, entry) = v.get("auths")?.as_object()?.iter().find(|(k, _)| {
        let h = host_of(k);
        h == want || (hub(&h) && hub(&want))
    })?;
    let _ = key;
    if let (Some(u), Some(p)) = (entry.get("username").and_then(|x| x.as_str()), entry.get("password").and_then(|x| x.as_str())) {
        return Some((u.to_string(), p.to_string()));
    }
    let raw = base64::engine::general_purpose::STANDARD.decode(entry.get("auth")?.as_str()?).ok()?;
    let (u, p) = String::from_utf8(raw).ok()?.split_once(':').map(|(u, p)| (u.to_string(), p.to_string()))?;
    (!u.is_empty()).then_some((u, p))
}

/// `$DOCKER_CONFIG/config.json`, else `~/.docker/config.json` (the agent runs as root, as `docker compose pull` does).
fn docker_config_text() -> Option<String> {
    let dir = std::env::var("DOCKER_CONFIG").ok().filter(|d| !d.is_empty()).map(std::path::PathBuf::from).or_else(|| std::env::var("HOME").ok().filter(|h| !h.is_empty()).map(|h| std::path::Path::new(&h).join(".docker")))?;
    std::fs::read_to_string(dir.join("config.json")).or_else(|_| std::fs::read_to_string("/root/.docker/config.json")).ok()
}

/// An HTTP client that remembers the registry's bearer token between requests, and logs in with what `docker login`
/// recorded for that registry (private images), else anonymously.
struct Session {
    http: reqwest::Client,
    token: std::sync::Mutex<Option<String>>,
    credentials: Option<(String, String)>,
}

impl Session {
    fn for_image(image: &ImageRef) -> Result<Self> {
        let credentials = image.auth.clone().or_else(|| docker_config_text().and_then(|c| credentials_from_config(&c, &image.registry)));
        Ok(Self { http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent(concat!("croustypatch-agent/", env!("CARGO_PKG_VERSION"))).build()?, token: Default::default(), credentials })
    }

    async fn send(&self, method: reqwest::Method, url: &str, repository: &str, accept: Option<&str>) -> Result<reqwest::Response> {
        let build = |this: &Self| {
            let mut r = this.http.request(method.clone(), url);
            if let Some(a) = accept {
                r = r.header("accept", a);
            }
            if let Some(t) = this.token.lock().unwrap().clone() {
                r = r.bearer_auth(t);
            }
            r
        };
        let r = build(self).send().await?;
        if r.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(r);
        }
        let header = r.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        // a registry that wants basic authentication (a private one): the stored login
        if header.to_ascii_lowercase().starts_with("basic") {
            let (u, p) = self.credentials.clone().ok_or_else(|| anyhow!("registry wants a login: run `docker login` on this host as root"))?;
            let mut req = self.http.request(method.clone(), url).basic_auth(u, Some(p));
            if let Some(a) = accept {
                req = req.header("accept", a);
            }
            return Ok(req.send().await?);
        }
        let ch = bearer_challenge(&header).ok_or_else(|| anyhow!("registry wants an authentication this agent does not support"))?;
        let mut q: Vec<(String, String)> = ch.1;
        if !q.iter().any(|(k, _)| k == "scope") {
            q.push(("scope".into(), format!("repository:{repository}:pull")));
        }
        let qs: Vec<String> = q.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect();
        let sep = if ch.0.contains('?') { '&' } else { '?' };
        let mut tr = self.http.get(format!("{}{sep}{}", ch.0, qs.join("&")));
        if let Some((u, p)) = &self.credentials {
            tr = tr.basic_auth(u, Some(p));
        }
        let resp = tr.send().await?;
        if !resp.status().is_success() {
            bail!("token request answered {}{}", resp.status(), if self.credentials.is_none() && resp.status().as_u16() == 401 { " (private image? run `docker login` on this host as root)" } else { "" });
        }
        let tok: serde_json::Value = resp.json().await?;
        let token = tok.get("token").or_else(|| tok.get("access_token")).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("no token from the registry"))?;
        *self.token.lock().unwrap() = Some(token.to_string());
        Ok(build(self).send().await?)
    }
}

/// Current digest of `image` in its registry (the manifest list digest for multi-arch images, which is what
/// `RepoDigests` records after a pull).
pub async fn remote_digest(image: &ImageRef) -> Result<String> {
    let url = format!("{}/v2/{}/manifests/{}", base_url(image), image.repository, image.tag);
    let r = Session::for_image(image)?.send(reqwest::Method::HEAD, &url, &image.repository, Some(ACCEPT)).await?;
    if !r.status().is_success() {
        bail!("registry answered {} for {}:{}", r.status(), image.repository, image.tag);
    }
    r.headers().get("docker-content-digest").and_then(|v| v.to_str().ok()).map(str::to_owned).ok_or_else(|| anyhow!("registry gave no digest"))
}

/// Pages of 1000 tags read at most from one repository, and the time allowed for all of them. GHCR lists tags in no
/// particular order, and some repositories have more than a hundred pages (immich-machine-learning: a tag per commit and
/// per hardware variant): past these limits the list is partial, and the newest version may be missing from it.
const MAX_TAG_PAGES: usize = 100;
const TAG_LISTING_BUDGET: std::time::Duration = std::time::Duration::from_secs(40);

/// The tags read from a registry, and whether that is all of the ones that could matter.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub tags: Vec<String>,
    /// False when the page or time limit stopped the reading, or a later page failed: a newer tag may be missing.
    pub complete: bool,
}

/// The `rel="next"` target of a `Link` header, as an absolute URL on the registry itself (a link to another host is
/// ignored: the agent only talks to the registry the image comes from).
pub fn next_link(header: &str, registry: &str) -> Option<String> {
    next_link_with(header, registry, false)
}

pub fn next_link_with(header: &str, registry: &str, insecure: bool) -> Option<String> {
    let base = base_url_with(registry, insecure);
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
pub async fn list_tags_checked(image: &ImageRef) -> Result<Listing> {
    let (template, _) = tag_shape(&image.tag).ok_or_else(|| anyhow!("{} is not a version tag", image.tag))?;
    let prefix = tag_prefix(&image.tag);
    let mut kept: Vec<String> = vec![];
    let mut complete = false;
    let mut failure: Option<anyhow::Error> = None;
    let read = async {
        let s = Session::for_image(image)?;
        let mut url = format!("{}/v2/{}/tags/list?n=1000", base_url(image), image.repository);
        for _ in 0..MAX_TAG_PAGES {
            let r = s.send(reqwest::Method::GET, &url, &image.repository, None).await?;
            if !r.status().is_success() {
                bail!("registry answered {} listing the tags of {}", r.status(), image.repository);
            }
            let next = r.headers().get("link").and_then(|v| v.to_str().ok()).and_then(|h| next_link_with(h, &image.registry, image.insecure));
            let body: serde_json::Value = r.json().await?;
            let page: Vec<&str> = body.get("tags").and_then(|t| t.as_array()).into_iter().flatten().filter_map(|t| t.as_str()).collect();
            let done = page.last().is_some_and(|t| past_prefix(t, &prefix)) && page.windows(2).all(|w| w[0] <= w[1]);
            kept.extend(page.iter().filter(|t| tag_shape(t).is_some_and(|(tt, _)| tt == template)).map(|t| t.to_string()));
            match next {
                Some(n) if !done => url = n,
                // the end of the list, or the rest is past the tag's prefix in an alphabetical list
                _ => return Ok::<bool, anyhow::Error>(true),
            }
        }
        Ok(false)
    };
    match tokio::time::timeout(TAG_LISTING_BUDGET, read).await {
        Ok(Ok(c)) => complete = c,
        Ok(Err(e)) => failure = Some(e),
        Err(_) => {}
    }
    if let Some(e) = failure {
        if kept.is_empty() {
            return Err(e);
        }
    }
    Ok(Listing { tags: kept, complete })
}

/// [`list_tags_checked`] without the completeness flag.
pub async fn list_tags(image: &ImageRef) -> Result<Vec<String>> {
    Ok(list_tags_checked(image).await?.tags)
}

/// The text before the first number of a tag: `v` for `v3.3.1`, empty for `17-alpine`.
pub fn tag_prefix(tag: &str) -> String {
    tag.chars().take_while(|c| !c.is_ascii_digit()).collect()
}

/// A tag split into its numbers and the text around them: `v3.3.1` is the template `v#.#.#` with `[3, 3, 1]`, and
/// `16-vectorchord1.1.1` is `#-vectorchord#.#.#` with `[16, 1, 1, 1]`. A trailing build hash is left out
/// (`2026.10.12-sha-3699278` is `#.#.#-sha-HASH` with `[2026, 10, 12]`). None when there is no number to compare.
pub fn tag_shape(tag: &str) -> Option<(String, Vec<u64>)> {
    // a trailing build hash (`2026.10.12-sha-3699278`) is dropped, and only the version before it is compared
    let (tag, hashed) = match tag.rfind("-sha-") {
        Some(i) if tag.len() - i - 5 >= 7 && tag[i + 5..].chars().all(|c| c.is_ascii_hexdigit()) => (&tag[..i], true),
        _ => (tag, false),
    };
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
    if hashed {
        template.push_str("-sha-HASH");
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

/// The digit runs of a tag as written (`019` keeps its zero), to write the probed tags the same way.
fn digit_runs(tag: &str) -> Vec<String> {
    let tag = tag.rfind("-sha-").map_or(tag, |i| &tag[..i]);
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    for c in tag.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A tag from its template and numbers (`v#.#.#` and `[3, 4, 1]` is `v3.4.1`); a number that was written with leading
/// zeros in the running tag keeps that width.
fn render(template: &str, nums: &[u64], widths: &[usize]) -> String {
    let mut i = 0;
    template
        .chars()
        .map(|c| {
            if c == '#' {
                let n = nums.get(i).copied().unwrap_or(0);
                let w = widths.get(i).copied().unwrap_or(0);
                i += 1;
                format!("{n:0w$}")
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// The versions that could follow `cur`, highest first: each number raised by 1 to 3 (2 for all but the last), with
/// the numbers after it set to 0 (`v3.3.1` to `v3.4.0`) or left as they were (`16-vectorchord1.1.1` to
/// `17-vectorchord1.1.1`). `keep_major`: the first number stays (the newest of the running major).
pub fn probe_candidates(cur: &[u64], keep_major: bool) -> Vec<Vec<u64>> {
    let mut out: Vec<Vec<u64>> = vec![];
    for i in (0..cur.len()).filter(|i| !(keep_major && *i == 0)) {
        for d in 1..=if i + 1 == cur.len() { 3 } else { 2 } {
            let mut zero = cur.to_vec();
            zero[i] += d;
            zero[i + 1..].iter_mut().for_each(|n| *n = 0);
            let mut keep = cur.to_vec();
            keep[i] += d;
            for c in [zero, keep] {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out.sort_by(|a, b| b.cmp(a));
    out
}

/// Requests one search makes at most for an image.
const PROBE_BUDGET: usize = 90;

/// The search itself: from `start`, the highest existing candidate becomes the new start, until nothing follows.
async fn walk<F, Fut>(template: &str, widths: &[usize], start: Vec<u64>, keep_major: bool, exists: &mut F, budget: &mut usize) -> (Vec<u64>, Option<anyhow::Error>)
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let mut best = start.clone();
    let mut err: Option<anyhow::Error> = None;
    'rounds: for _ in 0..12 {
        let mut hit: Option<Vec<u64>> = None;
        for n in probe_candidates(&best, keep_major) {
            // a first number that jumps by dozens is another numbering scheme
            if n[0].saturating_sub(start[0]) > 50 {
                continue;
            }
            if *budget == 0 {
                break 'rounds;
            }
            *budget -= 1;
            match exists(render(template, &n, widths)).await {
                Ok(true) => {
                    hit = Some(n);
                    break; // highest first: the rest are lower
                }
                Ok(false) => {}
                Err(e) => {
                    err = Some(e);
                    break 'rounds;
                }
            }
        }
        match hit {
            Some(h) => best = h,
            None => break,
        }
    }
    (best, err)
}

/// Follow the versions of an image one step at a time instead of listing them: the registry is asked (through `exists`)
/// whether `v3.3.2`, `v3.4.0`, `v4.0.0`... exist, the highest that does becomes the new starting point, and the search
/// goes on until nothing follows. For repositories whose tag list is too long to read (a tag per commit): a handful of
/// requests find the newest release. `exists` failing (not "no such tag") ends the search with an error unless
/// something newer was already found.
pub async fn probe_newer<F, Fut>(current: &str, mut exists: F) -> Result<Option<Newer>>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let (template, cur) = tag_shape(current).ok_or_else(|| anyhow!("{current} is not a version tag"))?;
    if template.contains("-sha-") {
        bail!("a tag with a build hash cannot be followed by its number");
    }
    let widths: Vec<usize> = digit_runs(current).iter().map(|d| if d.len() > 1 && d.starts_with('0') { d.len() } else { 0 }).collect();
    let mut budget = PROBE_BUDGET;
    let (best, err) = walk(&template, &widths, cur.clone(), false, &mut exists, &mut budget).await;
    if best == cur {
        return match err {
            Some(e) => Err(e),
            None => Ok(None),
        };
    }
    let major = best[0] > cur[0];
    let same_major = if major { Some(walk(&template, &widths, cur.clone(), true, &mut exists, &mut budget).await.0).filter(|w| *w > cur).map(|w| render(&template, &w, &widths)) } else { None };
    Ok(Some(Newer { tag: render(&template, &best, &widths), major, same_major }))
}

/// [`probe_newer`] against the image's registry: does `<repository>:<tag>` have a manifest?
pub async fn probe_registry(image: &ImageRef) -> Result<Option<Newer>> {
    let s = Session::for_image(image)?;
    let base = base_url(image);
    probe_newer(&image.tag, |tag| {
        let (s, url, repo) = (&s, format!("{base}/v2/{}/manifests/{tag}", image.repository), image.repository.clone());
        async move {
            let r = s.send(reqwest::Method::HEAD, &url, &repo, Some(ACCEPT)).await?;
            match r.status().as_u16() {
                200 => Ok(true),
                404 => Ok(false),
                other => bail!("registry answered {other} for {repo}:{tag}"),
            }
        }
    })
    .await
}

/// Percent-encode a query component (RFC 3986 unreserved characters stay).
pub fn pct(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

// ----------------------------------------------------------------------------------- configured registries

/// The registries the administrator configured (Settings > Docker registries) and whether the installation is
/// offline: where an image is looked up, and with which login.
#[derive(Debug, Clone, Default)]
pub struct Registries {
    pub profiles: Vec<croustypatch_common::protocol::RegistryProfile>,
    pub offline: bool,
}

/// Docker Hub has three names; configured registries and image references use `docker.io`.
fn upstream(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    if matches!(h.as_str(), "docker.io" | "index.docker.io" | "registry-1.docker.io") { "docker.io".into() } else { h }
}

impl Registries {
    /// The places to look `img` up, in order: a registry that mirrors the image's own (a local pull-through cache
    /// first), the image's own registry when it is a configured one, then the image's own registry directly. Offline,
    /// only local registries count: an image that none of them holds has no place to be looked up.
    pub fn targets(&self, img: &ImageRef) -> Vec<ImageRef> {
        let usable = |p: &&croustypatch_common::protocol::RegistryProfile| !self.offline || p.kind == "local";
        let on = |p: &croustypatch_common::protocol::RegistryProfile, repository: String| ImageRef {
            registry: p.host.clone(),
            repository,
            tag: img.tag.clone(),
            digest: None,
            insecure: p.insecure,
            auth: (!p.username.is_empty()).then(|| (p.username.clone(), p.password.clone())),
        };
        let mut out: Vec<ImageRef> = vec![];
        let from = upstream(&img.registry);
        for p in self.profiles.iter().filter(usable) {
            if !p.host.eq_ignore_ascii_case(&img.registry) && p.mirror_of.iter().any(|m| upstream(m) == from) {
                out.push(on(p, format!("{}{}", p.path_prefix, img.repository)));
            }
        }
        if let Some(p) = self.profiles.iter().filter(usable).find(|p| p.host.eq_ignore_ascii_case(&img.registry)) {
            out.push(on(p, img.repository.clone()));
        }
        if !self.offline && !out.iter().any(|t| t.registry.eq_ignore_ascii_case(&img.registry)) {
            out.push(img.clone());
        }
        out
    }
}
