//! App-center command recipes (usable by the asking user), self-update order checks, protocol compatibility.
use croustypatch_agent::app_command::{self, mode_allows, usable_by, Target};
use croustypatch_agent::exec::Runner;
use croustypatch_agent::runner::TaskLog;
use croustypatch_agent::selfupdate::check_order;
use croustypatch_common::protocol::{AppCommand, CommandAction, TelemetryResponse};
use std::os::unix::fs::PermissionsExt;

fn user(uid: u32, groups: &[u32]) -> Target {
    Target { name: format!("u{uid}"), uid, gid: groups.first().copied().unwrap_or(uid), groups: groups.to_vec(), home: format!("/home/u{uid}") }
}

#[test]
fn permission_bits_are_read_for_owner_group_and_others() {
    let alice = user(1000, &[1000, 50]);
    assert!(mode_allows(0o700, 1000, 0, &alice, 7), "owner");
    assert!(!mode_allows(0o700, 0, 0, &alice, 1), "root-only");
    assert!(mode_allows(0o750, 0, 50, &alice, 5), "through a supplementary group");
    assert!(!mode_allows(0o750, 0, 51, &alice, 1), "not in that group");
    assert!(mode_allows(0o755, 0, 0, &Target::everyone(), 5), "world readable and searchable");
    assert!(!mode_allows(0o750, 0, 0, &Target::everyone(), 1));
    assert!(mode_allows(0o000, 0, 0, &user(0, &[0]), 7), "root may do anything");
}

#[test]
fn an_app_installed_for_root_only_is_not_usable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let bin = dir.path().join("tool");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let everyone = Target::everyone();
    // The temp dir's parents (/tmp…) are world-searchable on any normal system.
    assert_eq!(usable_by(&bin, &everyone), Ok(()));
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let e = usable_by(&bin, &everyone).unwrap_err();
    assert!(e.contains("cannot run"), "{e}");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let e = usable_by(&bin, &everyone).unwrap_err();
    assert!(e.contains("cannot open the directory"), "a locked parent directory blocks it: {e}");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(usable_by(&dir.path().join("missing"), &everyone).unwrap_err().contains("does not exist"));
    let data = dir.path().join("data.txt");
    std::fs::write(&data, "x").unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(usable_by(&data, &everyone), Ok(()), "a plain file only needs to be readable");
}

#[test]
fn the_command_gets_the_user_only_as_environment() {
    let alice = user(1000, &[1000]);
    let c = app_command::command("echo hi", &alice);
    assert_eq!(c.program, "sh");
    assert_eq!(c.args, vec!["-c".to_string(), "umask 022\necho hi".to_string()], "the script is never built from user input");
    assert!(c.env.contains(&("CROUSTYPATCH_USER", "u1000".to_string())));
    assert!(c.env.contains(&("CROUSTYPATCH_USER_HOME", "/home/u1000".to_string())));
    assert!(c.env.contains(&("CROUSTYPATCH_USER_UID", "1000".to_string())));
    let web = app_command::command("echo hi", &Target::everyone());
    assert!(!web.env.iter().any(|(k, _)| k.starts_with("CROUSTYPATCH_USER")), "a web request names nobody");
    assert_eq!(c.timeout.as_secs(), app_command::TIMEOUT_SECS);
}

#[tokio::test]
async fn install_is_checked_against_the_asking_user() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.path().join("bin");
    let real = Runner { dry_run: false };
    let log = TaskLog::default();
    let cmd = |install: &str| AppCommand { install: install.into(), uninstall: format!("rm -f {}", path.display()), check: path.display().to_string() };

    // umask 022: what the command creates is usable by everyone
    let ok = cmd(&format!("printf '#!/bin/sh\\n' > {p}; chmod +x {p}", p = path.display()));
    let msg = app_command::run(&real, &ok, true, &Target::everyone(), &log).await.unwrap();
    assert!(msg.contains("usable by every user"), "{msg}");

    // explicitly locked down: reported as a failure
    let locked = cmd(&format!("printf '#!/bin/sh\\n' > {p}; chmod 700 {p}", p = path.display()));
    let e = app_command::run(&real, &locked, true, &Target::everyone(), &log).await.unwrap_err().to_string();
    assert!(e.contains("not usable"), "{e}");

    let failing = cmd("echo boom >&2; exit 3");
    let e = app_command::run(&real, &failing, true, &Target::everyone(), &log).await.unwrap_err().to_string();
    assert!(e.contains("exit 3") && e.contains("boom"), "{e}");

    let msg = app_command::run(&real, &ok, false, &Target::everyone(), &log).await.unwrap();
    assert_eq!(msg, "uninstalled");
    assert!(!path.exists());
    let leaves = AppCommand { install: "true".into(), uninstall: "true".into(), check: dir.path().display().to_string() };
    assert!(app_command::run(&real, &leaves, false, &Target::everyone(), &log).await.unwrap_err().to_string().contains("still there"));

    let dry = Runner { dry_run: true };
    assert!(app_command::run(&dry, &locked, true, &Target::everyone(), &log).await.unwrap().contains("dry-run"));
}

#[tokio::test]
async fn local_accounts_are_looked_up_through_the_name_service() {
    let me = std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|l| l.starts_with("Uid:")).and_then(|l| l.split_whitespace().nth(1)).unwrap().parse::<u32>().unwrap();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    let Some(name) = passwd.lines().find(|l| l.split(':').nth(2) == Some(&me.to_string())).map(|l| l.split(':').next().unwrap().to_string()) else { return };
    let t = app_command::lookup(&name).await.unwrap();
    assert_eq!((t.name.as_str(), t.uid), (name.as_str(), me));
    assert!(t.groups.contains(&t.gid), "the primary group is always among the groups");
    assert!(t.home.starts_with('/'), "{}", t.home);
    assert!(app_command::lookup("no-such-user-cp").await.is_err());
    assert!(app_command::lookup("-rf").await.is_err(), "never passed to getent as an option");
}

#[test]
fn self_update_orders_are_validated() {
    let sum = "a".repeat(64);
    assert!(check_order("linux-x86_64", &sum, "0.4.1").is_ok());
    assert!(check_order("linux-mips", &sum, "0.4.1").is_err());
    assert!(check_order("linux-x86_64", "abc", "0.4.1").is_err());
    assert!(check_order("linux-x86_64", &sum, "1.0; rm -rf /").is_err());
}

#[test]
fn messages_stay_readable_across_versions() {
    // an order from a server before command recipes still parses
    let a: CommandAction = serde_json::from_str(r#"{"type":"app","install":true,"packages":["htop"]}"#).unwrap();
    assert!(matches!(a, CommandAction::App { command: None, user: None, .. }));
    // a package order does not carry the new fields at all, so older agents read it unchanged
    let s = serde_json::to_string(&CommandAction::App { install: true, packages: vec!["htop".into()], command: None, user: None }).unwrap();
    assert_eq!(s, r#"{"type":"app","install":true,"packages":["htop"]}"#);
    let t: TelemetryResponse = serde_json::from_str(r#"{"commands":[],"heartbeat_secs":60}"#).unwrap();
    assert!(t.settings.is_none());
}
