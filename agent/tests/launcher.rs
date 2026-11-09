//! `croustypatch-portal`: desktop window or terminal; machine type detection.
use croustypatch_agent::facts::{desktop_detected_in, machine_type};
use croustypatch_agent::launcher::{choose, force_from_args, has_display, Client, Force};

#[test]
fn the_window_opens_only_in_a_graphical_session_with_the_app_installed() {
    assert_eq!(choose(Force::Auto, true, true), Client::Gui);
    assert_eq!(choose(Force::Auto, false, true), Client::Tui, "over SSH on a laptop: terminal");
    assert_eq!(choose(Force::Auto, true, false), Client::Tui, "not installed (server, FreeBSD, aarch64)");
    assert_eq!(choose(Force::Tui, true, true), Client::Tui);
    assert_eq!(choose(Force::Gui, false, true), Client::Gui, "--gui is obeyed");
    assert_eq!(choose(Force::Gui, true, false), Client::Tui, "--gui without the app falls back");
}

#[test]
fn flags_and_display_variables_are_read() {
    assert_eq!(force_from_args(["--gui".to_string()]), Force::Gui);
    assert_eq!(force_from_args(["--gui".to_string(), "--tui".to_string()]), Force::Tui, "the last one wins");
    assert_eq!(force_from_args(Vec::<String>::new()), Force::Auto);
    assert!(has_display(|k| (k == "WAYLAND_DISPLAY").then(|| "wayland-1".to_string())));
    assert!(has_display(|k| (k == "DISPLAY").then(|| ":0".to_string())));
    assert!(!has_display(|k| (k == "DISPLAY").then(|| " ".to_string())));
    assert!(!has_display(|_| None));
}

#[test]
fn a_desktop_is_recognised_by_its_display_manager_or_sessions() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    assert!(!desktop_detected_in(r), "an empty system is a server");
    std::fs::create_dir_all(r.join("usr/share/xsessions")).unwrap();
    assert!(!desktop_detected_in(r), "an empty sessions directory proves nothing");
    std::fs::write(r.join("usr/share/xsessions/README"), "").unwrap();
    assert!(!desktop_detected_in(r));
    std::fs::create_dir_all(r.join("usr/share/wayland-sessions")).unwrap();
    std::fs::write(r.join("usr/share/wayland-sessions/plasma.desktop"), "").unwrap();
    assert!(desktop_detected_in(r), "a Wayland session");
    let dm = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dm.path().join("etc/systemd/system")).unwrap();
    std::fs::write(dm.path().join("etc/systemd/system/display-manager.service"), "").unwrap();
    assert!(desktop_detected_in(dm.path()), "a display manager");
    let bsd = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(bsd.path().join("usr/local/share/xsessions")).unwrap();
    std::fs::write(bsd.path().join("usr/local/share/xsessions/xfce.desktop"), "").unwrap();
    assert!(desktop_detected_in(bsd.path()), "FreeBSD ports layout");
}

#[test]
fn the_machine_type_prefers_laptop_then_workstation() {
    assert_eq!(machine_type(true, true), "laptop");
    assert_eq!(machine_type(false, true), "workstation");
    assert_eq!(machine_type(false, false), "server");
}
