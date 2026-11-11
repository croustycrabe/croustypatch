//! App center (spec portail_app): an admin catalog of applications, and self-service install / uninstall on a host
//! without root rights. The server checks the catalog and the host's scope, then signs the order for the agent.
//!
//! Each application has one recipe per package manager (`apt`, `pacman`, `pkg`, `dnf`) or one for every system
//! (`*`): either native package names, or an admin-written command (install / uninstall / check path). Command
//! recipes run as root on the hosts in scope, so writing or changing one needs the `portal:commands` permission.
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::planner::{self, OPEN};
use crate::schema::{host_packages, hosts, portal_apps, task_events, tasks};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::Utc;
use croustypatch_common::model::{is_valid_check_path, is_valid_package_name, pick_recipe, RECIPE_KEYS};
use croustypatch_common::protocol::{capability, AppCommand};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub use crate::icons::GENERIC as ICONS;

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
    /// {"apt": {"packages": [...]}, "pacman": {...}, "*": {"command": {"install", "uninstall", "check"}}}
    #[serde(default)]
    recipes: Option<Value>,
    /// Shorthand kept for API clients from before recipes: the same package names on every system.
    #[serde(default)]
    packages: Option<Vec<String>>,
    #[serde(default)]
    group_ids: Vec<Uuid>,
    #[serde(default = "yes")]
    enabled: bool,
}
fn yes() -> bool {
    true
}

/// Permission needed to write or change an application that runs commands as root.
pub const COMMANDS_PERMISSION: &str = "portal:commands";
pub const MAX_SCRIPT: usize = 8000;

fn recipes_of(b: &AppBody) -> ApiResult<Value> {
    match (&b.recipes, &b.packages) {
        (Some(r), _) => Ok(r.clone()),
        (None, Some(p)) => Ok(json!({ "*": { "packages": p } })),
        (None, None) => Err(ApiError::bad("an application needs recipes (or packages)")),
    }
}

fn check_script(what: &str, s: &str) -> ApiResult<()> {
    if s.trim().is_empty() || s.len() > MAX_SCRIPT || s.contains('\0') {
        return Err(ApiError::bad(format!("the {what} command must be 1 to {MAX_SCRIPT} characters")));
    }
    Ok(())
}

/// Validate a recipe map; returns whether it contains a command recipe.
pub fn check_recipes(r: &Value) -> ApiResult<bool> {
    let o = r.as_object().filter(|o| !o.is_empty()).ok_or_else(|| ApiError::bad("recipes must name at least one system (apt, pacman, pkg, dnf or *)"))?;
    let mut has_command = false;
    for (k, v) in o {
        if !RECIPE_KEYS.contains(&k.as_str()) {
            return Err(ApiError::bad(format!("unknown system {k:?} in recipes (allowed: {})", RECIPE_KEYS.join(", "))));
        }
        match (v.get("packages"), v.get("command")) {
            (Some(p), None) => {
                let pkgs: Vec<&str> = p.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
                if pkgs.is_empty() || pkgs.len() > 50 || p.as_array().is_some_and(|a| a.len() != pkgs.len()) {
                    return Err(ApiError::bad(format!("{k}: an application needs 1 to 50 packages")));
                }
                if let Some(bad) = pkgs.iter().find(|p| !is_valid_package_name(p) || p.contains(':')) {
                    return Err(ApiError::bad(format!("{k}: invalid package name: {bad}")));
                }
            }
            (None, Some(c)) => {
                let cmd: AppCommand = serde_json::from_value(c.clone()).map_err(|_| ApiError::bad(format!("{k}: a command needs install, uninstall and check")))?;
                check_script("install", &cmd.install)?;
                check_script("uninstall", &cmd.uninstall)?;
                if !is_valid_check_path(&cmd.check) {
                    return Err(ApiError::bad(format!("{k}: check must be an absolute path (no ..), e.g. /opt/app/bin/app")));
                }
                has_command = true;
            }
            _ => return Err(ApiError::bad(format!("{k}: give either packages or a command"))),
        }
    }
    Ok(has_command)
}

pub fn has_command(r: &Value) -> bool {
    r.as_object().is_some_and(|o| o.values().any(|v| v.get("command").is_some()))
}

fn check(b: &AppBody) -> ApiResult<()> {
    super::name_ok("name", &b.name)?;
    if b.version.len() > 40 || b.category.len() > 40 || b.description.len() > 2000 {
        return Err(ApiError::bad("version and category: 40 characters max, description: 2000"));
    }
    crate::icons::check(&b.icon).map_err(ApiError::bad)?;
    check_recipes(&recipes_of(b)?)?;
    Ok(())
}

type Row = (Uuid, String, String, String, String, String, Value, Value, bool);
const COLS: (portal_apps::id, portal_apps::name, portal_apps::version, portal_apps::category, portal_apps::description, portal_apps::icon, portal_apps::recipes, portal_apps::group_ids, portal_apps::enabled) =
    (portal_apps::id, portal_apps::name, portal_apps::version, portal_apps::category, portal_apps::description, portal_apps::icon, portal_apps::recipes, portal_apps::group_ids, portal_apps::enabled);

fn app_json(r: &Row) -> Value {
    // `icon_url`: the image of a bundled logo (`brand:<slug>`); generic names are drawn by the interface and uploaded
    // images are already data URIs in `icon`.
    json!({ "id": r.0, "name": r.1, "version": r.2, "category": r.3, "description": r.4, "icon": r.5, "icon_url": crate::icons::url(&r.5), "recipes": r.6, "group_ids": r.7, "enabled": r.8, "has_command": has_command(&r.6) })
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
    let recipes = recipes_of(&b)?;
    let may_command = user.can(COMMANDS_PERMISSION);
    if has_command(&recipes) && !may_command {
        return Err(ApiError::forbidden(format!("command recipes run as root on every host in scope: they need the {COMMANDS_PERMISSION} permission")));
    }
    let row: Row = state
        .db(move |c| {
            c.transaction(|c| {
                if let Some(id) = id {
                    // Changing an app that already runs commands (its scope, or removing the command) needs the same right.
                    let old: Option<Value> = portal_apps::table.find(id).select(portal_apps::recipes).first(c).optional()?;
                    if old.as_ref().is_some_and(has_command) && !may_command {
                        return Err(ApiError::forbidden(format!("this application runs commands: changing it needs the {COMMANDS_PERMISSION} permission")));
                    }
                }
                let vals = (
                    portal_apps::name.eq(b.name.trim().to_string()),
                    portal_apps::version.eq(b.version.trim().to_string()),
                    portal_apps::category.eq(b.category.trim().to_string()),
                    portal_apps::description.eq(b.description.clone()),
                    portal_apps::icon.eq(b.icon.clone()),
                    portal_apps::recipes.eq(&recipes),
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
                audit::record(c, Event::new(&user.username, if id.is_some() { "portal.app_updated" } else { "portal.app_created" }).target("app", &r.0.to_string()).details(json!({ "name": r.1, "recipes": r.6, "group_ids": r.7, "enabled": r.8 })).ip(&user.ip))?;
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
    let may_command = user.can(COMMANDS_PERMISSION);
    state
        .db(move |c| {
            c.transaction(|c| {
                let old: Option<Value> = portal_apps::table.find(id).select(portal_apps::recipes).first(c).optional()?;
                if old.as_ref().is_some_and(has_command) && !may_command {
                    return Err(ApiError::forbidden(format!("this application runs commands: deleting it needs the {COMMANDS_PERMISSION} permission")));
                }
                if diesel::delete(portal_apps::table.find(id)).execute(c)? == 0 {
                    return Err(ApiError::NotFound("application"));
                }
                audit::record(c, Event::new(&user.username, "portal.app_deleted").target("app", &id.to_string()).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

fn portal_on(c: &mut PgConnection, host_id: Uuid) -> ApiResult<()> {
    if crate::modules::for_host(c, host_id)?.portal {
        Ok(())
    } else {
        Err(ApiError::Conflict("the app center is switched off for this host (host page > Modules)".into()))
    }
}

/// Catalog entries offered to a host: in its groups' scope and with a recipe for its package manager.
fn visible(c: &mut PgConnection, host_id: Uuid) -> ApiResult<Vec<Row>> {
    let groups = planner::host_groups(c, host_id)?;
    let pm: String = hosts::table.find(host_id).select(hosts::package_manager).first(c).map_err(|_| ApiError::NotFound("host"))?;
    let all: Vec<Row> = portal_apps::table.filter(portal_apps::enabled.eq(true)).order((portal_apps::category.asc(), portal_apps::name.asc())).select(COLS).load(c)?;
    Ok(all
        .into_iter()
        .filter(|r| {
            let scope: Vec<Uuid> = strs(&r.7).iter().filter_map(|s| s.parse().ok()).collect();
            (scope.is_empty() || scope.iter().any(|g| groups.contains(g))) && pick_recipe(&r.6, &pm).is_some()
        })
        .collect())
}

/// The portal view of one host: its apps, whether each is installed, and any request in progress.
/// The packages or command an app uses on a system.
fn resolve(recipes: &Value, pm: &str) -> (Vec<String>, Option<AppCommand>) {
    let r = pick_recipe(recipes, pm);
    let pkgs = r.and_then(|r| r.get("packages")).map(strs).unwrap_or_default();
    let cmd = r.and_then(|r| r.get("command")).and_then(|c| serde_json::from_value(c.clone()).ok());
    (pkgs, cmd)
}

pub fn view_for(c: &mut PgConnection, id: Uuid) -> ApiResult<Value> {
    portal_on(c, id)?;
    let (name, pm): (String, String) = hosts::table.find(id).select((hosts::hostname, hosts::package_manager)).first(c).map_err(|_| ApiError::NotFound("host"))?;
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
    // Command apps leave no package behind: their state on the server is the outcome of the last successful operation
    // (the terminal app center checks the path on the machine itself).
    let done: Vec<Option<Value>> = tasks::table.filter(tasks::host_id.eq(id)).filter(tasks::kind.eq("app")).filter(tasks::status.eq("succeeded")).order(tasks::finished_at.desc()).limit(500).select(tasks::exec).load(c)?;
    let last_command_state = |app: Uuid| done.iter().flatten().find(|e| e.get("app_id").and_then(|v| v.as_str()) == Some(&app.to_string())).and_then(|e| e.get("install")).and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(json!({
        "recent": recent.iter().map(|t| json!({
            "task_id": t.0, "status": t.1, "message": t.3, "updated_at": t.4,
            "app_id": t.2.as_ref().and_then(|e| e.get("app_id")).cloned(),
            "app_name": t.2.as_ref().and_then(|e| e.get("app_name")).cloned(),
            "install": t.2.as_ref().and_then(|e| e.get("install")).cloned(),
        })).collect::<Vec<_>>(),
        "host": { "id": id, "hostname": name },
        "apps": apps.iter().map(|r| {
            let (pkgs, cmd) = resolve(&r.6, &pm);
            let pending = open.iter().find(|t| t.2.as_ref().and_then(|e| e.get("app_id")).and_then(|v| v.as_str()) == Some(&r.0.to_string()));
            let mut v = app_json(r);
            v["kind"] = json!(if cmd.is_some() { "command" } else { "packages" });
            v["packages"] = json!(pkgs);
            v["command"] = json!(cmd);
            v["installed"] = json!(if cmd.is_some() { last_command_state(r.0) } else { pkgs.iter().all(|p| installed.contains(p)) });
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
/// `local_user` is the local account that asked through the terminal app center (None from the web UI).
pub fn create_request(c: &mut PgConnection, host_id: Uuid, app_id: Uuid, install: bool, actor: &str, ip: &str, local_user: Option<&str>) -> ApiResult<Uuid> {
    c.transaction(|c| {
        portal_on(c, host_id)?;
        let status: String = hosts::table.find(host_id).select(hosts::status).first(c).map_err(|_| ApiError::NotFound("host"))?;
        if matches!(status.as_str(), "pending" | "quarantined") {
            return Err(ApiError::Conflict(format!("the host is {status}")));
        }
        let app = visible(c, host_id)?.into_iter().find(|r| r.0 == app_id).ok_or_else(|| ApiError::forbidden("this application is not offered on this host"))?;
        let open: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq("app")).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
        if open > 0 {
            return Err(ApiError::Conflict("an install or uninstall is already running on this host".into()));
        }
        let pm: String = hosts::table.find(host_id).select(hosts::package_manager).first(c)?;
        let (packages, command) = resolve(&app.6, &pm);
        if command.is_some() && !crate::modules::host_has_capability(c, host_id, capability::APP_COMMAND)? {
            return Err(ApiError::Conflict("this application installs with a command, which this host's agent is too old to run: update the agent first".into()));
        }
        let verb = if install { "install" } else { "uninstall" };
        let exec = json!({ "install": install, "packages": packages, "command": command, "user": local_user, "app_id": app.0, "app_name": app.1 });
        let msg = format!("{verb} {} (app center) for {actor}", app.1);
        let id: Uuid = diesel::insert_into(tasks::table)
            .values((tasks::host_id.eq(host_id), tasks::kind.eq("app"), tasks::status.eq("queued"), tasks::requested_by.eq(actor), tasks::message.eq(&msg), tasks::exec.eq(Some(exec))))
            .returning(tasks::id)
            .get_result(c)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(&msg))).execute(c)?;
        audit::record(c, Event::new(actor, &format!("portal.{verb}_requested")).target("task", &id.to_string()).host(host_id).details(json!({ "app": app.1, "packages": packages, "command": command, "user": local_user })).ip(ip))?;
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
    let task = state.db(move |c| create_request(c, b.host_id, b.app_id, install, &actor, &ip, None)).await?;
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
            create_request(c, host, b.app_id, install, &format!("local:{}@{hostname}", b.user), &ip, Some(&b.user))
        })
        .await?;
    crate::gateway::push_pending(&state, host).await;
    Ok(HttpResponse::Accepted().json(json!({ "status": "queued", "task_id": task })))
}

// ------------------------------------------------------------------ suggested applications

const SUGGESTIONS: &str = include_str!("../../assets/app-suggestions.json");

/// GET /portal/suggestions — a curated list of common applications with their package names per system, to add to
/// the catalog in one click. Each entry says whether an app with that name is already in the catalog.
pub async fn suggestions(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let mut list: Vec<Value> = serde_json::from_str(SUGGESTIONS).map_err(ApiError::internal)?;
    let names: Vec<String> = state.db(|c| Ok(portal_apps::table.select(portal_apps::name).load(c)?)).await?;
    let names: std::collections::HashSet<String> = names.into_iter().map(|n| n.to_lowercase()).collect();
    for s in &mut list {
        let n = s.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        s["in_catalog"] = json!(names.contains(&n));
        s["icon_url"] = json!(s.get("icon").and_then(|i| i.as_str()).and_then(crate::icons::url));
    }
    Ok(HttpResponse::Ok().json(list))
}

/// GET /portal/brand-icons — the bundled logos, for the icon picker of the catalog editor.
pub async fn brand_icons(user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    Ok(HttpResponse::Ok().json(crate::icons::catalog()))
}

// ------------------------------------------------------------------ system updates from the app center

/// What the person at a machine sees about its updates, and whether they may start them.
pub fn updates_view(c: &mut PgConnection, host_id: Uuid) -> ApiResult<Value> {
    portal_on(c, host_id)?;
    let modules = crate::modules::for_host(c, host_id)?;
    let (hostname, status, reboot, checked): (String, String, bool, Option<chrono::DateTime<Utc>>) = hosts::table.find(host_id).select((hosts::hostname, hosts::status, hosts::reboot_required, hosts::last_inventory_at)).first(c).map_err(|_| ApiError::NotFound("host"))?;
    let pending: Vec<(String, String, Option<String>, bool)> = host_packages::table
        .filter(host_packages::host_id.eq(host_id))
        .filter(host_packages::source.eq("system"))
        .filter(host_packages::candidate.is_not_null())
        .order((host_packages::is_security.desc(), host_packages::name.asc()))
        .select((host_packages::name, host_packages::version, host_packages::candidate, host_packages::is_security))
        .load(c)?;
    let open: Option<(Uuid, String, String, String, chrono::DateTime<Utc>)> = tasks::table
        .filter(tasks::host_id.eq(host_id))
        .filter(tasks::kind.eq_any(["patch", "rollback"]))
        .filter(tasks::status.eq_any(OPEN))
        .order(tasks::created_at.desc())
        .select((tasks::id, tasks::kind, tasks::status, tasks::message, tasks::scheduled_for))
        .first(c)
        .optional()?;
    let since = Utc::now() - chrono::Duration::hours(24);
    let recent: Vec<(Uuid, String, String, Option<chrono::DateTime<Utc>>)> = tasks::table
        .filter(tasks::host_id.eq(host_id))
        .filter(tasks::kind.eq_any(["patch", "rollback"]))
        .filter(tasks::finished_at.gt(since))
        .order(tasks::finished_at.desc())
        .limit(5)
        .select((tasks::id, tasks::status, tasks::message, tasks::finished_at))
        .load(c)?;
    Ok(json!({
        "host": { "id": host_id, "hostname": hostname, "status": status },
        "allowed": modules.portal_updates,
        "reboot_required": reboot,
        "checked_at": checked,
        "pending": pending.iter().map(|(name, version, candidate, security)| json!({ "name": name, "version": version, "candidate": candidate, "is_security": security })).collect::<Vec<_>>(),
        "open_task": open.map(|(id, kind, status, message, at)| json!({ "id": id, "kind": kind, "status": status, "message": message, "scheduled_for": at })),
        "recent": recent.into_iter().map(|(id, status, message, at)| json!({ "task_id": id, "status": status, "message": message, "finished_at": at })).collect::<Vec<_>>(),
    }))
}

/// Create a patch task for the pending system updates of a host, asked for by the person using it. They decide: no
/// approval step and no waiting for a patch day (the restore point, health checks and rollback are the usual ones).
pub fn create_update_request(c: &mut PgConnection, host_id: Uuid, packages: &[String], actor: &str, ip: &str, local_user: &str) -> ApiResult<Uuid> {
    c.transaction(|c| {
        portal_on(c, host_id)?;
        if !crate::modules::for_host(c, host_id)?.portal_updates {
            return Err(ApiError::forbidden("updates from the app center are not allowed on this machine: ask your administrator"));
        }
        let host: crate::models::Host = hosts::table.find(host_id).select(crate::models::Host::as_select()).first(c).map_err(|_| ApiError::NotFound("host"))?;
        if matches!(host.status.as_str(), "pending" | "quarantined") {
            return Err(ApiError::Conflict(format!("the machine is {}", host.status)));
        }
        let open: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq_any(["patch", "rollback"])).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
        if open > 0 {
            return Err(ApiError::Conflict("an update is already running or waiting on this machine".into()));
        }
        if packages.len() > 1000 || packages.iter().any(|p| !is_valid_package_name(p)) {
            return Err(ApiError::bad("invalid package list"));
        }
        let mut q = host_packages::table.filter(host_packages::host_id.eq(host_id)).filter(host_packages::source.eq("system")).filter(host_packages::candidate.is_not_null()).into_boxed();
        if !packages.is_empty() {
            q = q.filter(host_packages::name.eq_any(packages.to_vec()));
        }
        let pending: Vec<crate::models::HostPackage> = q.select(crate::models::HostPackage::as_select()).load(c)?;
        if let Some(missing) = packages.iter().find(|p| !pending.iter().any(|x| &x.name == *p)) {
            return Err(ApiError::Conflict(format!("{missing} has no pending update")));
        }
        if pending.is_empty() {
            return Err(ApiError::Conflict("there is nothing to update".into()));
        }
        let batch: Vec<planner::Candidate> = pending.into_iter().filter_map(|p| p.candidate.clone().map(|candidate| planner::Candidate { pkg: p, candidate })).collect();
        let groups = planner::host_groups(c, host_id)?;
        let def = planner::applicable_policy(c, &host, &groups)?;
        let exec = def.as_ref().map(|p| planner::exec_policy(&p.def, host.has_staging, &host.criticality)).unwrap_or_default();
        let msg = format!("update from the app center for {local_user}");
        let id: Uuid = diesel::insert_into(tasks::table)
            .values((
                tasks::host_id.eq(host_id),
                tasks::kind.eq("patch"),
                tasks::status.eq("queued"),
                tasks::policy_id.eq(def.as_ref().map(|d| d.id)),
                tasks::packages.eq(planner::packages_json(&batch)),
                tasks::exec.eq(Some(serde_json::to_value(&exec).unwrap_or_default())),
                tasks::requested_by.eq(actor),
                tasks::message.eq(&msg),
            ))
            .returning(tasks::id)
            .get_result(c)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(&msg))).execute(c)?;
        audit::record(c, Event::new(actor, "portal.update_requested").target("task", &id.to_string()).host(host_id).details(json!({ "packages": batch.len(), "all": packages.is_empty(), "user": local_user })).ip(ip))?;
        Ok(id)
    })
}

/// GET /agent/portal/updates
pub async fn agent_updates(state: web::Data<AppState>, agent: crate::gateway::AgentAuth) -> ApiResult<HttpResponse> {
    let id = agent.host_id;
    Ok(HttpResponse::Ok().json(state.db(move |c| updates_view(c, id)).await?))
}

#[derive(Deserialize)]
pub struct AgentUpdateBody {
    /// Names to update; empty = every pending system update.
    #[serde(default)]
    packages: Vec<String>,
    /// Local account that asked (from the socket's peer credentials).
    user: String,
}

/// POST /agent/portal/update
pub async fn agent_update(state: web::Data<AppState>, agent: crate::gateway::AgentAuth, body: web::Json<AgentUpdateBody>) -> ApiResult<HttpResponse> {
    let b = body.into_inner();
    if b.user.is_empty() || b.user.len() > 32 || !b.user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err(ApiError::bad("invalid local user name"));
    }
    let host = agent.host_id;
    let ip = agent.ip.clone();
    let task = state
        .db(move |c| {
            let hostname: String = hosts::table.find(host).select(hosts::hostname).first(c)?;
            create_update_request(c, host, &b.packages, &format!("local:{}@{hostname}", b.user), &ip, &b.user)
        })
        .await?;
    crate::gateway::push_pending(&state, host).await;
    Ok(HttpResponse::Accepted().json(json!({ "status": "queued", "task_id": task })))
}
