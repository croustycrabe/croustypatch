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
