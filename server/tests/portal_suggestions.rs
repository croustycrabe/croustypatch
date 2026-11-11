//! The shipped list of suggested apps must be valid catalog entries (no database needed).
use croustypatch_server::api::portal::check_recipes;
use croustypatch_server::icons;
use serde_json::Value;

/// Suggestions that install with a command, because the software is in no distribution's own repositories (a vendor
/// repository, an official archive). They are a deliberate, reviewed exception: adding one needs the portal:commands
/// permission and the interface says so. A new one has to be named here, after its scripts were read and run on a clean
/// Debian and Ubuntu.
const REVIEWED_COMMANDS: &[&str] = &["Visual Studio Code", "Zed"];

#[test]
fn every_suggestion_is_a_valid_catalog_app() {
    let list: Vec<Value> = serde_json::from_str(include_str!("../assets/app-suggestions.json")).unwrap();
    assert!(list.len() >= 30);
    let mut names = std::collections::HashSet::new();
    for s in &list {
        let name = s["name"].as_str().unwrap();
        assert!(names.insert(name.to_lowercase()), "duplicate {name}");
        assert!(icons::check(s["icon"].as_str().unwrap()).is_ok(), "{name}: icon");
        assert!(s["description"].as_str().unwrap().len() <= 2000 && s["category"].as_str().unwrap().len() <= 40, "{name}");
        let has_command = check_recipes(&s["recipes"]).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(has_command, REVIEWED_COMMANDS.contains(&name), "{name}: packages only, unless it is a reviewed command app");
        for pm in ["apt", "pacman"] {
            assert!(s["recipes"].get(pm).is_some(), "{name}: both Debian and Arch are covered");
        }
    }
    for n in REVIEWED_COMMANDS {
        assert!(names.contains(&n.to_lowercase()), "{n} is listed as a reviewed command app but is not a suggestion");
    }
}

#[test]
fn the_editors_are_offered_on_debian_and_on_arch_in_the_way_each_system_can() {
    let list: Vec<Value> = serde_json::from_str(include_str!("../assets/app-suggestions.json")).unwrap();
    let app = |n: &str| list.iter().find(|s| s["name"] == n).unwrap_or_else(|| panic!("{n} is not suggested")).clone();
    // neither is in Debian's or Ubuntu's own repositories: a command (a signed vendor repository, an official archive)
    let (code, zed) = (app("Visual Studio Code"), app("Zed"));
    assert_eq!(code["recipes"]["apt"]["command"]["check"], "/usr/bin/code");
    assert_eq!(zed["recipes"]["apt"]["command"]["check"], "/usr/local/bin/zed");
    // Arch: Microsoft's build is AUR-only, so the repository's Code - OSS is what is installed, and the entry says so
    assert_eq!(code["recipes"]["pacman"]["packages"], serde_json::json!(["code"]));
    let say = code["description"].as_str().unwrap();
    assert!(say.contains("AUR") && say.contains("Code - OSS"), "{say}");
    assert_eq!(zed["recipes"]["pacman"]["packages"], serde_json::json!(["zed"]));
    assert_eq!(zed["icon"], "brand:zedindustries", "Zed has a logo; VS Code has none in the bundled set and keeps the generic glyph");
    assert_eq!(code["icon"], "code");
}

#[test]
fn the_command_suggestions_are_careful_scripts() {
    let list: Vec<Value> = serde_json::from_str(include_str!("../assets/app-suggestions.json")).unwrap();
    let mut seen = 0;
    for s in list.iter().filter(|s| REVIEWED_COMMANDS.contains(&s["name"].as_str().unwrap())) {
        let name = s["name"].as_str().unwrap();
        for (system, r) in s["recipes"].as_object().unwrap() {
            let Some(c) = r.get("command") else { continue };
            seen += 1;
            for what in ["install", "uninstall"] {
                let script = c[what].as_str().unwrap();
                assert!(script.starts_with("set -eu\n"), "{name} {system} {what}: stops at the first error");
                assert!(!script.contains("| sh") && !script.contains("| bash") && !script.contains("sudo "), "{name} {what}: nothing is piped into a shell");
                assert!(!script.contains("http://"), "{name} {what}: downloads use https");
            }
            let install = c["install"].as_str().unwrap();
            assert!(install.contains("https://"), "{name}: it downloads from the vendor over https");
            if install.contains("tar -x") {
                assert!(install.contains("--no-same-owner") && install.contains("chown -R root:root"), "{name}: an archive unpacked as root must not keep its owners");
                assert!(install.contains("tar -tzf"), "{name}: the archive is read completely before anything is replaced");
            }
            let check = c["check"].as_str().unwrap();
            assert!(check.starts_with("/usr/") && !check.contains(".."), "{name}: check is a system path every user can run");
        }
    }
    assert_eq!(seen, REVIEWED_COMMANDS.len());
}

#[test]
fn only_known_builds_can_be_downloaded() {
    use croustypatch_server::api::agent_dist::artifact_file;
    assert_eq!(artifact_file("linux-x86_64").as_deref(), Some("croustypatch-agent-linux-x86_64"));
    assert_eq!(artifact_file("portal-gui-linux-x86_64").as_deref(), Some("croustypatch-portal-gui-linux-x86_64"));
    assert_eq!(artifact_file("portal-gui-icon.png").as_deref(), Some("croustypatch-portal-gui.png"));
    for bad in ["portal-gui-freebsd-amd64", "portal-gui-../../etc/passwd", "../server.key", "portal-gui-", "croustypatch-agent-linux-x86_64"] {
        assert_eq!(artifact_file(bad), None, "{bad}");
    }
}
