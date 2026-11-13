//! Update notifier: runs in the user's graphical session (started at login) and tells the user, with a desktop
//! notification, when updates can be installed. It asks the agent's socket like the window does (so it has the same
//! rights), sends notifications with `notify-send` (libnotify) or, failing that, over D-Bus with `gdbus`, and never
//! opens a window or a webview.
use crate::notify_logic::{fingerprint, interval_secs, is_french, message, should_notify};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn state_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("croustypatch").join("last-notified"))
}

fn read_last() -> Option<String> {
    std::fs::read_to_string(state_file()?).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn write_last(fp: &str) {
    if let Some(p) = state_file() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, fp);
    }
}

/// (user may update, pending (name, candidate), security count, an update is already running)
fn summarise(v: &Value) -> (bool, Vec<(String, String)>, usize, bool) {
    let allowed = v.get("allowed").and_then(Value::as_bool).unwrap_or(false);
    let pending = v.get("pending").and_then(Value::as_array).cloned().unwrap_or_default();
    let items = pending.iter().filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), p.get("candidate").and_then(Value::as_str).unwrap_or("").to_string()))).collect();
    let security = pending.iter().filter(|p| p.get("is_security").and_then(Value::as_bool) == Some(true)).count();
    let busy = v.get("open_task").is_some_and(|t| !t.is_null());
    (allowed, items, security, busy)
}

fn open_window() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

/// A GVariant string literal: single quotes, with backslash and quote escaped.
fn gv(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Send a notification. With a click-to-open action when libnotify supports it, plainly otherwise.
fn send(title: &str, body: &str, open_label: &str) {
    let (title, body, open_label) = (title.to_string(), body.to_string(), open_label.to_string());
    // Waiting for a click blocks, so it happens on its own thread.
    std::thread::spawn(move || {
        let action = format!("--action=open={open_label}");
        let with_action = Command::new("notify-send").args(["--app-name=CroustyPatch", "--icon=croustypatch-portal", &action, "--wait", &title, &body]).stdin(Stdio::null()).stderr(Stdio::null()).output();
        match with_action {
            Ok(o) if o.status.success() => {
                if String::from_utf8_lossy(&o.stdout).trim() == "open" {
                    open_window();
                }
                return;
            }
            _ => {}
        }
        // An older libnotify (no --action), or no notify-send at all.
        if Command::new("notify-send").args(["--app-name=CroustyPatch", "--icon=croustypatch-portal", &title, &body]).stdin(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
            return;
        }
        let _ = Command::new("gdbus")
            .args(["call", "--session", "--dest", "org.freedesktop.Notifications", "--object-path", "/org/freedesktop/Notifications", "--method", "org.freedesktop.Notifications.Notify", "CroustyPatch", "0", "croustypatch-portal", &gv(&title), &gv(&body), "[]", "{}", "10000"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

fn check_once() {
    let Ok(v) = crate::ask(json!({ "op": "updates" })) else { return };
    let (allowed, items, security, busy) = summarise(&v);
    if items.is_empty() {
        write_last(""); // everything installed: the next list is news again
        return;
    }
    if busy {
        return; // an update is running: not the moment
    }
    let fp = fingerprint(&items);
    if !should_notify(allowed, items.len(), &fp, read_last().as_deref()) {
        return;
    }
    let french = is_french(&std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")).unwrap_or_default());
    let (title, body) = message(french, items.len(), security);
    send(&title, &body, if french { "Ouvrir" } else { "Open" });
    write_last(&fp);
}

/// Check now, then every half hour (CROUSTYPATCH_NOTIFIER_INTERVAL, in seconds, 60 to 86400).
pub fn run() {
    let every = interval_secs(std::env::var("CROUSTYPATCH_NOTIFIER_INTERVAL").ok().as_deref());
    std::thread::sleep(Duration::from_secs(20)); // let the session settle
    loop {
        check_once();
        std::thread::sleep(Duration::from_secs(every));
    }
}
