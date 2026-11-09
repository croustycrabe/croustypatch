use super::name_ok;
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::engine::policy::PolicyDef;
use crate::error::{ApiError, ApiResult};
use crate::gateway;
use crate::models::{Policy, Task, TaskEvent, Vulnerability, Waiver};
use crate::planner::{self, OPEN};
use crate::schema::{host_packages, hosts, policies, policy_groups, task_events, tasks, vulnerabilities, waivers};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::{DateTime, Duration, Utc};
use croustypatch_common::model::is_valid_package_name;
use croustypatch_common::protocol::ExecPolicy;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

// --------------------------------------------------------------------- policies

#[derive(Serialize)]
struct PolicyOut {
    #[serde(flatten)]
    policy: Policy,
    group_ids: Vec<Uuid>,
}

pub async fn list_policies(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("policies:read")?;
    let out = state
        .db(|c| {
            let ps: Vec<Policy> = policies::table.order((policies::priority.asc(), policies::name.asc())).select(Policy::as_select()).load(c)?;
            let links: Vec<(Uuid, Uuid)> = policy_groups::table.select((policy_groups::policy_id, policy_groups::group_id)).load(c)?;
            let mut m: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
            for (p, g) in links {
                m.entry(p).or_default().push(g);
            }
            Ok(ps.into_iter().map(|p| PolicyOut { group_ids: m.remove(&p.id).unwrap_or_default(), policy: p }).collect::<Vec<_>>())
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize)]
pub struct PolicyBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "t")]
    enabled: bool,
    #[serde(default = "hundred")]
    priority: i32,
    definition: Value,
    #[serde(default)]
    group_ids: Vec<Uuid>,
}
fn t() -> bool {
    true
}
fn hundred() -> i32 {
    100
}

fn parse_def(v: &Value) -> ApiResult<PolicyDef> {
    let def: PolicyDef = serde_json::from_value(v.clone()).map_err(|e| ApiError::bad(format!("invalid policy definition: {e}")))?;
    def.validate().map_err(ApiError::BadRequest)?;
    Ok(def)
}

pub async fn create_policy(state: web::Data<AppState>, user: AuthUser, body: web::Json<PolicyBody>) -> ApiResult<HttpResponse> {
    user.require("policies:create")?;
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    let def = parse_def(&b.definition)?;
    let canonical = serde_json::to_value(&def).map_err(ApiError::internal)?;
    let out = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                let p: Policy = diesel::insert_into(policies::table)
                    .values((policies::name.eq(b.name.trim()), policies::description.eq(&b.description), policies::enabled.eq(b.enabled), policies::priority.eq(b.priority), policies::definition.eq(&canonical)))
                    .returning(Policy::as_returning())
                    .get_result(c)?;
                for g in &b.group_ids {
                    diesel::insert_into(policy_groups::table).values((policy_groups::policy_id.eq(p.id), policy_groups::group_id.eq(g))).on_conflict_do_nothing().execute(c)?;
                }
                audit::record(c, Event::new(&user.username, "policy.created").target("policy", &p.id.to_string()).details(json!({"name": p.name, "groups": b.group_ids, "definition": canonical})).ip(&user.ip))?;
                Ok(PolicyOut { policy: p, group_ids: b.group_ids })
            })
        })
        .await?;
    Ok(HttpResponse::Created().json(out))
}

pub async fn update_policy(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<PolicyBody>) -> ApiResult<HttpResponse> {
    user.require("policies:write")?;
    let id = path.into_inner();
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    let def = parse_def(&b.definition)?;
    let canonical = serde_json::to_value(&def).map_err(ApiError::internal)?;
    let out = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                let p: Policy = diesel::update(policies::table.find(id))
                    .set((policies::name.eq(b.name.trim()), policies::description.eq(&b.description), policies::enabled.eq(b.enabled), policies::priority.eq(b.priority), policies::definition.eq(&canonical), policies::updated_at.eq(Utc::now())))
                    .returning(Policy::as_returning())
                    .get_result(c)
                    .map_err(|e| match e {
                        diesel::result::Error::NotFound => ApiError::NotFound("policy"),
                        e => e.into(),
                    })?;
                diesel::delete(policy_groups::table.filter(policy_groups::policy_id.eq(id))).execute(c)?;
                for g in &b.group_ids {
                    diesel::insert_into(policy_groups::table).values((policy_groups::policy_id.eq(id), policy_groups::group_id.eq(g))).on_conflict_do_nothing().execute(c)?;
                }
                audit::record(c, Event::new(&user.username, "policy.updated").target("policy", &id.to_string()).details(json!({"name": p.name, "groups": b.group_ids, "definition": canonical, "enabled": p.enabled})).ip(&user.ip))?;
                Ok(PolicyOut { policy: p, group_ids: b.group_ids })
            })
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

pub async fn delete_policy(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("policies:delete")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            if diesel::delete(policies::table.find(id)).execute(c)? == 0 {
                return Err(ApiError::NotFound("policy"));
            }
            audit::record(c, Event::new(&user.username, "policy.deleted").target("policy", &id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Deserialize)]
pub struct SimulateBody {
    host_id: Uuid,
    definition: Value,
}

/// "What would this policy do to that host right now?" — nothing is created.
pub async fn simulate(state: web::Data<AppState>, user: AuthUser, body: web::Json<SimulateBody>) -> ApiResult<HttpResponse> {
    user.require("policies:read")?;
    let b = body.into_inner();
    let def = parse_def(&b.definition)?;
    let out = state.db(move |c| planner::simulate(c, b.host_id, &def)).await?;
    Ok(HttpResponse::Ok().json(out))
}

// ------------------------------------------------------------------------ tasks

#[derive(Serialize)]
struct TaskRow {
    #[serde(flatten)]
    task: Task,
    hostname: String,
}

#[derive(Deserialize)]
pub struct TaskQuery {
    status: Option<String>,
    kind: Option<String>,
    host_id: Option<Uuid>,
    limit: Option<i64>,
}

pub async fn list_tasks(state: web::Data<AppState>, user: AuthUser, q: web::Query<TaskQuery>) -> ApiResult<HttpResponse> {
    user.require("tasks:read")?;
    let q = q.into_inner();
    let out = state
        .db(move |c| {
            let mut query = tasks::table.inner_join(hosts::table).into_boxed();
            if let Some(s) = q.status {
                // "open" is a convenience alias for every non-terminal status
                if s == "open" {
                    query = query.filter(tasks::status.eq_any(OPEN));
                } else {
                    query = query.filter(tasks::status.eq(s));
                }
            }
            if let Some(k) = q.kind {
                query = query.filter(tasks::kind.eq(k));
            }
            if let Some(h) = q.host_id {
                query = query.filter(tasks::host_id.eq(h));
            }
            let rows: Vec<(Task, String)> = query.order(tasks::created_at.desc()).limit(q.limit.unwrap_or(100).clamp(1, 500)).select((Task::as_select(), hosts::hostname)).load(c)?;
            Ok(rows.into_iter().map(|(task, hostname)| TaskRow { task, hostname }).collect::<Vec<_>>())
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

pub async fn get_task(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:read")?;
    let id = path.into_inner();
    let out = state
        .db(move |c| {
            let (task, hostname): (Task, String) = tasks::table.inner_join(hosts::table).filter(tasks::id.eq(id)).select((Task::as_select(), hosts::hostname)).first(c).map_err(|_| ApiError::NotFound("task"))?;
            let events: Vec<TaskEvent> = task_events::table.filter(task_events::task_id.eq(id)).order(task_events::id.asc()).select(TaskEvent::as_select()).load(c)?;
            Ok(json!({ "task": TaskRow { task, hostname }, "events": events }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize)]
pub struct NewTasks {
    host_ids: Vec<Uuid>,
    /// Only "patch" is accepted here; scans have their own endpoint.
    #[serde(default = "patch_kind")]
    kind: String,
    /// Empty = every pending update.
    #[serde(default)]
    packages: Vec<String>,
}
fn patch_kind() -> String {
    "patch".into()
}

/// Operator-initiated patching. Explicit human action: maintenance windows and approval are bypassed,
/// the audit trail records who did it.
pub async fn create_tasks(state: web::Data<AppState>, user: AuthUser, body: web::Json<NewTasks>) -> ApiResult<HttpResponse> {
    user.require("tasks:create")?;
    let b = body.into_inner();
    if b.kind != "patch" {
        return Err(ApiError::bad("only kind 'patch' can be created here"));
    }
    if b.host_ids.is_empty() || b.host_ids.len() > 200 {
        return Err(ApiError::bad("host_ids must contain 1..=200 hosts"));
    }
    if let Some(bad) = b.packages.iter().find(|p| !is_valid_package_name(p)) {
        return Err(ApiError::bad(format!("invalid package name: {bad}")));
    }
    let host_ids = b.host_ids.clone();
    let ids: Vec<(Uuid, Uuid)> = state
        .db(move |c| {
            let mut created = vec![];
            for host_id in b.host_ids {
                let r = c.transaction::<_, ApiError, _>(|c| {
                    let host: crate::models::Host = hosts::table.find(host_id).select(crate::models::Host::as_select()).first(c).map_err(|_| ApiError::NotFound("host"))?;
                    if matches!(host.status.as_str(), "pending" | "quarantined") {
                        return Err(ApiError::Conflict(format!("{} is {}", host.hostname, host.status)));
                    }
                    let open: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq_any(["patch", "rollback"])).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
                    if open > 0 {
                        return Err(ApiError::Conflict(format!("{} already has an open patch task", host.hostname)));
                    }
                    let mut q = host_packages::table.filter(host_packages::host_id.eq(host_id)).filter(host_packages::candidate.is_not_null()).into_boxed();
                    if !b.packages.is_empty() {
                        q = q.filter(host_packages::name.eq_any(b.packages.clone()));
                    }
                    let pending: Vec<crate::models::HostPackage> = q.select(crate::models::HostPackage::as_select()).load(c)?;
                    if pending.is_empty() {
                        return Err(ApiError::Conflict(format!("{} has nothing to update", host.hostname)));
                    }
                    let batch: Vec<planner::Candidate> = pending.into_iter().filter_map(|p| p.candidate.clone().map(|candidate| planner::Candidate { pkg: p, candidate })).collect();
                    let groups = planner::host_groups(c, host_id)?;
                    let def = planner::applicable_policy(c, &host, &groups)?;
                    let exec: ExecPolicy = def.as_ref().map(|p| planner::exec_policy(&p.def, host.has_staging, &host.criticality)).unwrap_or_default();
                    let id: Uuid = diesel::insert_into(tasks::table)
                        .values((
                            tasks::host_id.eq(host_id),
                            tasks::kind.eq("patch"),
                            tasks::status.eq("queued"),
                            tasks::policy_id.eq(def.as_ref().map(|d| d.id)),
                            tasks::packages.eq(planner::packages_json(&batch)),
                            tasks::exec.eq(Some(serde_json::to_value(&exec).unwrap_or_default())),
                            tasks::requested_by.eq(&user.username),
                            tasks::message.eq(format!("manual patch by {}", user.username)),
                        ))
                        .returning(tasks::id)
                        .get_result(c)?;
                    diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(format!("requested by {}", user.username)))).execute(c)?;
                    audit::record(c, Event::new(&user.username, "task.patch_requested").target("task", &id.to_string()).host(host_id).details(json!({"packages": batch.len()})).ip(&user.ip))?;
                    Ok(id)
                });
                created.push(r.map(|id| (host_id, id)));
            }
            // Report partial failures instead of hiding them.
            let (ok, errs): (Vec<_>, Vec<_>) = created.into_iter().partition(|r| r.is_ok());
            if ok.is_empty() {
                return Err(errs.into_iter().next().unwrap().unwrap_err());
            }
            Ok(ok.into_iter().map(|r| r.unwrap()).collect())
        })
        .await?;
    for h in &host_ids {
        gateway::push_pending(&state, *h).await;
    }
    Ok(HttpResponse::Accepted().json(json!({ "tasks": ids.iter().map(|(h, t)| json!({"host_id": h, "task_id": t})).collect::<Vec<_>>() })))
}

/// Move a task between states, guarded by its current state.
fn transition(c: &mut PgConnection, user: &AuthUser, id: Uuid, from: &[&str], to: &str, action: &str, extra: impl FnOnce(&mut PgConnection, &Task) -> ApiResult<()>) -> ApiResult<Uuid> {
    c.transaction::<_, ApiError, _>(|c| {
        let task: Task = tasks::table.find(id).for_update().select(Task::as_select()).first(c).map_err(|_| ApiError::NotFound("task"))?;
        if !from.contains(&task.status.as_str()) {
            return Err(ApiError::Conflict(format!("task is {}, expected {}", task.status, from.join(" or "))));
        }
        let now = Utc::now();
        diesel::update(tasks::table.find(id)).set((tasks::status.eq(to), tasks::updated_at.eq(now), tasks::finished_at.eq(matches!(to, "rejected" | "cancelled").then_some(now)))).execute(c)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq(to), task_events::message.eq(format!("{action} by {}", user.username)))).execute(c)?;
        extra(c, &task)?;
        audit::record(c, Event::new(&user.username, action).target("task", &id.to_string()).host(task.host_id).ip(&user.ip))?;
        Ok(task.host_id)
    })
}

pub async fn approve_task(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:write")?;
    let id = path.into_inner();
    let name = user.username.clone();
    let host_id = state
        .db(move |c| {
            transition(c, &user, id, &["pending_approval"], "queued", "task.approved", |c, t| {
                let def: Option<PolicyDef> = match t.policy_id {
                    Some(p) => policies::table.find(p).select(policies::definition).first::<Value>(c).optional()?.and_then(|d| serde_json::from_value(d).ok()),
                    None => None,
                };
                let ring: i32 = hosts::table.find(t.host_id).select(hosts::ring).first(c)?;
                let when = planner::compute_schedule(def.as_ref(), ring, Utc::now());
                diesel::update(tasks::table.find(t.id)).set((tasks::approved_by.eq(Some(&name)), tasks::scheduled_for.eq(when))).execute(c)?;
                Ok(())
            })
        })
        .await?;
    gateway::push_pending(&state, host_id).await;
    Ok(HttpResponse::NoContent().finish())
}

pub async fn reject_task(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:write")?;
    let id = path.into_inner();
    state.db(move |c| transition(c, &user, id, &["pending_approval"], "rejected", "task.rejected", |_, _| Ok(()))).await?;
    Ok(HttpResponse::NoContent().finish())
}

pub async fn cancel_task(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:write")?;
    let id = path.into_inner();
    state.db(move |c| transition(c, &user, id, &["pending_approval", "queued"], "cancelled", "task.cancelled", |_, _| Ok(()))).await?;
    Ok(HttpResponse::NoContent().finish())
}

/// Ask the agent to revert a patch task (manual rollback).
pub async fn rollback_task(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:create")?;
    let parent = path.into_inner();
    let (host_id, new_id) = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                let t: Task = tasks::table.find(parent).select(Task::as_select()).first(c).map_err(|_| ApiError::NotFound("task"))?;
                if t.kind != "patch" || !matches!(t.status.as_str(), "succeeded" | "failed" | "soaking") {
                    return Err(ApiError::Conflict("only a finished or soaking patch task can be rolled back".into()));
                }
                let open: i64 = tasks::table.filter(tasks::host_id.eq(t.host_id)).filter(tasks::kind.eq_any(["patch", "rollback"])).filter(tasks::status.eq_any(OPEN)).filter(tasks::id.ne(parent)).count().get_result(c)?;
                if open > 0 {
                    return Err(ApiError::Conflict("the host has another open task".into()));
                }
                let id: Uuid = diesel::insert_into(tasks::table)
                    .values((tasks::host_id.eq(t.host_id), tasks::kind.eq("rollback"), tasks::status.eq("queued"), tasks::parent_task_id.eq(Some(parent)), tasks::packages.eq(&t.packages), tasks::requested_by.eq(&user.username), tasks::message.eq(format!("rollback of {parent}"))))
                    .returning(tasks::id)
                    .get_result(c)?;
                diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(format!("rollback requested by {}", user.username)))).execute(c)?;
                audit::record(c, Event::new(&user.username, "task.rollback_requested").target("task", &id.to_string()).host(t.host_id).details(json!({"parent": parent})).ip(&user.ip))?;
                Ok((t.host_id, id))
            })
        })
        .await?;
    gateway::push_pending(&state, host_id).await;
    Ok(HttpResponse::Accepted().json(json!({ "task_id": new_id })))
}

// ---------------------------------------------------------------- vulnerabilities

#[derive(Deserialize)]
pub struct VulnQuery {
    q: Option<String>,
    limit: Option<i64>,
}

pub async fn list_vulns(state: web::Data<AppState>, user: AuthUser, q: web::Query<VulnQuery>) -> ApiResult<HttpResponse> {
    user.require("vulnerabilities:read")?;
    let q = q.into_inner();
    let out = state
        .db(move |c| {
            let mut query = vulnerabilities::table.into_boxed();
            if let Some(t) = q.q.filter(|t| !t.trim().is_empty()) {
                let like = format!("%{}%", t.trim().replace('%', "\\%").replace('_', "\\_"));
                query = query.filter(vulnerabilities::cve_id.ilike(like.clone()).or(vulnerabilities::summary.ilike(like)));
            }
            let rows: Vec<Vulnerability> = query.order((vulnerabilities::cvss.desc().nulls_last(), vulnerabilities::cve_id.desc())).limit(q.limit.unwrap_or(200).clamp(1, 1000)).select(Vulnerability::as_select()).load(c)?;
            // How many hosts still carry each CVE (unpatched = update pending)?
            let pend: Vec<(Uuid, Value)> = host_packages::table.filter(host_packages::candidate.is_not_null()).select((host_packages::host_id, host_packages::cves)).load(c)?;
            let mut exposure: HashMap<String, std::collections::HashSet<Uuid>> = HashMap::new();
            for (h, cves) in pend {
                for cve in cves.as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
                    exposure.entry(cve.to_string()).or_default().insert(h);
                }
            }
            Ok(rows.into_iter().map(|v| { let n = exposure.get(&v.cve_id).map(|s| s.len()).unwrap_or(0); json!({ "vulnerability": v, "exposed_hosts": n }) }).collect::<Vec<_>>())
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize)]
pub struct VulnImport {
    cve_id: String,
    cvss: Option<f64>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    exploited: bool,
    published_at: Option<DateTime<Utc>>,
    source: Option<String>,
}

/// Import a CVE feed (air-gapped mode: pushed from a bastion; connected mode: any script).
pub async fn import_vulns(state: web::Data<AppState>, user: AuthUser, body: web::Json<Vec<VulnImport>>) -> ApiResult<HttpResponse> {
    user.require("vulnerabilities:write")?;
    let items = body.into_inner();
    if items.len() > 50_000 {
        return Err(ApiError::bad("at most 50000 entries per import"));
    }
    let re = regex::Regex::new(r"^CVE-\d{4}-\d{4,}$").unwrap();
    if let Some(bad) = items.iter().find(|i| !re.is_match(&i.cve_id) || i.cvss.is_some_and(|s| !(0.0..=10.0).contains(&s))) {
        return Err(ApiError::bad(format!("invalid entry: {}", bad.cve_id)));
    }
    let n = items.len();
    state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                for chunk in items.chunks(1000) {
                    for i in chunk {
                        let sev = i.cvss.map(|s| croustypatch_common::model::RiskLevel::from_cvss(s).as_str().to_ascii_lowercase()).unwrap_or_else(|| "unknown".into());
                        diesel::insert_into(vulnerabilities::table)
                            .values((vulnerabilities::cve_id.eq(&i.cve_id), vulnerabilities::cvss.eq(i.cvss), vulnerabilities::severity.eq(&sev), vulnerabilities::summary.eq(&i.summary), vulnerabilities::exploited.eq(i.exploited), vulnerabilities::published_at.eq(i.published_at), vulnerabilities::source.eq(i.source.clone().unwrap_or_else(|| "import".into()))))
                            .on_conflict(vulnerabilities::cve_id)
                            .do_update()
                            .set((vulnerabilities::cvss.eq(i.cvss), vulnerabilities::severity.eq(&sev), vulnerabilities::summary.eq(&i.summary), vulnerabilities::exploited.eq(i.exploited), vulnerabilities::published_at.eq(i.published_at), vulnerabilities::updated_at.eq(Utc::now())))
                            .execute(c)?;
                    }
                }
                audit::record(c, Event::new(&user.username, "vulnerabilities.imported").details(json!({"count": n})).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::Ok().json(json!({ "imported": n })))
}

// ---------------------------------------------------------------------- waivers

pub async fn list_waivers(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("waivers:read")?;
    let rows: Vec<Waiver> = state.db(|c| Ok(waivers::table.order(waivers::created_at.desc()).limit(500).select(Waiver::as_select()).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows))
}

#[derive(Deserialize)]
pub struct NewWaiver {
    host_id: Option<Uuid>,
    group_id: Option<Uuid>,
    cve_id: Option<String>,
    package: Option<String>,
    reason: String,
    compensating_controls: String,
    expires_at: DateTime<Utc>,
}

pub const MAX_WAIVER_DAYS: i64 = 90;

pub async fn create_waiver(state: web::Data<AppState>, user: AuthUser, body: web::Json<NewWaiver>) -> ApiResult<HttpResponse> {
    user.require("waivers:create")?;
    let b = body.into_inner();
    if b.host_id.is_none() == b.group_id.is_none() {
        return Err(ApiError::bad("target exactly one of host_id or group_id"));
    }
    if b.cve_id.is_none() && b.package.is_none() {
        return Err(ApiError::bad("a waiver must name a cve_id or a package"));
    }
    if b.package.as_deref().is_some_and(|p| !is_valid_package_name(p)) {
        return Err(ApiError::bad("invalid package name"));
    }
    if b.reason.trim().len() < 10 {
        return Err(ApiError::bad("a justification of at least 10 characters is required"));
    }
    if b.compensating_controls.trim().len() < 10 {
        return Err(ApiError::bad("compensating controls are mandatory (e.g. temporary WAF rule)"));
    }
    let now = Utc::now();
    if b.expires_at <= now || b.expires_at > now + Duration::days(MAX_WAIVER_DAYS) {
        return Err(ApiError::bad(format!("waivers are time-boxed: expiry must be within {MAX_WAIVER_DAYS} days")));
    }
    let w: Waiver = state
        .db(move |c| {
            let w: Waiver = diesel::insert_into(waivers::table)
                .values((waivers::host_id.eq(b.host_id), waivers::group_id.eq(b.group_id), waivers::cve_id.eq(&b.cve_id), waivers::package.eq(&b.package), waivers::reason.eq(b.reason.trim()), waivers::compensating_controls.eq(b.compensating_controls.trim()), waivers::requested_by.eq(&user.username), waivers::expires_at.eq(b.expires_at)))
                .returning(Waiver::as_returning())
                .get_result(c)?;
            audit::record(c, Event::new(&user.username, "waiver.requested").target("waiver", &w.id.to_string()).details(json!({"cve": w.cve_id, "package": w.package, "expires_at": w.expires_at})).ip(&user.ip))?;
            Ok(w)
        })
        .await?;
    Ok(HttpResponse::Created().json(w))
}

/// Dual control: the CISO and a business owner must both approve, and not be the requester.
pub async fn approve_waiver(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let ciso = user.can("waivers:approve_ciso");
    let biz = user.can("waivers:approve_business");
    if !ciso && !biz {
        return Err(ApiError::forbidden("missing permission waivers:approve_ciso or waivers:approve_business"));
    }
    let w: Waiver = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                let w: Waiver = waivers::table.find(id).for_update().select(Waiver::as_select()).first(c).map_err(|_| ApiError::NotFound("waiver"))?;
                if w.status != "pending" || w.expires_at <= Utc::now() {
                    return Err(ApiError::Conflict(format!("waiver is {}", w.status)));
                }
                if w.requested_by == user.username {
                    return Err(ApiError::forbidden("the requester cannot approve their own waiver"));
                }
                let now = Utc::now();
                let mut approved_as = vec![];
                // A holder of both permissions fills the CISO slot first; the business slot needs someone else.
                if ciso && w.ciso_approved_by.is_none() && w.business_approved_by.as_deref() != Some(&user.username) {
                    diesel::update(waivers::table.find(id)).set((waivers::ciso_approved_by.eq(Some(&user.username)), waivers::ciso_approved_at.eq(Some(now)))).execute(c)?;
                    approved_as.push("ciso");
                } else if biz && w.business_approved_by.is_none() && w.ciso_approved_by.as_deref() != Some(&user.username) {
                    diesel::update(waivers::table.find(id)).set((waivers::business_approved_by.eq(Some(&user.username)), waivers::business_approved_at.eq(Some(now)))).execute(c)?;
                    approved_as.push("business");
                } else {
                    return Err(ApiError::Conflict("no approval slot available for you (each slot needs a different person)".into()));
                }
                let updated: Waiver = waivers::table.find(id).select(Waiver::as_select()).first(c)?;
                if updated.ciso_approved_by.is_some() && updated.business_approved_by.is_some() {
                    diesel::update(waivers::table.find(id)).set(waivers::status.eq("approved")).execute(c)?;
                }
                audit::record(c, Event::new(&user.username, "waiver.approval").target("waiver", &id.to_string()).details(json!({"as": approved_as})).ip(&user.ip))?;
                Ok(waivers::table.find(id).select(Waiver::as_select()).first(c)?)
            })
        })
        .await?;
    Ok(HttpResponse::Ok().json(w))
}

async fn close_waiver(state: web::Data<AppState>, user: AuthUser, id: Uuid, from: &'static [&'static str], to: &'static str, perm: &'static str) -> ApiResult<HttpResponse> {
    if !user.can(perm) && !user.can("waivers:approve_ciso") && !user.can("waivers:approve_business") {
        return Err(ApiError::forbidden(format!("missing permission {perm}")));
    }
    state
        .db(move |c| {
            let n = diesel::update(waivers::table.find(id).filter(waivers::status.eq_any(from))).set(waivers::status.eq(to)).execute(c)?;
            if n == 0 {
                return Err(ApiError::Conflict("waiver is not in a state that allows this".into()));
            }
            audit::record(c, Event::new(&user.username, &format!("waiver.{to}")).target("waiver", &id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

pub async fn reject_waiver(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    close_waiver(state, user, path.into_inner(), &["pending"], "rejected", "waivers:write").await
}

pub async fn revoke_waiver(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    close_waiver(state, user, path.into_inner(), &["pending", "approved"], "revoked", "waivers:write").await
}
