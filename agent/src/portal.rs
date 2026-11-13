//! Local app center service (spec portail_app §4.2): a Unix socket through which unprivileged local users (the TUI)
//! list the catalog for this host and ask for installs. The agent relays to the server with its own identity; the
//! server applies the same checks as for the web portal (module on, app offered to this host, one at a time) and
//! records the local account that asked (taken from the socket's peer credentials, not from the client).
use crate::config::Config;
use crate::identity::Identity;
use crate::net;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// One line of JSON from the client.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    List,
    Install { app_id: String },
    Uninstall { app_id: String },
    /// Pending system updates of this machine, and whether the user may start them.
    Updates,
    /// Start the pending system updates (all, or the named ones), as the user who asks.
    Update {
        #[serde(default)]
        packages: Vec<String>,
    },
    /// Look for updates now (refresh the repositories and send a fresh inventory); returns at once.
    Check,
}

/// A check is already running: asking again does nothing.
static CHECKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One line of JSON back: `{"ok": true, "data": …}` or `{"ok": false, "error": "…"}`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub error: String,
}

/// Local account name for a uid, from /etc/passwd.
pub fn user_name(passwd: &str, uid: u32) -> String {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() > 2 && f[2] == uid.to_string()).then(|| f[0].to_string())
    }).filter(|n| !n.is_empty() && n.len() <= 32 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))).unwrap_or_else(|| format!("uid{uid}"))
}

/// Add `local: {present, total, missing}` to every app, from the packages installed on this machine; a command app
/// counts as present when its check path is usable by the asking user (`usable`).
pub fn annotate_local(view: &mut Value, installed: &std::collections::HashSet<String>, usable: impl Fn(&str) -> bool) {
    if let Some(apps) = view.get_mut("apps").and_then(|a| a.as_array_mut()) {
        for app in apps {
            if let Some(check) = app.get("command").and_then(|c| c.get("check")).and_then(|c| c.as_str()).map(str::to_owned) {
                let ok = usable(&check);
                app["local"] = json!({ "present": ok as u32, "total": 1, "missing": if ok { vec![] } else { vec![check] } });
                continue;
            }
            let pkgs: Vec<String> = app.get("packages").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            let missing: Vec<&String> = pkgs.iter().filter(|p| !installed.contains(*p)).collect();
            app["local"] = json!({ "present": pkgs.len() - missing.len(), "total": pkgs.len(), "missing": missing });
        }
    }
}

/// For command apps that say where their latest version is published: what is installed, what is the latest, and
/// whether an update is available (`local.installed_version`, `local.latest_version`, `local.update`). Quiet when the
/// lookup fails: the app is then shown as before.
pub async fn annotate_updates(view: &mut Value) {
    use croustypatch_common::protocol::LatestVersion;
    let Some(apps) = view.get_mut("apps").and_then(|a| a.as_array_mut()) else { return };
    for app in apps {
        let Some(LatestVersion::Aur(pkg)) = app.get("command").and_then(|c| c.get("latest")).and_then(|l| serde_json::from_value::<LatestVersion>(l.clone()).ok()) else { continue };
        if app.pointer("/local/present").and_then(|p| p.as_u64()) != Some(1) {
            continue;
        }
        let (Some(have), Ok(latest)) = (crate::modules::aur::installed(&pkg).await, crate::modules::aur::latest(&pkg).await) else { continue };
        let newer = crate::modules::aur::is_newer(&latest, &have).await;
        app["local"]["installed_version"] = json!(have);
        app["local"]["latest_version"] = json!(latest);
        app["local"]["update"] = json!(newer);
    }
}

/// Group id of a group name, from /etc/group.
pub fn group_id(group: &str, groups: &str) -> Option<u32> {
    groups.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() > 2 && f[0] == group).then(|| f[2].parse().ok()).flatten()
    })
}

pub struct Relay {
    /// The running agent, to check which packages are really on this machine (None in tests of the relay alone).
    pub agent: Option<Arc<crate::runner::Agent>>,
    pub cfg: Config,
    pub identity: Identity,
    pub server_ips: Arc<dyn Fn() -> Vec<IpAddr> + Send + Sync>,
}

impl Relay {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let client = net::http_client_pinned(&self.cfg, &(self.server_ips)())?;
        let url = net::api_url(&self.cfg, path);
        let req = if method == "POST" { client.post(&url).json(&body.unwrap_or(Value::Null)) } else { client.get(&url) };
        let r = req.bearer_auth(self.identity.jwt()?).send().await.context("contacting the server")?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            // The server puts a short code in "error" and the readable explanation in "message".
            return Err(anyhow!("{}", v.get("message").or_else(|| v.get("error")).and_then(|e| e.as_str()).unwrap_or(status.as_str())));
        }
        Ok(v)
    }

    pub async fn handle(&self, req: Request, user: &str, uid: u32) -> Response {
        let bad_id = matches!(&req, Request::Install { app_id } | Request::Uninstall { app_id } if app_id.len() > 64);
        let bad_list = matches!(&req, Request::Update { packages } if packages.len() > 1000 || packages.iter().any(|p| !croustypatch_common::model::is_valid_package_name(p)));
        let res = match req {
            _ if bad_id => Err(anyhow!("bad app id")),
            _ if bad_list => Err(anyhow!("bad package list")),
            Request::Updates => self.call("GET", "/agent/portal/updates", None).await,
            Request::Update { packages } => self.call("POST", "/agent/portal/update", Some(json!({ "packages": packages, "user": user }))).await,
            Request::Check => match &self.agent {
                Some(a) if !CHECKING.swap(true, std::sync::atomic::Ordering::SeqCst) => {
                    let a = a.clone();
                    tokio::spawn(async move {
                        let log = crate::runner::TaskLog::default();
                        a.backend.refresh(&a.runner, &log).await;
                        if let Err(e) = a.send_inventory().await {
                            tracing::warn!("update check: {e:#}");
                        }
                        CHECKING.store(false, std::sync::atomic::Ordering::SeqCst);
                    });
                    Ok(json!({ "started": true }))
                }
                Some(_) => Ok(json!({ "started": false, "already": true })),
                None => Err(anyhow!("not available here")),
            },
            Request::List => match self.call("GET", "/agent/portal", None).await {
                Ok(mut v) => {
                    if let Some(a) = &self.agent {
                        // Checked on this machine at every listing: the truth, even for software installed by hand.
                        if let Ok(inv) = a.backend.inventory(&a.runner).await {
                            let names: std::collections::HashSet<String> = inv.into_iter().filter(|p| p.source == "system").map(|p| p.name).collect();
                            // Command apps: usable by the person at the keyboard, not just present for root.
                            let target = crate::app_command::lookup(user).await.unwrap_or(crate::app_command::Target { name: user.to_string(), uid, gid: u32::MAX - 1, groups: vec![], home: String::new() });
                            annotate_local(&mut v, &names, |p| crate::app_command::usable_by(Path::new(p), &target).is_ok());
                            if !a.offline() {
                                annotate_updates(&mut v).await;
                            }
                        }
                    }
                    Ok(v)
                }
                e => e,
            },
            Request::Install { app_id } => self.call("POST", "/agent/portal/install", Some(json!({ "app_id": app_id, "action": "INSTALL", "user": user }))).await,
            Request::Uninstall { app_id } => self.call("POST", "/agent/portal/install", Some(json!({ "app_id": app_id, "action": "UNINSTALL", "user": user }))).await,
        };
        match res {
            Ok(data) => Response { ok: true, data, error: String::new() },
            Err(e) => Response { ok: false, data: Value::Null, error: format!("{e:#}") },
        }
    }
}

/// Serve the socket until the agent stops.
pub async fn serve(relay: Arc<Relay>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = Path::new(&relay.cfg.portal_socket).to_path_buf();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    let group = relay.cfg.portal_group.trim().to_string();
    if group.is_empty() {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
    } else {
        let gid = group_id(&group, &std::fs::read_to_string("/etc/group").unwrap_or_default()).ok_or_else(|| anyhow!("portal_group {group} does not exist"))?;
        std::os::unix::fs::chown(&path, None, Some(gid))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
    }
    tracing::info!(socket = %path.display(), "app center socket ready");
    loop {
        let (stream, _) = listener.accept().await?;
        let relay = relay.clone();
        tokio::spawn(async move {
            let uid = stream.peer_cred().map(|c| c.uid()).unwrap_or(u32::MAX);
            let user = user_name(&std::fs::read_to_string("/etc/passwd").unwrap_or_default(), uid);
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r.take(64 * 1024)).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let resp = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => relay.handle(req, &user, uid).await,
                    Err(_) => Response { ok: false, data: Value::Null, error: "bad request".into() },
                };
                let mut out = serde_json::to_vec(&resp).unwrap_or_default();
                out.push(b'\n');
                if w.write_all(&out).await.is_err() {
                    break;
                }
            }
        });
    }
}

/// Client side: one request, one response.
pub async fn ask(socket: &str, req: &Request) -> Result<Response> {
    let s = tokio::net::UnixStream::connect(socket).await.with_context(|| format!("cannot reach the CroustyPatch agent at {socket} (is it running?)"))?;
    let (r, mut w) = s.into_split();
    let mut line = serde_json::to_vec(req)?;
    line.push(b'\n');
    w.write_all(&line).await?;
    let mut reader = BufReader::new(r);
    let mut buf = String::new();
    reader.read_line(&mut buf).await?;
    serde_json::from_str(&buf).context("unreadable answer from the agent")
}
