//! App center (spec portail_app): an admin catalog of applications, and self-service install / uninstall on a host
//! without root rights. The server checks the catalog and the host's scope, then signs the order for the agent.
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::planner::{self, OPEN};
use crate::schema::{host_packages, hosts, portal_apps, task_events, tasks};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::Utc;
use croustypatch_common::model::is_valid_package_name;
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub const ICONS: &[&str] = &["", "app-window", "code", "terminal", "browser", "file-text", "chart-bar", "shield-check", "database", "image", "music-notes", "chat-circle", "wrench", "cube"];

#[derive(Deserialize)]
pub struct AppBody {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon: String,
    packages: Vec<String>,
    #[serde(default)]
    group_ids: Vec<Uuid>,
    #[serde(default = "yes")]
    enabled: bool,
}
fn yes() -> bool {
    true
}

fn check(b: &AppBody) -> ApiResult<()> {
    super::name_ok("name", &b.name)?;
    if b.version.len() > 40 || b.category.len() > 40 || b.description.len() > 2000 {
        return Err(ApiError::bad("version and category: 40 characters max, description: 2000"));
    }
    if !ICONS.contains(&b.icon.as_str()) {
        return Err(ApiError::bad(format!("icon must be one of: {}", ICONS[1..].join(", "))));
    }
    if b.packages.is_empty() || b.packages.len() > 50 {
        return Err(ApiError::bad("an application needs 1 to 50 packages"));
    }
    if let Some(bad) = b.packages.iter().find(|p| !is_valid_package_name(p) || p.contains(':')) {
        return Err(ApiError::bad(format!("invalid package name: {bad}")));
    }
    Ok(())
}

type Row = (Uuid, String, String, String, String, String, Value, Value, bool);
const COLS: (portal_apps::id, portal_apps::name, portal_apps::version, portal_apps::category, portal_apps::description, portal_apps::icon, portal_apps::packages, portal_apps::group_ids, portal_apps::enabled) =
    (portal_apps::id, portal_apps::name, portal_apps::version, portal_apps::category, portal_apps::description, portal_apps::icon, portal_apps::packages, portal_apps::group_ids, portal_apps::enabled);

fn app_json(r: &Row) -> Value {
    json!({ "id": r.0, "name": r.1, "version": r.2, "category": r.3, "description": r.4, "icon": r.5, "packages": r.6, "group_ids": r.7, "enabled": r.8 })
}
fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

pub async fn list(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let rows: Vec<Row> = state.db(|c| Ok(portal_apps::table.order((portal_apps::category.asc(), portal_apps::name.asc())).select(COLS).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows.iter().map(app_json).collect::<Vec<_>>()))
}

pub async fn save(state: web::Data<AppState>, user: AuthUser, id: Option<Uuid>, b: AppBody) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    check(&b)?;
    let row: Row = state
        .db(move |c| {
            c.transaction(|c| {
                let vals = (
                    portal_apps::name.eq(b.name.trim().to_string()),
                    portal_apps::version.eq(b.version.trim().to_string()),
                    portal_apps::category.eq(b.category.trim().to_string()),
                    portal_apps::description.eq(b.description.clone()),
                    portal_apps::icon.eq(b.icon.clone()),
                    portal_apps::packages.eq(json!(b.packages)),
                    portal_apps::group_ids.eq(json!(b.group_ids)),
                    portal_apps::enabled.eq(b.enabled),
                );
                let map = |e: diesel::result::Error| match e {
                    diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => ApiError::Conflict("an application with this name exists".into()),
                    diesel::result::Error::NotFound => ApiError::NotFound("application"),
                    e => e.into(),
                };
                let r: Row = match id {
                    Some(id) => diesel::update(portal_apps::table.find(id)).set((vals, portal_apps::updated_at.eq(Utc::now()))).returning(COLS).get_result(c).map_err(map)?,
                    None => diesel::insert_into(portal_apps::table).values(vals).returning(COLS).get_result(c).map_err(map)?,
                };
                audit::record(c, Event::new(&user.username, if id.is_some() { "portal.app_updated" } else { "portal.app_created" }).target("app", &r.0.to_string()).details(json!({ "name": r.1, "packages": r.6 })).ip(&user.ip))?;
                Ok(r)
            })
        })
        .await?;
    Ok(if id.is_some() { HttpResponse::Ok() } else { HttpResponse::Created() }.json(app_json(&row)))
}

pub async fn create(state: web::Data<AppState>, user: AuthUser, body: web::Json<AppBody>) -> ApiResult<HttpResponse> {
    save(state, user, None, body.into_inner()).await
}
pub async fn update(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<AppBody>) -> ApiResult<HttpResponse> {
    save(state, user, Some(path.into_inner()), body.into_inner()).await
}
pub async fn delete(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            c.transaction(|c| {
                if diesel::delete(portal_apps::table.find(id)).execute(c)? == 0 {
                    return Err(ApiError::NotFound("application"));
                }
                audit::record(c, Event::new(&user.username, "portal.app_deleted").target("app", &id.to_string()).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

fn portal_on(c: &mut PgConnection) -> ApiResult<()> {
    if crate::compliance::module_on(&crate::compliance::modules(c), "portal") {
        Ok(())
    } else {
        Err(ApiError::Conflict("the app center module is switched off (Settings > Modules)".into()))
    }
}

/// Catalog entries visible on a host's groups.
fn visible(c: &mut PgConnection, host_id: Uuid) -> ApiResult<Vec<Row>> {
    let groups = planner::host_groups(c, host_id)?;
    let all: Vec<Row> = portal_apps::table.filter(portal_apps::enabled.eq(true)).order((portal_apps::category.asc(), portal_apps::name.asc())).select(COLS).load(c)?;
    Ok(all
        .into_iter()
        .filter(|r| {
            let scope: Vec<Uuid> = strs(&r.7).iter().filter_map(|s| s.parse().ok()).collect();
            scope.is_empty() || scope.iter().any(|g| groups.contains(g))
        })
        .collect())
}

/// The portal view of one host: its apps, whether each is installed, and any request in progress.
pub fn view_for(c: &mut PgConnection, id: Uuid) -> ApiResult<Value> {
    portal_on(c)?;
    let name: String = hosts::table.find(id).select(hosts::hostname).first(c).map_err(|_| ApiError::NotFound("host"))?;
    let apps = visible(c, id)?;
    let installed: Vec<String> = host_packages::table.filter(host_packages::host_id.eq(id)).filter(host_packages::source.eq("system")).select(host_packages::name).load(c)?;
    let open: Vec<(Uuid, String, Option<Value>, String)> = tasks::table.filter(tasks::host_id.eq(id)).filter(tasks::kind.eq("app")).filter(tasks::status.eq_any(OPEN)).select((tasks::id, tasks::status, tasks::exec, tasks::message)).load(c)?;
    // Outcome of the host's recent app-center operations, so clients can tell how a request ended.
    let since = Utc::now() - chrono::Duration::hours(24);
    let recent: Vec<(Uuid, String, Option<Value>, String, chrono::DateTime<Utc>)> = tasks::table
        .filter(tasks::host_id.eq(id))
        .filter(tasks::kind.eq("app"))
        .filter(tasks::updated_at.ge(since))
        .order(tasks::updated_at.desc())
        .limit(10)
        .select((tasks::id, tasks::status, tasks::exec, tasks::message, tasks::updated_at))
        .load(c)?;
    Ok(json!({
        "recent": recent.iter().map(|t| json!({
            "task_id": t.0, "status": t.1, "message": t.3, "updated_at": t.4,
            "app_id": t.2.as_ref().and_then(|e| e.get("app_id")).cloned(),
            "app_name": t.2.as_ref().and_then(|e| e.get("app_name")).cloned(),
            "install": t.2.as_ref().and_then(|e| e.get("install")).cloned(),
        })).collect::<Vec<_>>(),
        "host": { "id": id, "hostname": name },
        "apps": apps.iter().map(|r| {
            let pkgs = strs(&r.6);
            let pending = open.iter().find(|t| t.2.as_ref().and_then(|e| e.get("app_id")).and_then(|v| v.as_str()) == Some(&r.0.to_string()));
            let mut v = app_json(r);
            v["installed"] = json!(pkgs.iter().all(|p| installed.contains(p)));
            v["pending"] = pending.map(|t| json!({ "task_id": t.0, "status": t.1, "install": t.2.as_ref().and_then(|e| e.get("install")).cloned(), "message": t.3 })).unwrap_or(Value::Null);
            v
        }).collect::<Vec<_>>(),
    }))
}

pub async fn host_view(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let id = path.into_inner();
    let out = state.db(move |c| view_for(c, id)).await?;
    Ok(HttpResponse::Ok().json(out))
}

/// Create an install / uninstall task after every check (module on, host usable, app offered, one at a time).
pub fn create_request(c: &mut PgConnection, host_id: Uuid, app_id: Uuid, install: bool, actor: &str, ip: &str) -> ApiResult<Uuid> {
    c.transaction(|c| {
        portal_on(c)?;
        let status: String = hosts::table.find(host_id).select(hosts::status).first(c).map_err(|_| ApiError::NotFound("host"))?;
        if matches!(status.as_str(), "pending" | "quarantined") {
            return Err(ApiError::Conflict(format!("the host is {status}")));
        }
        let app = visible(c, host_id)?.into_iter().find(|r| r.0 == app_id).ok_or_else(|| ApiError::forbidden("this application is not offered on this host"))?;
        let open: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq("app")).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
        if open > 0 {
            return Err(ApiError::Conflict("an install or uninstall is already running on this host".into()));
        }
        let verb = if install { "install" } else { "uninstall" };
        let exec = json!({ "install": install, "packages": app.6, "app_id": app.0, "app_name": app.1 });
        let msg = format!("{verb} {} (app center) for {actor}", app.1);
        let id: Uuid = diesel::insert_into(tasks::table)
            .values((tasks::host_id.eq(host_id), tasks::kind.eq("app"), tasks::status.eq("queued"), tasks::requested_by.eq(actor), tasks::message.eq(&msg), tasks::exec.eq(Some(exec))))
            .returning(tasks::id)
            .get_result(c)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(&msg))).execute(c)?;
        audit::record(c, Event::new(actor, &format!("portal.{verb}_requested")).target("task", &id.to_string()).host(host_id).details(json!({ "app": app.1, "packages": app.6 })).ip(ip))?;
        Ok(id)
    })
}

fn action_of(a: &str) -> ApiResult<bool> {
    match a {
        "INSTALL" => Ok(true),
        "UNINSTALL" => Ok(false),
        _ => Err(ApiError::bad("action must be INSTALL or UNINSTALL")),
    }
}

#[derive(Deserialize)]
pub struct InstallBody {
    host_id: Uuid,
    app_id: Uuid,
    /// INSTALL | UNINSTALL
    action: String,
}

/// POST /portal/install — spec portail_app §6.
pub async fn request(state: web::Data<AppState>, user: AuthUser, body: web::Json<InstallBody>) -> ApiResult<HttpResponse> {
    user.require("tasks:create")?;
    let b = body.into_inner();
    let install = action_of(&b.action)?;
    let (actor, ip) = (user.username.clone(), user.ip.clone());
    let task = state.db(move |c| create_request(c, b.host_id, b.app_id, install, &actor, &ip)).await?;
    crate::gateway::push_pending(&state, b.host_id).await;
    Ok(HttpResponse::Accepted().json(json!({ "status": "queued", "task_id": task, "message": "Deployment command sent to the agent." })))
}

// ------------------------------------------------------------------ agent side (local TUI / GUI clients)

/// GET /agent/portal — the portal view of the calling agent's own host.
pub async fn agent_view(state: web::Data<AppState>, agent: crate::gateway::AgentAuth) -> ApiResult<HttpResponse> {
    let id = agent.host_id;
    Ok(HttpResponse::Ok().json(state.db(move |c| view_for(c, id)).await?))
}

#[derive(Deserialize)]
pub struct AgentInstallBody {
    app_id: Uuid,
    action: String,
    /// Local account that asked (from the socket's peer credentials), for the audit trail.
    user: String,
}

/// POST /agent/portal/install — a local user of the host asked through the agent. The host can only act on itself,
/// and only on apps offered to it.
pub async fn agent_request(state: web::Data<AppState>, agent: crate::gateway::AgentAuth, body: web::Json<AgentInstallBody>) -> ApiResult<HttpResponse> {
    let b = body.into_inner();
    let install = action_of(&b.action)?;
    if b.user.is_empty() || b.user.len() > 32 || !b.user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err(ApiError::bad("invalid local user name"));
    }
    let host = agent.host_id;
    let ip = agent.ip.clone();
    let task = state
        .db(move |c| {
            let hostname: String = hosts::table.find(host).select(hosts::hostname).first(c)?;
            create_request(c, host, b.app_id, install, &format!("local:{}@{hostname}", b.user), &ip)
        })
        .await?;
    crate::gateway::push_pending(&state, host).await;
    Ok(HttpResponse::Accepted().json(json!({ "status": "queued", "task_id": task })))
}
