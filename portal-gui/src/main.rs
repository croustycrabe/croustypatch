//! CroustyPatch desktop app center.
//!
//! Exactly as powerful as the terminal app center: it talks to the agent's local socket
//! (`/run/croustypatch/portal.sock`), and the agent identifies the user from the socket's peer credentials, never
//! from anything this program says. The window can call six commands (list, install, uninstall, updates, update, check); no shell, file
//! system or network access is exposed to it.
mod notifier;
mod notify_logic;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const DEFAULT_SOCKET: &str = "/run/croustypatch/portal.sock";

fn socket_path() -> String {
    std::env::var("CROUSTYPATCH_PORTAL_SOCKET").ok().filter(|s| s.starts_with('/')).unwrap_or_else(|| DEFAULT_SOCKET.into())
}

/// One request, one answer: the same line protocol as the terminal app center.
pub(crate) fn ask(req: Value) -> Result<Value, String> {
    let path = socket_path();
    let mut s = UnixStream::connect(&path).map_err(|e| format!("agent unreachable at {path} ({e})"))?;
    s.set_read_timeout(Some(Duration::from_secs(90))).map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    s.write_all(&line).map_err(|e| e.to_string())?;
    // At most 8 MiB, like the agent's own limits.
    let mut buf = String::new();
    BufReader::new(std::io::Read::take(s, 8 * 1024 * 1024)).read_line(&mut buf).map_err(|e| e.to_string())?;
    let r: Value = serde_json::from_str(&buf).map_err(|_| "unreadable answer from the agent".to_string())?;
    if r.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(r.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(r.get("error").and_then(Value::as_str).unwrap_or("the agent refused the request").to_string())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

async fn blocking(req: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || ask(req)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list() -> Result<Value, String> {
    blocking(json!({ "op": "list" })).await
}

#[tauri::command]
async fn install(app_id: String) -> Result<Value, String> {
    if !valid_id(&app_id) {
        return Err("bad application id".into());
    }
    blocking(json!({ "op": "install", "app_id": app_id })).await
}

#[tauri::command]
async fn uninstall(app_id: String) -> Result<Value, String> {
    if !valid_id(&app_id) {
        return Err("bad application id".into());
    }
    blocking(json!({ "op": "uninstall", "app_id": app_id })).await
}

/// A package name as package managers take it: letters, digits and `+ . _ - : ~ @`, not starting with `-` or `.`.
fn valid_pkg(p: &str) -> bool {
    !p.is_empty() && p.len() <= 128 && !p.starts_with(['-', '.']) && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '_' | '-' | ':' | '~' | '@'))
}

#[tauri::command]
async fn updates() -> Result<Value, String> {
    blocking(json!({ "op": "updates" })).await
}

/// Start the pending updates: the named packages, or all of them when the list is empty.
#[tauri::command]
async fn update(packages: Vec<String>) -> Result<Value, String> {
    if packages.len() > 1000 || !packages.iter().all(|p| valid_pkg(p)) {
        return Err("bad package list".into());
    }
    blocking(json!({ "op": "update", "packages": packages })).await
}

/// Ask the agent to look for updates now.
/// The version of this window (the same as the agent it ships with): shown in the header.
#[tauri::command]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
async fn check() -> Result<Value, String> {
    blocking(json!({ "op": "check" })).await
}

fn main() {
    // Started at login by the desktop autostart entry: no window, only notifications.
    if std::env::args().any(|a| a == "--notifier") {
        notifier::run();
        return;
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list, install, uninstall, updates, update, check, version])
        .run(tauri::generate_context!())
        .expect("cannot start the CroustyPatch app center");
}
