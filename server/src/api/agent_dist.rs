//! Agent distribution: the installer script and the agent binaries, so a host is enrolled with one command.
//!
//! These endpoints are public on purpose (a new host has no credentials yet) and carry no secret: the enrolment
//! token is supplied by the operator on the host, never embedded here. Binaries are verified by SHA-256.
use crate::auth::{is_https, public_host};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use actix_files::NamedFile;
use actix_web::http::header::{self, ContentDisposition, DispositionType};
use actix_web::{web, HttpRequest, HttpResponse};
use croustypatch_common::crypto::sha256_hex;
use serde::Serialize;
use std::path::PathBuf;

/// (id, os, arch). The id is also the file suffix: `croustypatch-agent-<id>` in the agent directory.
pub const PLATFORMS: &[(&str, &str, &str)] = &[("linux-x86_64", "linux", "x86_64"), ("linux-aarch64", "linux", "aarch64")];

const INSTALL_SCRIPT: &str = include_str!("../../assets/install-agent.sh");

/// Platforms with a build of the desktop app center (Tauri; aarch64 hosts use the terminal one).
pub const GUI_PLATFORMS: &[&str] = &["linux-x86_64"];
/// Download id of the desktop app center's icon (installed with its menu entry).
pub const GUI_ICON: &str = "portal-gui-icon.png";

/// File name behind a download id. Only these names ever reach the filesystem: no path can be smuggled in.
pub fn artifact_file(id: &str) -> Option<String> {
    if PLATFORMS.iter().any(|(p, _, _)| *p == id) {
        return Some(format!("croustypatch-agent-{id}"));
    }
    if let Some(p) = id.strip_prefix("portal-gui-").filter(|p| GUI_PLATFORMS.contains(p)) {
        return Some(format!("croustypatch-portal-gui-{p}"));
    }
    (id == GUI_ICON).then(|| "croustypatch-portal-gui.png".to_string())
}

fn file_path(state: &AppState, name: &str) -> PathBuf {
    state.cfg.agent_dir.iter().map(|d| d.join(name)).find(|p| p.is_file()).unwrap_or_else(|| state.cfg.agent_dir.first().cloned().unwrap_or_default().join(name))
}

/// The first configured directory that holds a binary for this platform (or the first directory, for error messages).
pub(crate) fn binary_path(state: &AppState, id: &str) -> PathBuf {
    file_path(state, &format!("croustypatch-agent-{id}"))
}

/// The desktop app center build for a platform, if this server has one.
pub(crate) fn gui_path(state: &AppState, platform: &str) -> Option<PathBuf> {
    artifact_file(&format!("portal-gui-{platform}")).map(|n| file_path(state, &n)).filter(|p| p.is_file())
}

#[derive(Serialize)]
pub struct Platform {
    id: &'static str,
    os: &'static str,
    arch: &'static str,
    available: bool,
    size: Option<u64>,
    sha256: Option<String>,
    /// This server also hands out the desktop app center for this platform.
    portal_gui: bool,
}

/// Which platforms this server can hand an agent to.
pub async fn platforms(state: web::Data<AppState>) -> ApiResult<HttpResponse> {
    let mut out = vec![];
    for (id, os, arch) in PLATFORMS {
        let path = binary_path(&state, id);
        let (size, sha256) = match tokio::fs::read(&path).await {
            Ok(bytes) => (Some(bytes.len() as u64), Some(sha256_hex(&bytes))),
            Err(_) => (None, None),
        };
        out.push(Platform { id, os, arch, available: size.is_some(), size, sha256, portal_gui: gui_path(&state, id).is_some() });
    }
    Ok(HttpResponse::Ok().json(out))
}

/// `GET /agent/download/<platform>` serves the binary; `<platform>.sha256` serves its checksum.
pub async fn download(req: HttpRequest, state: web::Data<AppState>, path: web::Path<String>) -> ApiResult<HttpResponse> {
    let raw = path.into_inner();
    let (id, want_sum) = match raw.strip_suffix(".sha256") {
        Some(id) => (id, true),
        None => (raw.as_str(), false),
    };
    let name = artifact_file(id).ok_or(ApiError::NotFound("platform"))?;
    let file = file_path(&state, &name);
    if !file.is_file() {
        return Err(ApiError::NotFound("build for this platform"));
    }
    if want_sum {
        let bytes = tokio::fs::read(&file).await.map_err(ApiError::internal)?;
        let body = format!("{}  {name}\n", sha256_hex(&bytes));
        return Ok(HttpResponse::Ok().content_type("text/plain; charset=utf-8").body(body));
    }
    let named = NamedFile::open(&file)
        .map_err(ApiError::internal)?
        .set_content_disposition(ContentDisposition { disposition: DispositionType::Attachment, parameters: vec![header::DispositionParam::Filename(name.clone())] });
    Ok(named.into_response(&req))
}

fn host_is_safe(h: &str) -> bool {
    !h.is_empty() && h.len() <= 255 && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

/// The installer, with this server's own public URL filled in.
pub async fn install_script(req: HttpRequest, state: web::Data<AppState>) -> ApiResult<HttpResponse> {
    let host = public_host(&req, &state.cfg);
    // The script is piped into a root shell: refuse anything that is not a plain host name.
    if !host_is_safe(&host) {
        return Err(ApiError::bad("cannot determine this server's public address from the request"));
    }
    let scheme = if is_https(&req, &state) { "https" } else { "http" };
    let body = INSTALL_SCRIPT.replace("__SERVER__", &format!("{scheme}://{host}"));
    Ok(HttpResponse::Ok().content_type("text/x-shellscript; charset=utf-8").insert_header((header::CACHE_CONTROL, "no-store")).body(body))
}
