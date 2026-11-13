//! Optional modules: image references, registry auth challenge, container inventory, helper scripts.
use croustypatch_agent::modules::{docker, helper_scripts, registry};
use serde_json::json;

#[test]
fn image_references_resolve_like_docker_does() {
    let r = registry::parse("nginx").unwrap();
    assert_eq!((r.registry.as_str(), r.repository.as_str(), r.tag.as_str()), ("registry-1.docker.io", "library/nginx", "latest"));
    let r = registry::parse("louislam/uptime-kuma:1").unwrap();
    assert_eq!((r.repository.as_str(), r.tag.as_str()), ("louislam/uptime-kuma", "1"));
    let r = registry::parse("ghcr.io/home-assistant/home-assistant:stable").unwrap();
    assert_eq!((r.registry.as_str(), r.repository.as_str(), r.tag.as_str()), ("ghcr.io", "home-assistant/home-assistant", "stable"));
    let r = registry::parse("registry.lan:5000/team/app").unwrap();
    assert_eq!((r.registry.as_str(), r.repository.as_str(), r.tag.as_str()), ("registry.lan:5000", "team/app", "latest"));
    assert!(registry::parse("postgres:16@sha256:abc").unwrap().digest.is_some(), "digest-pinned images are never updated");
    assert!(registry::parse("bad ref").is_err());
}

#[test]
fn registry_challenges_are_parsed() {
    let (realm, p) = registry::bearer_challenge(r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull""#).unwrap();
    assert_eq!(realm, "https://auth.docker.io/token");
    assert!(p.contains(&("service".into(), "registry.docker.io".into())));
    assert_eq!(registry::pct("repository:a/b:pull"), "repository%3Aa%2Fb%3Apull");
}

#[test]
fn containers_become_packages_with_an_update_when_the_registry_moved() {
    let inspect = json!({ "Name": "/uptime", "Image": "sha256:1111aaaa2222", "Config": { "Image": "louislam/uptime-kuma:1", "Labels": {
        "com.docker.compose.project": "uptime", "com.docker.compose.service": "kuma", "com.docker.compose.project.working_dir": "/opt/stacks/uptime",
        "com.docker.compose.project.config_files": "/opt/stacks/uptime/compose.yaml" } } });
    let c = docker::container_from_inspect(&inspect, vec!["louislam/uptime-kuma@sha256:aaaa111122223333".into()]).unwrap();
    assert_eq!(c.compose.as_ref().unwrap().config_files, vec!["/opt/stacks/uptime/compose.yaml".to_string()]);
    let same = docker::entry(&c, Some("sha256:aaaa111122223333"));
    assert_eq!((same.name.as_str(), same.source.as_str(), same.candidate.clone()), ("docker:uptime", "docker", None));
    let newer = docker::entry(&c, Some("sha256:bbbb444455556666"));
    assert!(newer.candidate.unwrap().contains("bbbb44445555"));
    assert!(croustypatch_common::model::is_valid_package_name(&newer.name));
    let manual = docker::container_from_inspect(&json!({ "Name": "/x", "Image": "sha256:1", "Config": { "Image": "x", "Labels": {} } }), vec![]).unwrap();
    assert!(manual.compose.is_none(), "containers started by hand are reported, not recreated");
}

#[test]
fn helper_script_apps_are_detected_from_their_version_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vaultwarden_version.txt"), "1.32.0\n").unwrap();
    std::fs::write(dir.path().join("../../evil_version.txt".replace("../", "")), "x").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    let found = helper_scripts::detect(dir.path());
    let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["helper:evil", "helper:vaultwarden"]);
    assert_eq!(found[1].version, "1.32.0");
    assert!(found.iter().all(|p| p.candidate.is_none() && p.source == "helper-script"));
}

#[test]
fn compose_files_of_a_manager_in_a_container_are_found_through_its_mounts() {
    // The real case: Arcane runs in a container and writes its own paths into the compose labels.
    let host = tempfile::tempdir().unwrap();
    let vol = host.path().join("volumes/arcane-agent_arcane-data/_data");
    std::fs::create_dir_all(vol.join("projects/hoppscotch")).unwrap();
    std::fs::write(vol.join("projects/hoppscotch/compose.yaml"), "services: {}").unwrap();
    let mounts = vec![
        (host.path().join("termix-data").to_string_lossy().into_owned(), "/app/data".to_string()), // another container, same mount point
        (vol.to_string_lossy().into_owned(), "/app/data".to_string()),
        ("/var/run/docker.sock".to_string(), "/var/run/docker.sock".to_string()),
    ];
    let c = docker::Compose { project: "hoppscotch".into(), service: "db".into(), working_dir: "/app/data/projects/hoppscotch".into(), config_files: vec!["/app/data/projects/hoppscotch/compose.yaml".into()] };
    let r = docker::resolve_compose(&c, &mounts, |p| p.exists()).unwrap();
    assert_eq!(r.config_files, vec![vol.join("projects/hoppscotch/compose.yaml").to_string_lossy().into_owned()]);
    assert_eq!(r.working_dir, vol.join("projects/hoppscotch").to_string_lossy().into_owned());
    assert_eq!((r.project.as_str(), r.service.as_str()), ("hoppscotch", "db"));

    // a stack deployed from the host keeps its paths untouched
    std::fs::create_dir_all(host.path().join("stacks/termix")).unwrap();
    std::fs::write(host.path().join("stacks/termix/compose.yaml"), "").unwrap();
    let local = docker::Compose { project: "termix".into(), service: "termix".into(), working_dir: host.path().join("stacks/termix").to_string_lossy().into_owned(), config_files: vec![host.path().join("stacks/termix/compose.yaml").to_string_lossy().into_owned()] };
    assert_eq!(docker::resolve_compose(&local, &mounts, |p| p.exists()).unwrap(), local);

    // nothing on the host: refused before anything changes, with the reason
    let lost = docker::Compose { config_files: vec!["/srv/elsewhere/compose.yaml".into()], working_dir: "/srv/elsewhere".into(), ..c.clone() };
    let e = docker::resolve_compose(&lost, &mounts, |p| p.exists()).unwrap_err().to_string();
    assert!(e.contains("not on this host") && e.contains("nothing was changed"), "{e}");
    // a mount translation never climbs out of the mounted directory
    assert_eq!(docker::host_path("/app/data/../../etc/shadow", &mounts, |p| p.exists() && !p.ends_with("shadow")), None);
}

#[test]
fn container_mounts_are_read_from_docker_inspect() {
    let v = json!({ "Name": "/arcane-agent", "Image": "sha256:ab", "Config": { "Image": "ghcr.io/getarcaneapp/agent:v2.1.0", "Labels": {} },
        "Mounts": [{ "Type": "volume", "Source": "/var/lib/docker/volumes/arcane-agent_arcane-data/_data", "Destination": "/app/data" }] });
    let c = docker::container_from_inspect(&v, vec![]).unwrap();
    assert_eq!(c.mounts, vec![("/var/lib/docker/volumes/arcane-agent_arcane-data/_data".to_string(), "/app/data".to_string())]);
    // restore points saved by older agents (no mounts) still load
    let old: docker::Container = serde_json::from_value(json!({ "name": "x", "image_ref": "a", "image_id": "b", "repo_digests": [], "compose": null })).unwrap();
    assert!(old.mounts.is_empty());
}

fn tags(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_tag_is_compared_only_with_tags_of_the_same_shape() {
    use registry::{newer_version, tag_shape};
    // the shapes of the tags running on the docker hosts
    assert_eq!(tag_shape("v3.3.1"), Some(("v#.#.#".into(), vec![3, 3, 1])));
    assert_eq!(tag_shape("16-vectorchord1.1.1"), Some(("#-vectorchord#.#.#".into(), vec![16, 1, 1, 1])));
    assert_eq!(tag_shape("17.6.1.136"), Some(("#.#.#.#".into(), vec![17, 6, 1, 136])));
    assert_eq!(tag_shape("17-alpine"), Some(("#-alpine".into(), vec![17])));
    assert_eq!(tag_shape("2026.10.12-sha-3699278"), Some(("#.#.#-sha-HASH".into(), vec![2026, 10, 12])), "the build hash is left out of the comparison");
    assert_eq!(tag_shape("v1-sha-xyz"), None, "a word after -sha- is not a hash");
    assert!(tag_shape("latest").is_none() && tag_shape("release").is_none() && tag_shape("").is_none());
    assert!(tag_shape(&format!("v1.{}", "9".repeat(30))).is_none(), "a number that does not fit is not a version");

    let immich = tags(&["v3.3.0", "v3.3.1", "v3.4.0", "v3.4.0-rc.1", "v3.10.0", "v3.9.0", "v2.7.5", "release", "pr-12345", "sha-abc1234", "3.5.0", "v3.5"]);
    let n = newer_version("v3.3.1", &immich).unwrap();
    assert_eq!((n.tag.as_str(), n.major), ("v3.10.0", false), "versions are compared as numbers: 3.10 is above 3.9; a candidate or another shape is ignored");
    assert!(newer_version("v3.10.0", &immich).is_none(), "already the newest");
    assert!(newer_version("v3.3.1", &tags(&["v3.3.1", "v3.3.0", "v3.4.0-rc.1", "latest"])).is_none(), "nothing newer of this shape");
    // a new major is flagged
    let n = newer_version("v2.7.5", &tags(&["v2.7.5", "v2.8.0", "v3.0.0"])).unwrap();
    assert_eq!((n.tag.as_str(), n.major), ("v3.0.0", true));
    let n = newer_version("17-alpine", &tags(&["16-alpine", "17-alpine", "17-bookworm", "18-alpine", "18.1-alpine", "18-bookworm"])).unwrap();
    assert_eq!((n.tag.as_str(), n.major), ("18-alpine", true), "a flavour stays a flavour");
    // pgvecto-style tags: only the part after the flavour moves
    let n = newer_version("16-vectorchord1.1.1", &tags(&["16-vectorchord1.1.1", "16-vectorchord1.2.0", "17-vectorchord1.3.0", "16-vectorchord1.2.0-rc1"])).unwrap();
    assert_eq!(n.tag, "17-vectorchord1.3.0");
    // another numbering scheme next to a single number is not a release
    assert!(newer_version("4", &tags(&["4", "20261111"])).is_none());
    assert_eq!(newer_version("4", &tags(&["4", "5"])).unwrap().tag, "5");
    // a tag with a build hash, or a moving tag, has no version to compare
    assert_eq!(newer_version("2026.10.12-sha-3699278", &tags(&["2026.10.13-sha-aaaaaaa", "2026.11.1", "2026.10.20-sha-bbbbbbb"])).unwrap().tag, "2026.10.20-sha-bbbbbbb", "only tags built the same way are compared");
    assert!(newer_version("2026.10.12-sha-3699278", &tags(&["2026.10.12-sha-aaaaaaa", "2026.10.11-sha-bbbbbbb"])).is_none(), "a rebuild of the same version is not newer");
    assert!(newer_version("latest", &tags(&["v1.0.0"])).is_none());
}

#[test]
fn only_a_next_link_on_the_registry_itself_is_followed() {
    use registry::next_link;
    assert_eq!(next_link("</v2/library/postgres/tags/list?last=17&n=1000>; rel=\"next\"", "registry-1.docker.io").unwrap(), "https://registry-1.docker.io/v2/library/postgres/tags/list?last=17&n=1000");
    assert_eq!(next_link("<https://ghcr.io/v2/a/b/tags/list?last=x>; rel=\"next\"", "ghcr.io").unwrap(), "https://ghcr.io/v2/a/b/tags/list?last=x");
    assert!(next_link("<https://evil.example/v2/a/b/tags/list>; rel=\"next\"", "ghcr.io").is_none(), "another host is ignored");
    assert!(next_link("<//evil.example/x>; rel=\"next\"", "ghcr.io").is_none());
    assert!(next_link("</v2/a/tags/list>; rel=\"prev\"", "ghcr.io").is_none());
    assert_eq!(next_link("</v2/a>; rel=\"prev\", </v2/b>; rel=\"next\"", "localhost:5000").unwrap(), "http://localhost:5000/v2/b", "a registry on this machine speaks plain http");
}

/// A registry on loopback: anonymous token flow, tags served four to a page through a `last=` cursor, as the real ones do.
async fn fake_registry(all_tags: Vec<&'static str>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            let (log, all_tags) = (log.clone(), all_tags.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(path.clone());
                let authed = head.to_lowercase().contains("authorization: bearer t1");
                let (status, extra, body) = if path.starts_with("/token") {
                    ("200 OK", String::new(), r#"{"token":"t1"}"#.to_string())
                } else if !authed {
                    ("401 Unauthorized", format!("www-authenticate: Bearer realm=\"http://127.0.0.1:{port}/token\",service=\"fake\"\r\n"), "{}".to_string())
                } else {
                    let start = path.split("last=").nth(1).and_then(|c| all_tags.iter().position(|t| t == &c.split('&').next().unwrap_or(""))).map_or(0, |i| i + 1);
                    let page: Vec<&str> = all_tags.iter().skip(start).take(4).copied().collect();
                    let more = start + page.len() < all_tags.len();
                    let link = if more { format!("link: </v2/immich/server/tags/list?n=1000&last={}>; rel=\"next\"\r\n", page.last().unwrap()) } else { String::new() };
                    ("200 OK", link, serde_json::json!({ "name": "immich/server", "tags": page }).to_string())
                };
                let _ = s.write_all(format!("HTTP/1.1 {status}\r\n{extra}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            });
        }
    });
    (format!("127.0.0.1:{port}"), seen)
}

fn tag_pages(seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> usize {
    seen.lock().unwrap().iter().filter(|p| p.contains("/tags/list")).count()
}

#[tokio::test]
async fn a_registry_that_lists_tags_in_no_order_is_read_to_the_end() {
    // like GHCR: the newest versions can be on any page
    let (host, seen) = fake_registry(vec!["release", "v3.10.0", "pr-5", "v3.3.0", "v3.3.1", "v3.4.0-rc.1", "v3.9.0", "latest", "sha-abc1234", "v3.4.0", "nightly", "v2.7.5"]).await;
    let img = registry::parse(&format!("{host}/immich/server:v3.3.1")).unwrap();
    let kept = registry::list_tags(&img).await.unwrap();
    let mut sorted = kept.clone();
    sorted.sort();
    assert_eq!(sorted, tags(&["v2.7.5", "v3.10.0", "v3.3.0", "v3.3.1", "v3.4.0", "v3.9.0"]), "only tags of the shape v#.#.# are kept");
    assert_eq!(registry::newer_version(&img.tag, &kept).unwrap().tag, "v3.10.0", "3.10 is above 3.9");
    assert_eq!(tag_pages(&seen), 4, "denied once, then three pages of four: {:?}", seen.lock().unwrap());
    assert_eq!(seen.lock().unwrap().iter().filter(|p| p.starts_with("/token")).count(), 1, "one token for the whole listing");

    // the inventory entry carries it, next to the digest the running tag tracks
    let c = docker::Container { name: "immich_server".into(), image_ref: format!("{host}/immich/server:v3.3.1"), image_id: "sha256:aaaa".into(), repo_digests: vec![], compose: None, mounts: vec![], state: "running".into(), health: String::new(), started_at: String::new() };
    let n = docker::newer_version(&img).await;
    assert_eq!(n.as_ref().unwrap().tag, "v3.10.0");
    let e = docker::entry_with(&c, None, n.as_ref(), None);
    assert_eq!(e.meta["latest"], serde_json::json!({ "tag": "v3.10.0", "major": false, "same_major": null }));
    assert!(e.candidate.is_none(), "a newer version is information: it is not an update this agent can apply");
    assert!(docker::entry_with(&c, None, None, None).meta.get("latest").is_none());
    // the hint of whether the suggestion can be applied, and by changing which file, rides along
    let hint = serde_json::json!({ "mode": "file", "file": "/opt/stacks/immich/compose.yaml" });
    let e = docker::entry_with(&c, None, n.as_ref(), Some(hint.clone()));
    assert_eq!(e.meta["latest"]["apply"], hint);
    assert_eq!(e.meta["latest"]["tag"], "v3.10.0");
    // asked once: the answer is kept for hours
    let before = seen.lock().unwrap().len();
    assert_eq!(docker::newer_version(&img).await.unwrap().tag, "v3.10.0");
    assert_eq!(seen.lock().unwrap().len(), before, "served from memory");
    // a moving tag is never looked up
    let moving = registry::parse(&format!("{host}/immich/server:release")).unwrap();
    assert!(docker::newer_version(&moving).await.is_none());
    assert_eq!(seen.lock().unwrap().len(), before);
}

#[tokio::test]
async fn an_alphabetical_registry_is_not_read_past_the_tags_it_is_looking_for() {
    // like Docker Hub: sorted, so once a page is past the `v` tags the rest cannot matter
    let (host, seen) = fake_registry(vec!["latest", "pr-1", "release", "sha-abc1234", "v3.10.0", "v3.3.0", "v3.3.1", "v3.4.0", "v3.4.0-rc.1", "v3.9.0", "w-1", "x-1", "y-1", "y-2", "z-1", "z-2"]).await;
    let img = registry::parse(&format!("{host}/immich/server:v3.3.1")).unwrap();
    let kept = registry::list_tags(&img).await.unwrap();
    assert_eq!(registry::newer_version(&img.tag, &kept).unwrap().tag, "v3.10.0");
    assert_eq!(tag_pages(&seen), 4, "denied once, then three pages: the fourth (y, z) is not read: {:?}", seen.lock().unwrap());
    // a tag that starts with a digit lives at the start of an alphabetical list
    let (host, seen) = fake_registry(vec!["10", "17-alpine", "18-alpine", "18.1", "alpine", "bookworm", "latest", "stable"]).await;
    let img = registry::parse(&format!("{host}/immich/server:17-alpine")).unwrap();
    let kept = registry::list_tags(&img).await.unwrap();
    assert_eq!(registry::newer_version(&img.tag, &kept).unwrap().tag, "18-alpine");
    assert_eq!(tag_pages(&seen), 3, "denied once, then two pages: stopped at the first letter: {:?}", seen.lock().unwrap());
}

#[test]
fn a_new_major_also_names_the_newest_of_the_running_major() {
    let n = registry::newer_version("16-vectorchord1.1.1", &tags(&["16-vectorchord1.1.1", "16-vectorchord1.2.0", "18-vectorchord1.1.1"])).unwrap();
    assert_eq!((n.tag.as_str(), n.major, n.same_major.as_deref()), ("18-vectorchord1.1.1", true, Some("16-vectorchord1.2.0")));
    let n = registry::newer_version("v2.5.0", &tags(&["v2.5.1", "v2.6.0", "v3.0.0"])).unwrap();
    assert_eq!((n.tag.as_str(), n.same_major.as_deref()), ("v3.0.0", Some("v2.6.0")));
    let n = registry::newer_version("v2.5.0", &tags(&["v2.5.1", "v2.6.0"])).unwrap();
    assert_eq!((n.tag.as_str(), n.major, n.same_major), ("v2.6.0", false, None), "nothing more to say when the newest is in the same major");
    assert_eq!(registry::tag_prefix("v3.3.1"), "v");
    assert_eq!(registry::tag_prefix("17-alpine"), "");
}

#[test]
fn an_aur_update_is_read_from_the_rpc_answer_and_names_are_checked() {
    use croustypatch_agent::modules::aur::{parse_version, valid_name};
    let rpc = serde_json::json!({ "resultcount": 1, "results": [{ "Name": "visual-studio-code-bin", "Version": "1.146.0-1" }], "type": "multiinfo", "version": 5 });
    assert_eq!(parse_version(&rpc).as_deref(), Some("1.146.0-1"));
    assert_eq!(parse_version(&serde_json::json!({ "resultcount": 0, "results": [] })), None, "a package that is not in the AUR");
    assert_eq!(parse_version(&serde_json::json!({ "results": [{ "Version": "" }] })), None);
    assert!(valid_name("visual-studio-code-bin") && valid_name("libc++-git") && !valid_name("") && !valid_name("-x") && !valid_name("a b") && !valid_name("a;b") && !valid_name("A") && !valid_name("../x"));
}

#[tokio::test]
async fn the_app_list_says_when_a_command_app_has_a_newer_version() {
    // an app with no version source, or that is not installed, is left exactly as it was
    let mut v = serde_json::json!({ "apps": [
        { "name": "Plain", "command": { "install": "x", "uninstall": "y", "check": "/usr/bin/x" }, "local": { "present": 1, "total": 1, "missing": [] } },
        { "name": "Absent", "command": { "install": "x", "uninstall": "y", "check": "/usr/bin/y", "latest": { "aur": "visual-studio-code-bin" } }, "local": { "present": 0, "total": 1, "missing": ["/usr/bin/y"] } },
        { "name": "Packages", "packages": ["git"], "local": { "present": 1, "total": 1, "missing": [] } },
    ] });
    let before = v.clone();
    croustypatch_agent::portal::annotate_updates(&mut v).await;
    assert_eq!(v, before, "no lookup, no change");
}

#[tokio::test]
async fn the_newest_release_is_found_by_asking_for_the_versions_after_the_running_one() {
    use registry::probe_newer;
    use std::collections::HashSet;
    let has = |set: &[&str]| -> HashSet<String> { set.iter().map(|s| s.to_string()).collect() };
    // immich-machine-learning: a tag per commit, more than 100 pages of them; only the releases matter
    let published = has(&["v3.3.1", "v3.3.2", "v3.3.3", "v3.3.4", "v3.3.5", "v3.3.6", "v3.4.0", "v3.4.1"]);
    let mut asked = 0;
    let n = probe_newer("v3.3.1", |t| {
        asked += 1;
        let ok = published.contains(&t);
        async move { Ok(ok) }
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(n.tag, "v3.4.1", "the walk goes through v3.3.4 and v3.3.6 to v3.4.1");
    assert!(!n.major && n.same_major.is_none());
    assert!(asked < 60, "a few dozen requests at most, asked {asked}");
    // a new major, and the newest of the running one
    let published = has(&["v2.5.0", "v2.5.1", "v2.6.0", "v3.0.0", "v3.1.0"]);
    let n = probe_newer("v2.5.0", |t| { let ok = published.contains(&t); async move { Ok(ok) } }).await.unwrap().unwrap();
    assert_eq!((n.tag.as_str(), n.major, n.same_major.as_deref()), ("v3.1.0", true, Some("v2.6.0")));
    // two numbers (envoy v1.39.1 has no v1.39.2 yet but v1.40.0), a leading zero kept, a suffix after the numbers
    assert_eq!(probe_newer("v1.39.1", |t| async move { Ok(t == "v1.40.0") }).await.unwrap().unwrap().tag, "v1.40.0");
    assert_eq!(probe_newer("17.6.1.019", |t| async move { Ok(t == "17.6.1.020") }).await.unwrap().unwrap().tag, "17.6.1.020");
    assert_eq!(probe_newer("16-vectorchord1.1.1", |t| async move { Ok(t == "17-vectorchord1.1.1") }).await.unwrap().unwrap().tag, "17-vectorchord1.1.1", "numbers after the raised one can stay");
    assert_eq!(probe_newer("9.19", |t| async move { Ok(t == "9.20") }).await.unwrap().unwrap().tag, "9.20");
    // nothing newer, a registry that fails, and tags that cannot be followed
    assert!(probe_newer("v3.4.1", |_| async { Ok(false) }).await.unwrap().is_none());
    assert!(probe_newer("v3.4.1", |_| async { anyhow::bail!("registry answered 500") }).await.is_err(), "a failing registry is not 'up to date'");
    let found = std::cell::Cell::new(false);
    let n = probe_newer("v3.4.1", |t| {
        let r = if found.get() { Err(anyhow::anyhow!("registry answered 429")) } else { Ok(t == "v3.4.2") };
        if t == "v3.4.2" {
            found.set(true);
        }
        async move { r }
    })
    .await;
    assert_eq!(n.unwrap().unwrap().tag, "v3.4.2", "a failure after a find keeps the find");
    assert!(probe_newer("2026.10.12-sha-3699278", |_| async { Ok(true) }).await.is_err());
    assert!(probe_newer("latest", |_| async { Ok(true) }).await.is_err());
    // the number of requests is bounded even when every tag exists
    let mut count = 0;
    let _ = probe_newer("v1.0.0", |_| { count += 1; async { Ok(true) } }).await;
    assert!(count <= 90, "{count}");
}

#[test]
fn private_registries_use_the_login_docker_recorded() {
    use base64::Engine;
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    let cfg = format!(r#"{{"auths": {{"https://index.docker.io/v1/": {{"auth": "{}"}}, "registry.lan:5000": {{"auth": "{}"}}, "ghcr.io": {{"username": "bot", "password": "tok"}}, "empty.example": {{}}}}, "credsStore": "pass"}}"#, b64("hubuser:hubpass"), b64("me:se:cret"));
    let creds = registry::credentials_from_config;
    assert_eq!(creds(&cfg, "registry-1.docker.io"), Some(("hubuser".into(), "hubpass".into())), "Docker Hub is recorded under its index address");
    assert_eq!(creds(&cfg, "registry.lan:5000"), Some(("me".into(), "se:cret".into())), "a colon in the password stays in the password");
    assert_eq!(creds(&cfg, "ghcr.io"), Some(("bot".into(), "tok".into())));
    assert_eq!(creds(&cfg, "quay.io"), None);
    assert_eq!(creds(&cfg, "empty.example"), None);
    assert_eq!(creds("not json", "ghcr.io"), None);
}
