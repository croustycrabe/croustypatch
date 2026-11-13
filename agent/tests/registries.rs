//! Configured Docker registries: where an image is looked up (mirrors first, offline only local ones), the login of a
//! private registry, and `docker login` before a pull.
use croustypatch_agent::exec::Runner;
use croustypatch_agent::modules::{docker, registry};
use croustypatch_agent::runner::TaskLog;
use croustypatch_common::protocol::RegistryProfile;

fn profile(name: &str, kind: &str, host: &str, mirror_of: &[&str], prefix: &str, user: &str) -> RegistryProfile {
    RegistryProfile { name: name.into(), kind: kind.into(), host: host.into(), mirror_of: mirror_of.iter().map(|s| s.to_string()).collect(), path_prefix: prefix.into(), username: user.into(), password: if user.is_empty() { String::new() } else { "s3cret".into() }, insecure: true }
}

fn names(t: &[registry::ImageRef]) -> Vec<String> {
    t.iter().map(|i| format!("{}/{}:{}", i.registry, i.repository, i.tag)).collect()
}

#[test]
fn an_image_is_looked_up_where_the_configuration_says() {
    let img = |s: &str| registry::parse(s).unwrap();
    // nothing configured: the image's own registry, as before
    let none = registry::Registries::default();
    assert_eq!(names(&none.targets(&img("postgres:17-alpine"))), vec!["registry-1.docker.io/library/postgres:17-alpine"]);

    // a local pull-through cache of Docker Hub in a Harbor project: asked first, Docker Hub second
    let regs = registry::Registries { profiles: vec![profile("Harbor", "local", "harbor.lan", &["docker.io"], "dockerhub-proxy/", "robot")], offline: false };
    let t = regs.targets(&img("postgres:17-alpine"));
    assert_eq!(names(&t), vec!["harbor.lan/dockerhub-proxy/library/postgres:17-alpine", "registry-1.docker.io/library/postgres:17-alpine"]);
    assert_eq!(t[0].auth, Some(("robot".into(), "s3cret".into())), "the registry's login goes with the lookup");
    assert!(t[0].insecure && t[1].auth.is_none() && !t[1].insecure);
    // another registry is not mirrored by it
    assert_eq!(names(&regs.targets(&img("ghcr.io/immich-app/immich-server:v3.4.1"))), vec!["ghcr.io/immich-app/immich-server:v3.4.1"]);
    // an image that already lives on a configured registry keeps its path and gets that registry's login
    let own = regs.targets(&img("harbor.lan/team/app:1.0"));
    assert_eq!((names(&own), own[0].auth.is_some()), (vec!["harbor.lan/team/app:1.0".to_string()], true), "its own registry once, with the login");

    // offline: only local registries; a distant one is ignored, and an image no local registry holds has no place
    let offline = registry::Registries { profiles: vec![profile("Harbor", "local", "harbor.lan", &["docker.io", "ghcr.io"], "", ""), profile("Hub", "distant", "mirror.example.com", &["docker.io"], "", "")], offline: true };
    assert_eq!(names(&offline.targets(&img("postgres:17"))), vec!["harbor.lan/library/postgres:17"], "never the distant mirror, never Docker Hub itself");
    assert_eq!(names(&offline.targets(&img("ghcr.io/x/y:v1.0.0"))), vec!["harbor.lan/x/y:v1.0.0"]);
    assert!(offline.targets(&img("quay.io/x/y:v1")).is_empty(), "nothing local mirrors quay.io: nothing is asked");
    assert!(registry::Registries { profiles: vec![], offline: true }.targets(&img("postgres:17")).is_empty(), "offline without a local registry: nothing is asked");
    assert_eq!(names(&offline.targets(&img("harbor.lan/team/app:2"))), vec!["harbor.lan/team/app:2"], "an image on the local registry itself");
}

/// A local registry in plain HTTP that wants a basic login: repository `dockerhub/library/postgres` with two tags.
async fn local_registry() -> String {
    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let want = format!("authorization: basic {}", base64::engine::general_purpose::STANDARD.encode("robot:s3cret")).to_lowercase();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            let want = want.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let first = head.lines().next().unwrap_or("").to_string();
                let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, extra, body) = if !head.contains(&want) {
                    ("401 Unauthorized", "www-authenticate: Basic realm=\"local\"\r\n".to_string(), String::new())
                } else if path.starts_with("/v2/dockerhub/library/postgres/tags/list") {
                    ("200 OK", String::new(), serde_json::json!({"name": "dockerhub/library/postgres", "tags": ["16-alpine", "17-alpine", "18-alpine", "latest"]}).to_string())
                } else if path.starts_with("/v2/dockerhub/library/postgres/manifests/17-alpine") {
                    ("200 OK", format!("docker-content-digest: sha256:{}\r\n", "a".repeat(64)), String::new())
                } else {
                    ("404 Not Found", String::new(), String::new())
                };
                let _ = s.write_all(format!("HTTP/1.1 {status}\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            });
        }
    });
    format!("127.0.0.1:{port}")
}

#[tokio::test]
async fn a_local_mirror_with_a_login_answers_for_docker_hub_images() {
    let host = local_registry().await;
    let regs = registry::Registries { profiles: vec![profile("Local", "local", &host, &["docker.io"], "dockerhub/", "robot")], offline: true };
    let img = registry::parse("postgres:17-alpine").unwrap();
    let t = regs.targets(&img);
    assert_eq!(t.len(), 1);
    assert_eq!(registry::remote_digest(&t[0]).await.unwrap(), format!("sha256:{}", "a".repeat(64)), "the digest of the local copy, read with the login");
    // the versions the local registry holds: 18-alpine is the newer one
    let found = docker::newer_version_checked(&t[0]).await;
    assert_eq!((found.newer.map(|n| n.tag), found.how.as_str()), (Some("18-alpine".to_string()), "listed"));
    // a wrong password is an error the page can show, not "up to date"
    let wrong = registry::Registries { profiles: vec![RegistryProfile { password: "nope".into(), ..profile("Local", "local", &host, &["docker.io"], "dockerhub/", "robot") }], offline: true };
    let t = wrong.targets(&img);
    assert!(registry::remote_digest(&t[0]).await.is_err());
    // no login at all: the registry's refusal says what to do
    let anon = registry::Registries { profiles: vec![profile("Local", "local", &host, &["docker.io"], "dockerhub/", "")], offline: true };
    let e = registry::remote_digest(&anon.targets(&img)[0]).await.unwrap_err().to_string();
    assert!(e.contains("docker login") || e.contains("401"), "{e}");
}

#[tokio::test]
async fn the_login_is_made_before_a_pull_and_the_password_stays_off_the_command_line() {
    let regs = registry::Registries { profiles: vec![profile("Local", "local", "reg.lan:5000", &[], "", "robot"), profile("Open", "local", "open.lan", &[], "", ""), profile("Hub", "distant", "hub.example.com", &[], "", "bob")], offline: false };
    let log = TaskLog::default();
    let images = ["reg.lan:5000/team/app:2".to_string(), "reg.lan:5000/team/other:1".to_string(), "open.lan/x:1".to_string(), "hub.example.com/y:3".to_string(), "nginx:1".to_string()];
    docker::login(&Runner { dry_run: true }, &regs, &images, &log).await.unwrap();
    let lines = log.snapshot().join("\n");
    assert_eq!(lines.matches("docker login reg.lan:5000 -u robot --password-stdin").count(), 1, "once per registry: {lines}");
    assert!(lines.contains("docker login hub.example.com -u bob"), "a distant registry online: {lines}");
    assert!(!lines.contains("open.lan") && !lines.contains("s3cret"), "no login without credentials, and no password in the log: {lines}");
    // offline: a distant registry is not contacted
    let log = TaskLog::default();
    docker::login(&Runner { dry_run: true }, &registry::Registries { offline: true, ..regs }, &images, &log).await.unwrap();
    let lines = log.snapshot().join("\n");
    assert!(lines.contains("reg.lan:5000") && !lines.contains("hub.example.com"), "{lines}");
}
