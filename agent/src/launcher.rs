//! `croustypatch-portal`: opens the desktop app center when the user sits at a graphical session and it is installed
//! (laptops and workstations get it from the installer), the terminal one otherwise, for example over SSH.
//! `--gui` / `--tui` force the choice.
use crate::facts::PORTAL_GUI;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Force {
    Auto,
    Gui,
    Tui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Gui,
    Tui,
}

/// `--gui` / `--tui` among the arguments (the last one wins).
pub fn force_from_args<I: IntoIterator<Item = String>>(args: I) -> Force {
    args.into_iter().fold(Force::Auto, |f, a| match a.as_str() {
        "--gui" => Force::Gui,
        "--tui" => Force::Tui,
        _ => f,
    })
}

/// A graphical session is reachable from this process.
pub fn has_display(var: impl Fn(&str) -> Option<String>) -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"].iter().any(|k| var(k).is_some_and(|v| !v.trim().is_empty()))
}

pub fn choose(force: Force, display: bool, gui_installed: bool) -> Client {
    match force {
        Force::Tui => Client::Tui,
        Force::Gui if gui_installed => Client::Gui,
        Force::Gui => Client::Tui,
        Force::Auto if display && gui_installed => Client::Gui,
        Force::Auto => Client::Tui,
    }
}

/// The decision for this process.
pub fn decide(force: Force) -> Client {
    choose(force, has_display(|k| std::env::var(k).ok()), Path::new(PORTAL_GUI).is_file())
}

/// Replace this process with the desktop app center. Only returns on failure. The user's environment is kept on
/// purpose: the window needs DISPLAY / WAYLAND_DISPLAY and XDG_RUNTIME_DIR.
pub fn exec_gui(socket: &str) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(PORTAL_GUI).env("CROUSTYPATCH_PORTAL_SOCKET", socket).exec()
}
