//! Optional modules: image references, registry auth challenge, container inventory, OpenSCAP results, helper scripts.
use croustypatch_agent::modules::{docker, helper_scripts, openscap, registry};
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
fn openscap_results_are_summarised() {
    let xml = r#"<TestResult>
      <rule-result idref="xccdf_org.ssgproject.content_rule_a" severity="high" time="t"><result>pass</result></rule-result>
      <rule-result idref="xccdf_org.ssgproject.content_rule_b" severity="high"><result>fail</result><ident>x</ident></rule-result>
      <rule-result idref="xccdf_org.ssgproject.content_rule_c" severity="low"><result>fail</result></rule-result>
      <rule-result idref="xccdf_org.ssgproject.content_rule_d" severity="medium"><result>notapplicable</result></rule-result>
      <rule-result idref="xccdf_org.ssgproject.content_rule_e" severity="medium"><result>pass</result></rule-result>
      <rule-result idref="xccdf_org.ssgproject.content_rule_f" severity="medium"><result>notchecked</result></rule-result>
    </TestResult>"#;
    let (pass, fail, na, other, failed) = openscap::summarize(xml);
    assert_eq!((pass, fail, na, other), (2, 2, 1, 1));
    assert_eq!(failed[0].rule_id, "xccdf_org.ssgproject.content_rule_b");
    assert_eq!(failed[0].severity, "high");
    assert_eq!(openscap::score(pass, fail), 50.0);
    assert_eq!(openscap::score(0, 0), 0.0);

    let dir = tempfile::tempdir().unwrap();
    for f in ["ssg-debian12-ds.xml", "ssg-ubuntu2404-ds.xml", "other.xml"] {
        std::fs::write(dir.path().join(f), "x").unwrap();
    }
    assert!(openscap::find_datastream(dir.path(), "ubuntu2404").unwrap().ends_with("ssg-ubuntu2404-ds.xml"));
    assert!(openscap::find_datastream(dir.path(), "arch").unwrap().ends_with("ssg-debian12-ds.xml"), "falls back to any SSG datastream");

    // A release without content of its own gets the newest one of its distribution, not the first file in the directory.
    let dir = tempfile::tempdir().unwrap();
    for f in ["ssg-debian11-ds.xml", "ssg-debian12-ds.xml", "ssg-ubuntu2204-ds.xml", "ssg-ubuntu2404-ds.xml"] {
        std::fs::write(dir.path().join(f), "x").unwrap();
    }
    assert!(openscap::find_datastream(dir.path(), "debian13").unwrap().ends_with("ssg-debian12-ds.xml"));
    assert!(openscap::find_datastream(dir.path(), "ubuntu26").unwrap().ends_with("ssg-ubuntu2404-ds.xml"));
    assert!(openscap::find_datastream(dir.path(), "ubuntu22").unwrap().ends_with("ssg-ubuntu2204-ds.xml"));
    assert!(openscap::find_datastream(dir.path(), "").is_some());
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

#[test]
fn openscap_profiles_and_version_are_read_from_oscap_output() {
    use croustypatch_agent::modules::openscap::{parse_profiles, parse_version, valid_id};
    let out = "xccdf_org.ssgproject.content_profile_anssi_bp28_enhanced:ANSSI-BP-028 (enhanced)\nxccdf_org.ssgproject.content_profile_cis:CIS Debian Linux 12 Benchmark for Level 2: Server\nnot a profile line\n:empty\nxccdf_bad id:x\n";
    let p = parse_profiles(out);
    assert_eq!(p.len(), 2, "{p:?}");
    assert_eq!((p[1].id.as_str(), p[1].title.as_str()), ("xccdf_org.ssgproject.content_profile_cis", "CIS Debian Linux 12 Benchmark for Level 2: Server"), "a colon inside the title is kept");
    assert_eq!(parse_version("OpenSCAP command line tool (oscap) 1.4.2\nCopyright 2009--2023 Red Hat Inc., Durham, North Carolina.\n"), "1.4.2");
    assert_eq!(parse_version("garbage"), "");
    assert!(valid_id("xccdf_org.ssgproject.content_rule_audit_rules-1") && !valid_id("a b") && !valid_id("x;rm") && !valid_id("") && !valid_id(&"a".repeat(301)));
}

#[test]
fn a_custom_profile_becomes_an_xccdf_tailoring_file() {
    use croustypatch_agent::modules::openscap::{tailoring_profile_id, tailoring_xml};
    use croustypatch_common::protocol::Tailoring;
    let t = Tailoring { id: "3f2a-9b".into(), name: "Serveurs <prod> & \"co\"".into(), base: "xccdf_org.ssgproject.content_profile_cis".into(), disabled_rules: vec!["xccdf_org.ssgproject.content_rule_a".into(), "xccdf_org.ssgproject.content_rule_b".into()] };
    assert_eq!(tailoring_profile_id(&t), "xccdf_org.croustypatch_profile_3f2a_9b");
    let x = tailoring_xml(&t, "/usr/share/xml/scap/ssg/content/ssg-debian13-ds.xml", "2026-11-11T10:00:00");
    assert!(x.contains("<xccdf-1.2:benchmark href=\"/usr/share/xml/scap/ssg/content/ssg-debian13-ds.xml\"/>"), "{x}");
    assert!(x.contains("id=\"xccdf_org.croustypatch_profile_3f2a_9b\" extends=\"xccdf_org.ssgproject.content_profile_cis\""), "{x}");
    assert!(x.contains("<xccdf-1.2:title>Serveurs &lt;prod&gt; &amp; &quot;co&quot;</xccdf-1.2:title>"), "names are escaped: {x}");
    assert_eq!(x.matches("selected=\"false\"").count(), 2);
    assert!(x.contains("idref=\"xccdf_org.ssgproject.content_rule_a\"") && x.ends_with("</xccdf-1.2:Tailoring>\n"));
    if let Ok(path) = std::env::var("TAILORING_OUT") {
        std::fs::write(path, x).unwrap(); // used by the manual check against a real oscap
    }
}

#[test]
fn what_to_install_depends_on_the_system() {
    use croustypatch_agent::modules::openscap::{install_hint, packages_for};
    use croustypatch_common::model::PackageManager::*;
    assert_eq!(packages_for(Apt, "Debian GNU/Linux").unwrap().0, vec!["openscap-scanner", "ssg-debian"]);
    assert_eq!(packages_for(Apt, "Ubuntu").unwrap().0, vec!["openscap-scanner", "ssg-debderived"]);
    let (arch, note) = packages_for(Pacman, "Arch Linux").unwrap();
    assert_eq!(arch, vec!["openscap"]);
    assert!(note.unwrap().contains("AUR"), "Arch has no SCAP content in its repositories and says so");
    assert!(packages_for(Pkg, "FreeBSD").is_none() && packages_for(Dnf, "Fedora").is_none());
    assert!(install_hint(Apt, "Debian GNU/Linux").contains("ssg-debian") && !install_hint(Apt, "Debian").contains("dnf"), "the hint follows the host's package manager");
    assert!(install_hint(Pacman, "Arch").contains("pacman"));
}

#[test]
fn openscap_is_installed_before_an_audit_only_when_it_is_missing_and_allowed() {
    use croustypatch_agent::modules::openscap::{install_plan, InstallPlan};
    use croustypatch_common::model::PackageManager::*;
    let debian = |have_oscap, have_content, auto| install_plan(have_oscap, have_content, auto, Apt, "Debian GNU/Linux");
    assert_eq!(debian(true, true, true), InstallPlan::Nothing, "already there");
    assert_eq!(debian(false, false, false), InstallPlan::Nothing, "the administrator switched automatic installation off: the audit explains what to install");
    assert_eq!(debian(false, false, true), InstallPlan::Install { packages: vec!["openscap-scanner", "ssg-debian"], note: None });
    assert_eq!(debian(true, false, true), InstallPlan::Install { packages: vec!["openscap-scanner", "ssg-debian"], note: None }, "scanner without content: the content is installed too");
    assert!(matches!(install_plan(false, false, true, Apt, "Ubuntu"), InstallPlan::Install { packages, .. } if packages == vec!["openscap-scanner", "ssg-debderived"]));
    // Arch: only the scanner is in the repositories, and the run says where the rest comes from
    match install_plan(false, false, true, Pacman, "Arch Linux") {
        InstallPlan::Install { packages, note } => {
            assert_eq!(packages, vec!["openscap"]);
            assert!(note.unwrap().contains("AUR"));
        }
        other => panic!("{other:?}"),
    }
    // nothing to install on a system without packages for it (FreeBSD, the mock backend of the tests)
    assert_eq!(install_plan(false, false, true, Pkg, "FreeBSD"), InstallPlan::Nothing);
    assert_eq!(install_plan(false, false, true, Unknown, ""), InstallPlan::Nothing);
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
    assert_eq!(tag_shape("2026.10.12-sha-3699278"), None, "a build hash is not a version");
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
    assert!(newer_version("2026.10.12-sha-3699278", &tags(&["2026.10.13-sha-aaaaaaa", "2026.11.1"])).is_none());
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
    let e = docker::entry_with(&c, None, n.as_ref());
    assert_eq!(e.meta["latest"], serde_json::json!({ "tag": "v3.10.0", "major": false, "same_major": null }));
    assert!(e.candidate.is_none(), "a newer version is information: it is not an update this agent can apply");
    assert!(docker::entry_with(&c, None, None).meta.get("latest").is_none());
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
