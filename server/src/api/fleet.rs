use super::{name_ok, one_of};
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::gateway::{self, token_hash};
use crate::models::{EnrollmentToken, Host, HostGroup, HostPackage, Task};
use crate::planner::OPEN;
use crate::schema::{enrollment_tokens, host_group_members, host_groups, host_packages, hosts, task_events, tasks};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::{Duration, Utc};
use croustypatch_common::crypto::random_token;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Serialize)]
struct HostSummary {
    #[serde(flatten)]
    host: Host,
    group_ids: Vec<Uuid>,
    /// Pending updates of every kind (system packages and container images).
    updates: i64,
    security_updates: i64,
    /// Of those, the container images: the system updates are `updates - docker_updates`.
    docker_updates: i64,
}

#[derive(Deserialize)]
pub struct HostQuery {
    status: Option<String>,
    q: Option<String>,
    group: Option<Uuid>,
}

pub async fn list_hosts(state: web::Data<AppState>, user: AuthUser, q: web::Query<HostQuery>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let q = q.into_inner();
    let out = state
        .db(move |c| {
            let mut query = hosts::table.into_boxed();
            if let Some(s) = &q.status {
                query = query.filter(hosts::status.eq(s.clone()));
            }
            if let Some(t) = q.q.as_ref().filter(|t| !t.trim().is_empty()) {
                let like = format!("%{}%", t.trim().replace('%', "\\%").replace('_', "\\_"));
                query = query.filter(hosts::hostname.ilike(like.clone()).or(hosts::display_name.ilike(like)));
            }
            if let Some(g) = q.group {
                query = query.filter(hosts::id.eq_any(host_group_members::table.filter(host_group_members::group_id.eq(g)).select(host_group_members::host_id)));
            }
            let list: Vec<Host> = query.order(hosts::hostname.asc()).select(Host::as_select()).load(c)?;
            let counts: Vec<(Uuid, bool, String)> = host_packages::table.filter(host_packages::candidate.is_not_null()).select((host_packages::host_id, host_packages::is_security, host_packages::source)).load(c)?;
            let mut upd: HashMap<Uuid, (i64, i64, i64)> = HashMap::new();
            for (h, sec, source) in counts {
                let e = upd.entry(h).or_default();
                e.0 += 1;
                e.1 += sec as i64;
                e.2 += (source == "docker") as i64;
            }
            let members: Vec<(Uuid, Uuid)> = host_group_members::table.select((host_group_members::host_id, host_group_members::group_id)).load(c)?;
            let mut groups: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
            for (h, g) in members {
                groups.entry(h).or_default().push(g);
            }
            Ok(list
                .into_iter()
                .map(|h| {
                    let (updates, security_updates, docker_updates) = upd.get(&h.id).copied().unwrap_or_default();
                    HostSummary { group_ids: groups.remove(&h.id).unwrap_or_default(), host: h, updates, security_updates, docker_updates }
                })
                .collect::<Vec<_>>())
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

pub async fn get_host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let id = path.into_inner();
    let out = state
        .db(move |c| {
            let host: Host = hosts::table.find(id).select(Host::as_select()).first(c).map_err(|_| ApiError::NotFound("host"))?;
            let packages: Vec<HostPackage> = host_packages::table.filter(host_packages::host_id.eq(id)).order((host_packages::candidate.is_null().asc(), host_packages::is_security.desc(), host_packages::name.asc())).select(HostPackage::as_select()).load(c)?;
            let group_ids: Vec<Uuid> = host_group_members::table.filter(host_group_members::host_id.eq(id)).select(host_group_members::group_id).load(c)?;
            let recent: Vec<Task> = tasks::table.filter(tasks::host_id.eq(id)).order(tasks::created_at.desc()).limit(30).select(Task::as_select()).load(c)?;
            Ok(json!({ "host": host, "group_ids": group_ids, "packages": packages, "tasks": recent }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize)]
pub struct MetricsQuery {
    /// How far back, in hours (default 24, at most two weeks).
    hours: Option<i64>,
}

/// GET /hosts/{id}/metrics — the latest resource reading and its history (one point every five minutes).
pub async fn host_metrics(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, q: web::Query<MetricsQuery>) -> ApiResult<HttpResponse> {
    use crate::schema::host_metrics;
    user.require("hosts:read")?;
    let id = path.into_inner();
    let hours = q.hours.unwrap_or(24).clamp(1, 24 * 14);
    let out = state
        .db(move |c| {
            let (current, at): (serde_json::Value, Option<chrono::DateTime<Utc>>) = hosts::table.find(id).select((hosts::metrics, hosts::metrics_at)).first(c).map_err(|_| ApiError::NotFound("host"))?;
            let pts: Vec<(chrono::DateTime<Utc>, Option<f32>, Option<f32>, Option<f32>)> = host_metrics::table
                .filter(host_metrics::host_id.eq(id))
                .filter(host_metrics::at.ge(Utc::now() - Duration::hours(hours)))
                .order(host_metrics::at.asc())
                .select((host_metrics::at, host_metrics::cpu_pct, host_metrics::mem_pct, host_metrics::load_1m))
                .load(c)?;
            Ok(json!({ "current": current, "at": at, "points": pts.into_iter().map(|(at, cpu, mem, load)| json!({ "at": at, "cpu": cpu, "mem": mem, "load": load })).collect::<Vec<_>>() }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize)]
pub struct HostPatch {
    display_name: Option<String>,
    environment: Option<String>,
    criticality: Option<String>,
    asset_type: Option<String>,
    internet_exposed: Option<bool>,
    has_staging: Option<bool>,
    ring: Option<i32>,
    tags: Option<Vec<String>>,
    heartbeat_secs: Option<i32>,
    group_ids: Option<Vec<Uuid>>,
    /// Optional modules for this host: any subset of docker, helper_script, openscap, portal, eco.
    modules: Option<serde_json::Value>,
    /// Audit profile of this host: '' (fleet default), an XCCDF profile id or `custom:<uuid>`.
    compliance_profile: Option<String>,
}

pub async fn update_host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<HostPatch>) -> ApiResult<HttpResponse> {
    user.require("hosts:write")?;
    let id = path.into_inner();
    let b = body.into_inner();
    if let Some(v) = &b.environment {
        one_of("environment", v, &["production", "staging", "dev"])?;
    }
    if let Some(v) = &b.criticality {
        one_of("criticality", v, &["low", "medium", "high"])?;
    }
    if let Some(v) = &b.asset_type {
        one_of("asset_type", v, &["server", "workstation", "laptop"])?;
    }
    if b.ring.is_some_and(|r| !(0..=10).contains(&r)) {
        return Err(ApiError::bad("ring must be 0..=10"));
    }
    if b.heartbeat_secs.is_some_and(|h| !(10..=3600).contains(&h)) {
        return Err(ApiError::bad("heartbeat_secs must be 10..=3600"));
    }
    if let Some(n) = &b.display_name {
        if !n.is_empty() {
            name_ok("display_name", n)?;
        }
    }
    if b.tags.as_ref().is_some_and(|t| t.len() > 30 || t.iter().any(|x| x.is_empty() || x.len() > 40)) {
        return Err(ApiError::bad("at most 30 tags of 1-40 characters"));
    }
    if let Some(m) = &b.modules {
        crate::modules::validate(m)?;
    }
    if b.compliance_profile.as_deref().is_some_and(|p| !crate::compliance::valid_profile_ref(p)) {
        return Err(ApiError::bad("compliance_profile must be empty, an XCCDF profile id or a custom profile"));
    }
    let modules_changed = b.modules.is_some();
    let out: Host = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                #[derive(AsChangeset)]
                #[diesel(table_name = hosts)]
                struct P {
                    display_name: Option<String>,
                    environment: Option<String>,
                    criticality: Option<String>,
                    asset_type: Option<String>,
                    internet_exposed: Option<bool>,
                    has_staging: Option<bool>,
                    ring: Option<i32>,
                    tags: Option<serde_json::Value>,
                    heartbeat_secs: Option<i32>,
                    compliance_profile: Option<String>,
                }
                if let Some(cp) = b.compliance_profile.as_deref().and_then(|p| p.strip_prefix(crate::compliance::CUSTOM_PREFIX)) {
                    let known: i64 = crate::schema::compliance_profiles::table.filter(crate::schema::compliance_profiles::id.eq(Uuid::parse_str(cp).unwrap_or_default())).count().get_result(c)?;
                    if known == 0 {
                        return Err(ApiError::bad("unknown custom audit profile"));
                    }
                }
                let p = P {
                    display_name: b.display_name.clone().map(|s| s.trim().to_string()),
                    environment: b.environment.clone(),
                    criticality: b.criticality.clone(),
                    asset_type: b.asset_type.clone(),
                    internet_exposed: b.internet_exposed,
                    has_staging: b.has_staging,
                    ring: b.ring,
                    tags: b.tags.as_ref().map(|t| json!(t)),
                    heartbeat_secs: b.heartbeat_secs,
                    compliance_profile: b.compliance_profile.clone(),
                };
                let any = p.compliance_profile.is_some() || p.display_name.is_some() || p.environment.is_some() || p.criticality.is_some() || p.asset_type.is_some() || p.internet_exposed.is_some() || p.has_staging.is_some() || p.ring.is_some() || p.tags.is_some() || p.heartbeat_secs.is_some();
                if any {
                    let n = diesel::update(hosts::table.find(id)).set(&p).execute(c)?;
                    if n == 0 {
                        return Err(ApiError::NotFound("host"));
                    }
                }
                if let Some(m) = &b.modules {
                    // A partial map only changes the modules it names.
                    let mut cur: serde_json::Value = hosts::table.find(id).select(hosts::modules).first(c).map_err(|_| ApiError::NotFound("host"))?;
                    let mut merged = serde_json::to_value(crate::modules::parse(&cur)).unwrap_or_default();
                    for (k, v) in m.as_object().into_iter().flatten() {
                        merged[k] = v.clone();
                    }
                    if merged != cur {
                        diesel::update(hosts::table.find(id)).set(hosts::modules.eq(&merged)).execute(c)?;
                        audit::record(c, Event::new(&user.username, "host.modules_changed").target("host", &id.to_string()).host(id).details(json!({ "before": cur, "after": merged })).ip(&user.ip))?;
                        cur = merged;
                    }
                    let _ = cur;
                }
                if let Some(gs) = &b.group_ids {
                    diesel::delete(host_group_members::table.filter(host_group_members::host_id.eq(id))).execute(c)?;
                    for g in gs {
                        diesel::insert_into(host_group_members::table).values((host_group_members::host_id.eq(id), host_group_members::group_id.eq(g))).on_conflict_do_nothing().execute(c)?;
                    }
                }
                audit::record(c, Event::new(&user.username, "host.updated").target("host", &id.to_string()).host(id).details(json!({
                    "environment": b.environment, "criticality": b.criticality, "internet_exposed": b.internet_exposed,
                    "has_staging": b.has_staging, "ring": b.ring, "groups": b.group_ids, "compliance_profile": b.compliance_profile
                })).ip(&user.ip))?;
                Ok(hosts::table.find(id).select(Host::as_select()).first(c).map_err(|_| ApiError::NotFound("host"))?)
            })
        })
        .await?;
    if modules_changed {
        crate::modules::push_settings(&state, id).await;
    }
    Ok(HttpResponse::Ok().json(out))
}

pub async fn delete_host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("hosts:delete")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            let n = diesel::delete(hosts::table.find(id)).execute(c)?;
            if n == 0 {
                return Err(ApiError::NotFound("host"));
            }
            audit::record(c, Event::new(&user.username, "host.deleted").target("host", &id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

/// Create a queued control task (scan / quarantine / release) for a host.
async fn control_task(state: &web::Data<AppState>, user: AuthUser, host_id: Uuid, kind: &'static str, action: &'static str) -> ApiResult<HttpResponse> {
    control_task_with(state, user, host_id, kind, action, None).await
}

pub(crate) async fn control_task_with(state: &web::Data<AppState>, user: AuthUser, host_id: Uuid, kind: &'static str, action: &'static str, exec: Option<serde_json::Value>) -> ApiResult<HttpResponse> {
    let id: Uuid = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                let status: String = hosts::table.find(host_id).select(hosts::status).first(c).map_err(|_| ApiError::NotFound("host"))?;
                if status == "quarantined" && matches!(kind, "scan" | "reboot" | "agent_update") {
                    return Err(ApiError::Conflict("host is quarantined; release it first".into()));
                }
                let open: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq(kind)).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
                if open > 0 {
                    return Err(ApiError::Conflict(format!("a {kind} task is already pending for this host")));
                }
                let id: Uuid = diesel::insert_into(tasks::table)
                    .values((tasks::host_id.eq(host_id), tasks::kind.eq(kind), tasks::status.eq("queued"), tasks::requested_by.eq(&user.username), tasks::message.eq(format!("{kind} requested by {}", user.username)), tasks::exec.eq(exec.clone())))
                    .returning(tasks::id)
                    .get_result(c)?;
                diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(format!("requested by {}", user.username)))).execute(c)?;
                audit::record(c, Event::new(&user.username, action).target("task", &id.to_string()).host(host_id).ip(&user.ip))?;
                Ok(id)
            })
        })
        .await?;
    gateway::push_pending(state, host_id).await;
    Ok(HttpResponse::Accepted().json(json!({ "task_id": id })))
}

pub async fn scan_host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:create")?;
    control_task(&state, user, path.into_inner(), "scan", "task.scan_requested").await
}

pub async fn quarantine_host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:write")?;
    control_task(&state, user, path.into_inner(), "quarantine", "host.quarantine_requested").await
}

#[derive(Deserialize, Default)]
pub struct RebootBody {
    /// Minutes of warning for logged-in users; default: 10 on workstations and laptops, 0 on servers.
    pub delay_minutes: Option<u32>,
    #[serde(default)]
    pub message: String,
}

/// Restart a host now (or after a warning delay): the "force reboot" button of the reboot module.
pub async fn reboot_host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: Option<web::Json<RebootBody>>) -> ApiResult<HttpResponse> {
    user.require("tasks:write")?;
    let id = path.into_inner();
    let b = body.map(|b| b.into_inner()).unwrap_or_default();
    if b.message.len() > 200 {
        return Err(ApiError::bad("message must be at most 200 characters"));
    }
    let asset: String = state.db(move |c| Ok(hosts::table.find(id).select(hosts::asset_type).first::<String>(c).map_err(|_| ApiError::NotFound("host"))?)).await?;
    let delay = b.delay_minutes.unwrap_or(crate::reboot::default_delay(&asset)).min(24 * 60);
    control_task_with(&state, user, id, "reboot", "host.reboot_requested", Some(json!({ "delay_minutes": delay, "message": b.message }))).await
}

/// The agent build this server hands out, for a host's platform: (platform id, sha256, version, desktop app center
/// sha256 when the host has it installed and this server has a build of it).
pub fn agent_build_for(state: &AppState, facts: &serde_json::Value) -> ApiResult<(String, String, String, Option<String>)> {
    let os = facts.get("os_family").and_then(|v| v.as_str()).unwrap_or("");
    let arch = facts.get("arch").and_then(|v| v.as_str()).unwrap_or("");
    let platform = match (os, arch) {
        ("linux", "x86_64" | "amd64") => "linux-x86_64",
        ("linux", "aarch64" | "arm64") => "linux-aarch64",
        ("freebsd", "amd64" | "x86_64") => "freebsd-amd64",
        _ => return Err(ApiError::Conflict(format!("no agent build for {os} {arch}"))),
    };
    let path = super::agent_dist::binary_path(state, platform);
    let bytes = std::fs::read(&path).map_err(|_| ApiError::Conflict(format!("this server has no agent binary for {platform}")))?;
    let gui = if facts.get("portal_gui").and_then(|v| v.as_bool()).unwrap_or(false) {
        super::agent_dist::gui_path(state, platform).and_then(|p| std::fs::read(p).ok()).map(|b| croustypatch_common::crypto::sha256_hex(&b))
    } else {
        None
    };
    Ok((platform.to_string(), croustypatch_common::crypto::sha256_hex(&bytes), env!("CARGO_PKG_VERSION").to_string(), gui))
}

/// POST /hosts/{id}/agent-update — the agent downloads this server's build, checks its SHA-256 and restarts on it.
pub async fn update_agent(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("hosts:write")?;
    let id = path.into_inner();
    let facts: serde_json::Value = state.db(move |c| Ok(hosts::table.find(id).select(hosts::facts).first(c).map_err(|_| ApiError::NotFound("host"))?)).await?;
    if !crate::modules::has_capability(&facts, croustypatch_common::protocol::capability::SELF_UPDATE) {
        return Err(ApiError::Conflict("this agent is too old to update itself: re-run the installer one-liner on the host once".into()));
    }
    let (platform, sha256, version, gui_sha256) = agent_build_for(&state, &facts)?;
    control_task_with(&state, user, id, "agent_update", "host.agent_update_requested", Some(json!({ "platform": platform, "sha256": sha256, "version": version, "gui_sha256": gui_sha256 }))).await
}

pub async fn release_host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:write")?;
    control_task(&state, user, path.into_inner(), "release", "host.release_requested").await
}

// ----------------------------------------------------------------------- groups

#[derive(Serialize)]
struct GroupOut {
    #[serde(flatten)]
    group: HostGroup,
    host_count: i64,
}

pub async fn list_groups(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("groups:read")?;
    let out = state
        .db(|c| {
            let gs: Vec<HostGroup> = host_groups::table.order(host_groups::name.asc()).select(HostGroup::as_select()).load(c)?;
            let counts: Vec<(Uuid, i64)> = host_group_members::table.group_by(host_group_members::group_id).select((host_group_members::group_id, diesel::dsl::count_star())).load(c)?;
            let m: HashMap<Uuid, i64> = counts.into_iter().collect();
            Ok(gs.into_iter().map(|g| GroupOut { host_count: m.get(&g.id).copied().unwrap_or(0), group: g }).collect::<Vec<_>>())
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

/// Colour keys a group may use. Fixed on purpose: every choice sits well with the brand palette.
pub const GROUP_COLORS: &[&str] = &["brick", "sand", "moss", "teal", "sky", "plum", "rose", "slate", "ink"];

#[derive(Deserialize)]
pub struct GroupBody {
    name: String,
    #[serde(default)]
    description: String,
    /// One of `GROUP_COLORS`, or empty for an automatic colour derived from the name.
    #[serde(default)]
    color: String,
}

/// A palette name, a custom `#rrggbb`, or empty (automatic). Returns the value to store (hex in lower case).
fn color_ok(c: &str) -> ApiResult<String> {
    let is_hex = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
    if c.is_empty() || GROUP_COLORS.contains(&c) {
        Ok(c.to_string())
    } else if is_hex {
        Ok(c.to_ascii_lowercase())
    } else {
        Err(ApiError::bad(format!("color must be empty, a #rrggbb value, or one of: {}", GROUP_COLORS.join(", "))))
    }
}

pub async fn create_group(state: web::Data<AppState>, user: AuthUser, body: web::Json<GroupBody>) -> ApiResult<HttpResponse> {
    user.require("groups:create")?;
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    let color = color_ok(&b.color)?;
    let g: HostGroup = state
        .db(move |c| {
            let g: HostGroup = diesel::insert_into(host_groups::table).values((host_groups::name.eq(b.name.trim()), host_groups::description.eq(&b.description), host_groups::color.eq(&color))).returning(HostGroup::as_returning()).get_result(c)?;
            audit::record(c, Event::new(&user.username, "group.created").target("group", &g.id.to_string()).details(json!({"name": g.name})).ip(&user.ip))?;
            Ok(g)
        })
        .await?;
    Ok(HttpResponse::Created().json(g))
}

pub async fn update_group(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<GroupBody>) -> ApiResult<HttpResponse> {
    user.require("groups:write")?;
    let id = path.into_inner();
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    let color = color_ok(&b.color)?;
    let g: HostGroup = state
        .db(move |c| {
            let g: HostGroup = diesel::update(host_groups::table.find(id)).set((host_groups::name.eq(b.name.trim()), host_groups::description.eq(&b.description), host_groups::color.eq(&color))).returning(HostGroup::as_returning()).get_result(c).map_err(|e| match e {
                diesel::result::Error::NotFound => ApiError::NotFound("group"),
                e => e.into(),
            })?;
            audit::record(c, Event::new(&user.username, "group.updated").target("group", &id.to_string()).ip(&user.ip))?;
            Ok(g)
        })
        .await?;
    Ok(HttpResponse::Ok().json(g))
}

pub async fn delete_group(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("groups:delete")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            if diesel::delete(host_groups::table.find(id)).execute(c)? == 0 {
                return Err(ApiError::NotFound("group"));
            }
            audit::record(c, Event::new(&user.username, "group.deleted").target("group", &id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

// ----------------------------------------------------------- enrolment tokens

pub async fn list_tokens(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("enrollment:read")?;
    let rows: Vec<EnrollmentToken> = state.db(|c| Ok(enrollment_tokens::table.order(enrollment_tokens::created_at.desc()).limit(200).select(EnrollmentToken::as_select()).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows))
}

#[derive(Deserialize)]
pub struct NewToken {
    #[serde(default)]
    label: String,
    group_id: Option<Uuid>,
    environment: Option<String>,
    comm_mode: Option<String>,
    /// Validity in hours (default 24, max 720).
    hours: Option<i64>,
    max_uses: Option<i32>,
    /// Modules for the hosts enrolled with this token; absent = the defaults (Settings > Modules) at enrolment.
    modules: Option<serde_json::Value>,
}

pub async fn create_token(state: web::Data<AppState>, user: AuthUser, body: web::Json<NewToken>) -> ApiResult<HttpResponse> {
    user.require("enrollment:create")?;
    let b = body.into_inner();
    let env = b.environment.unwrap_or_else(|| "production".into());
    let mode = b.comm_mode.unwrap_or_else(|| "pull".into());
    one_of("environment", &env, &["production", "staging", "dev"])?;
    one_of("comm_mode", &mode, &["push", "pull"])?;
    let hours = b.hours.unwrap_or(24);
    let max_uses = b.max_uses.unwrap_or(1);
    if !(1..=720).contains(&hours) || !(1..=1000).contains(&max_uses) {
        return Err(ApiError::bad("hours must be 1..=720 and max_uses 1..=1000"));
    }
    // Stored as given (only the modules the person chose): a module left out is decided at enrolment, for example
    // updates from the app center are on for laptops and workstations and off for servers.
    let modules = match &b.modules {
        Some(m) => {
            crate::modules::validate(m)?;
            Some(m.clone())
        }
        None => None,
    };
    let token = format!("cpt_{}", random_token(24));
    let hash = token_hash(&token);
    let row: EnrollmentToken = state
        .db(move |c| {
            let t: EnrollmentToken = diesel::insert_into(enrollment_tokens::table)
                .values((
                    enrollment_tokens::token_hash.eq(&hash),
                    enrollment_tokens::label.eq(&b.label),
                    enrollment_tokens::group_id.eq(b.group_id),
                    enrollment_tokens::environment.eq(&env),
                    enrollment_tokens::comm_mode.eq(&mode),
                    enrollment_tokens::expires_at.eq(Utc::now() + Duration::hours(hours)),
                    enrollment_tokens::max_uses.eq(max_uses),
                    enrollment_tokens::created_by.eq(&user.username),
                    enrollment_tokens::modules.eq(&modules),
                ))
                .returning(EnrollmentToken::as_returning())
                .get_result(c)?;
            audit::record(c, Event::new(&user.username, "enrollment.token_created").target("token", &t.id.to_string()).details(json!({"label": t.label, "max_uses": max_uses, "hours": hours, "modules": t.modules})).ip(&user.ip))?;
            Ok(t)
        })
        .await?;
    // The clear token is returned exactly once; only its hash is stored.
    Ok(HttpResponse::Created().json(json!({ "token": token, "details": row })))
}

pub async fn revoke_token(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("enrollment:delete")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            if diesel::update(enrollment_tokens::table.find(id)).set(enrollment_tokens::revoked.eq(true)).execute(c)? == 0 {
                return Err(ApiError::NotFound("token"));
            }
            audit::record(c, Event::new(&user.username, "enrollment.token_revoked").target("token", &id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}
