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
