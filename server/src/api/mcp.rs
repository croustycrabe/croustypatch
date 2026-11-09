//! Model Context Protocol server (spec stack_technique §3): lets an AI assistant (Claude, a local agent…) read the
//! fleet and *request* patches, through JSON-RPC 2.0 over HTTP (`POST /api/v1/mcp`, the "streamable HTTP" transport
//! answered with plain JSON).
//!
//! Authentication: a personal API token (`Authorization: Bearer cpat_…`), so every call runs as a real user with
//! that user's role. Nothing an assistant asks for bypasses people: patch requests always wait for approval.
use crate::audit::{self, Event};
use crate::auth::{AuthUser, API_TOKEN_PREFIX};
use crate::error::{ApiError, ApiResult};
use crate::models::{Host, HostPackage, Task};
use crate::planner::{self, OPEN};
use crate::schema::{api_tokens, host_packages, hosts, task_events, tasks};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::{Duration, Utc};
use croustypatch_common::model::is_valid_package_name;
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub const PROTOCOL_VERSION: &str = "2025-06-18";

// ------------------------------------------------------------------ API tokens

#[derive(Deserialize)]
pub struct NewToken {
    name: String,
    /// Lifetime in days (default 90, max 365).
    days: Option<i64>,
}

pub async fn list_tokens(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    let uid = user.id;
    type Row = (Uuid, String, String, Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>, chrono::DateTime<Utc>);
    let rows: Vec<Row> = state
        .db(move |c| Ok(api_tokens::table.filter(api_tokens::user_id.eq(uid)).filter(api_tokens::revoked.eq(false)).order(api_tokens::created_at.desc()).select((api_tokens::id, api_tokens::name, api_tokens::prefix, api_tokens::expires_at, api_tokens::last_used_at, api_tokens::created_at)).load(c)?))
        .await?;
    Ok(HttpResponse::Ok().json(rows.into_iter().map(|r| json!({ "id": r.0, "name": r.1, "prefix": r.2, "expires_at": r.3, "last_used_at": r.4, "created_at": r.5 })).collect::<Vec<_>>()))
}

pub async fn create_token(state: web::Data<AppState>, user: AuthUser, body: web::Json<NewToken>) -> ApiResult<HttpResponse> {
    let b = body.into_inner();
    super::name_ok("name", &b.name)?;
    let days = b.days.unwrap_or(90);
    if !(1..=365).contains(&days) {
        return Err(ApiError::bad("days must be 1-365"));
    }
    let token = format!("{API_TOKEN_PREFIX}{}", croustypatch_common::crypto::random_token(32));
    let hash = croustypatch_common::crypto::sha256_hex(token.as_bytes());
    let prefix: String = token.chars().take(API_TOKEN_PREFIX.len() + 6).collect();
    let expires = Utc::now() + Duration::days(days);
    let uid = user.id;
    let name = b.name.trim().to_string();
    let id: Uuid = state
        .db(move |c| {
            c.transaction(|c| {
                let n: i64 = api_tokens::table.filter(api_tokens::user_id.eq(uid)).filter(api_tokens::revoked.eq(false)).count().get_result(c)?;
                if n >= 20 {
                    return Err(ApiError::Conflict("at most 20 active tokens per account".into()));
                }
                let id: Uuid = diesel::insert_into(api_tokens::table).values((api_tokens::user_id.eq(uid), api_tokens::name.eq(&name), api_tokens::token_hash.eq(&hash), api_tokens::prefix.eq(&prefix), api_tokens::expires_at.eq(Some(expires)))).returning(api_tokens::id).get_result(c)?;
                audit::record(c, Event::new(&user.username, "api_token.created").target("api_token", &id.to_string()).details(json!({ "name": name, "days": days })).ip(&user.ip))?;
                Ok(id)
            })
        })
        .await?;
    Ok(HttpResponse::Created().json(json!({ "id": id, "token": token, "expires_at": expires })))
}

pub async fn revoke_token(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let uid = user.id;
    state
        .db(move |c| {
            c.transaction(|c| {
                if diesel::update(api_tokens::table.find(id).filter(api_tokens::user_id.eq(uid))).set(api_tokens::revoked.eq(true)).execute(c)? == 0 {
                    return Err(ApiError::NotFound("token"));
                }
                audit::record(c, Event::new(&user.username, "api_token.revoked").target("api_token", &id.to_string()).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

// ------------------------------------------------------------------ MCP

fn tools() -> Value {
    let obj = |props: Value, req: &[&str]| json!({ "type": "object", "properties": props, "required": req, "additionalProperties": false });
    json!([
        { "name": "fleet_summary", "title": "Fleet summary", "description": "Counts of hosts by status, pending and security updates, tasks awaiting approval.", "inputSchema": obj(json!({}), &[]), "annotations": { "readOnlyHint": true } },
        { "name": "list_hosts", "title": "List hosts", "description": "Managed hosts with status, OS, pending and security update counts. Optional filter on status or a name fragment.", "inputSchema": obj(json!({ "status": { "type": "string", "enum": ["online","offline","eco","maintenance","alert","quarantined","pending"] }, "search": { "type": "string" } }), &[]), "annotations": { "readOnlyHint": true } },
        { "name": "get_host", "title": "Host details", "description": "One host (by hostname or id): attributes, pending updates with CVEs, recent tasks.", "inputSchema": obj(json!({ "host": { "type": "string" } }), &["host"]), "annotations": { "readOnlyHint": true } },
        { "name": "list_tasks", "title": "List tasks", "description": "Recent remediation tasks, newest first. Optional status filter (e.g. pending_approval, failed).", "inputSchema": obj(json!({ "status": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 100 } }), &[]), "annotations": { "readOnlyHint": true } },
        { "name": "get_task", "title": "Task details", "description": "One task: packages, the policy decision and its reasoning, the agent's log tail.", "inputSchema": obj(json!({ "task_id": { "type": "string" } }), &["task_id"]), "annotations": { "readOnlyHint": true } },
        { "name": "request_patch", "title": "Request a patch", "description": "Ask for updates on one host. The task is created AWAITING HUMAN APPROVAL; nothing is installed until a person approves it in CroustyPatch. Leave packages empty for every pending update.", "inputSchema": obj(json!({ "host": { "type": "string" }, "packages": { "type": "array", "items": { "type": "string" } }, "reason": { "type": "string", "maxLength": 300 } }), &["host", "reason"]), "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false } },
    ])
}

fn rpc_ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}
fn rpc_err(id: &Value, code: i64, msg: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } })
}
fn tool_text(v: Value, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "structuredContent": v, "isError": is_error })
}

/// POST /api/v1/mcp — one JSON-RPC message (or a batch).
pub async fn endpoint(state: web::Data<AppState>, user: AuthUser, body: web::Json<Value>) -> ApiResult<HttpResponse> {
    let msg = body.into_inner();
    if let Value::Array(batch) = msg {
        let mut out = vec![];
        for m in batch {
            if let Some(r) = handle(&state, &user, m).await {
                out.push(r);
            }
        }
        return Ok(if out.is_empty() { HttpResponse::Accepted().finish() } else { HttpResponse::Ok().json(out) });
    }
    Ok(match handle(&state, &user, msg).await {
        Some(r) => HttpResponse::Ok().json(r),
        None => HttpResponse::Accepted().finish(), // notifications get no answer
    })
}

/// GET on the MCP URL: this server does not open server-initiated streams.
pub async fn no_stream() -> HttpResponse {
    HttpResponse::MethodNotAllowed().insert_header(("allow", "POST")).finish()
}

async fn handle(state: &web::Data<AppState>, user: &AuthUser, m: Value) -> Option<Value> {
    let id = m.get("id").cloned();
    let method = m.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let Some(id) = id else { return None }; // notification (e.g. notifications/initialized)
    let params = m.get("params").cloned().unwrap_or(json!({}));
    Some(match method {
        "initialize" => rpc_ok(&id, json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "croustypatch", "title": "CroustyPatch", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!("You are connected to CroustyPatch as {} (role {}). You can read the fleet and request patches; every patch request waits for a human approval.", user.username, user.role),
        })),
        "ping" => rpc_ok(&id, json!({})),
        "tools/list" => rpc_ok(&id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call(state, user, &name, args).await {
                Ok(v) => rpc_ok(&id, tool_text(v, false)),
                Err(CallError::Unknown) => rpc_err(&id, -32602, &format!("unknown tool {name}")),
                Err(CallError::Api(e)) => rpc_ok(&id, tool_text(json!({ "error": e.to_string() }), true)),
            }
        }
        _ => rpc_err(&id, -32601, "method not found"),
    })
}

enum CallError {
    Unknown,
    Api(ApiError),
}
impl From<ApiError> for CallError {
    fn from(e: ApiError) -> Self {
        CallError::Api(e)
    }
}

fn find_host(c: &mut PgConnection, key: &str) -> ApiResult<Host> {
    let q = hosts::table.select(Host::as_select());
    let found = match key.parse::<Uuid>() {
        Ok(id) => q.filter(hosts::id.eq(id)).first(c).optional()?,
        Err(_) => q.filter(hosts::hostname.eq(key).or(hosts::display_name.eq(key))).first(c).optional()?,
    };
    found.ok_or(ApiError::NotFound("host"))
}

async fn call(state: &web::Data<AppState>, user: &AuthUser, name: &str, args: Value) -> Result<Value, CallError> {
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    match name {
        "fleet_summary" => {
            user.require("hosts:read")?;
            Ok(state
                .db(|c| {
                    let by_status: Vec<(String, i64)> = hosts::table.group_by(hosts::status).select((hosts::status, diesel::dsl::count_star())).load(c)?;
                    let pending: i64 = host_packages::table.filter(host_packages::candidate.is_not_null()).count().get_result(c)?;
                    let security: i64 = host_packages::table.filter(host_packages::candidate.is_not_null()).filter(host_packages::is_security.eq(true)).count().get_result(c)?;
                    let approvals: i64 = tasks::table.filter(tasks::status.eq("pending_approval")).count().get_result(c)?;
                    let reboot: i64 = hosts::table.filter(hosts::reboot_required.eq(true)).count().get_result(c)?;
                    Ok(json!({ "hosts_by_status": by_status.into_iter().collect::<std::collections::HashMap<_, _>>(), "pending_updates": pending, "security_updates": security, "tasks_awaiting_approval": approvals, "hosts_needing_reboot": reboot }))
                })
                .await?)
        }
        "list_hosts" => {
            user.require("hosts:read")?;
            let (status, search) = (s("status"), s("search").map(|x| x.to_lowercase()));
            Ok(state
                .db(move |c| {
                    let hs: Vec<Host> = hosts::table.select(Host::as_select()).order(hosts::hostname.asc()).load(c)?;
                    let counts: Vec<(Uuid, bool)> = host_packages::table.filter(host_packages::candidate.is_not_null()).select((host_packages::host_id, host_packages::is_security)).load(c)?;
                    Ok(json!(hs
                        .into_iter()
                        .filter(|h| status.as_deref().is_none_or(|st| h.status == st))
                        .filter(|h| search.as_deref().is_none_or(|q| h.hostname.to_lowercase().contains(q) || h.display_name.as_deref().unwrap_or("").to_lowercase().contains(q)))
                        .map(|h| {
                            let mine: Vec<&(Uuid, bool)> = counts.iter().filter(|x| x.0 == h.id).collect();
                            json!({ "id": h.id, "hostname": h.hostname, "status": h.status, "os": h.distro, "type": h.asset_type, "environment": h.environment, "pending_updates": mine.len(), "security_updates": mine.iter().filter(|x| x.1).count(), "reboot_required": h.reboot_required, "last_seen_at": h.last_seen_at })
                        })
                        .collect::<Vec<_>>()))
                })
                .await?)
        }
        "get_host" => {
            user.require("hosts:read")?;
            let key = s("host").ok_or(ApiError::bad("host is required"))?;
            Ok(state
                .db(move |c| {
                    let h = find_host(c, &key)?;
                    let pkgs: Vec<HostPackage> = host_packages::table.filter(host_packages::host_id.eq(h.id)).filter(host_packages::candidate.is_not_null()).order(host_packages::name.asc()).select(HostPackage::as_select()).load(c)?;
                    let recent: Vec<Task> = tasks::table.filter(tasks::host_id.eq(h.id)).order(tasks::created_at.desc()).limit(10).select(Task::as_select()).load(c)?;
                    Ok(json!({
                        "id": h.id, "hostname": h.hostname, "status": h.status, "os": h.distro, "type": h.asset_type, "environment": h.environment, "criticality": h.criticality, "internet_exposed": h.internet_exposed, "reboot_required": h.reboot_required, "last_seen_at": h.last_seen_at,
                        "pending_updates": pkgs.iter().map(|p| json!({ "name": p.name, "installed": p.version, "candidate": p.candidate, "security": p.is_security, "cves": p.cves })).collect::<Vec<_>>(),
                        "recent_tasks": recent.iter().map(|t| json!({ "id": t.id, "kind": t.kind, "status": t.status, "message": t.message, "created_at": t.created_at })).collect::<Vec<_>>(),
                    }))
                })
                .await?)
        }
        "list_tasks" => {
            user.require("tasks:read")?;
            let status = s("status");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20).clamp(1, 100);
            Ok(state
                .db(move |c| {
                    let mut q = tasks::table.inner_join(hosts::table).order(tasks::created_at.desc()).limit(limit).into_boxed();
                    if let Some(st) = status {
                        q = q.filter(tasks::status.eq(st));
                    }
                    let rows: Vec<(Task, String)> = q.select((Task::as_select(), hosts::hostname)).load(c)?;
                    Ok(json!(rows.into_iter().map(|(t, h)| json!({ "id": t.id, "host": h, "kind": t.kind, "status": t.status, "message": t.message, "packages": t.packages.as_array().map(|a| a.len()).unwrap_or(0), "created_at": t.created_at })).collect::<Vec<_>>()))
                })
                .await?)
        }
        "get_task" => {
            user.require("tasks:read")?;
            let id: Uuid = s("task_id").and_then(|x| x.parse().ok()).ok_or(ApiError::bad("task_id must be a task id"))?;
            Ok(state
                .db(move |c| {
                    let (t, h): (Task, String) = tasks::table.inner_join(hosts::table).filter(tasks::id.eq(id)).select((Task::as_select(), hosts::hostname)).first(c).map_err(|_| ApiError::NotFound("task"))?;
                    Ok(json!({ "id": t.id, "host": h, "kind": t.kind, "status": t.status, "message": t.message, "packages": t.packages, "decision": t.decision, "requested_by": t.requested_by, "approved_by": t.approved_by, "log_tail": t.result.as_ref().and_then(|r| r.get("log_tail")).cloned(), "created_at": t.created_at, "finished_at": t.finished_at }))
                })
                .await?)
        }
        "request_patch" => {
            user.require("tasks:create")?;
            let key = s("host").ok_or(ApiError::bad("host is required"))?;
            let reason: String = s("reason").unwrap_or_default().chars().filter(|c| !c.is_control()).take(300).collect();
            if reason.trim().is_empty() {
                return Err(ApiError::bad("reason is required").into());
            }
            let names: Vec<String> = args.get("packages").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            if let Some(bad) = names.iter().find(|p| !is_valid_package_name(p)) {
                return Err(ApiError::bad(format!("invalid package name: {bad}")).into());
            }
            let actor = format!("mcp:{}", user.username);
            let ip = user.ip.clone();
            Ok(state
                .db(move |c| {
                    c.transaction(|c| {
                        let h = find_host(c, &key)?;
                        if matches!(h.status.as_str(), "pending" | "quarantined") {
                            return Err(ApiError::Conflict(format!("{} is {}", h.hostname, h.status)));
                        }
                        let open: i64 = tasks::table.filter(tasks::host_id.eq(h.id)).filter(tasks::kind.eq_any(["patch", "rollback"])).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
                        if open > 0 {
                            return Err(ApiError::Conflict(format!("{} already has an open patch task", h.hostname)));
                        }
                        let mut q = host_packages::table.filter(host_packages::host_id.eq(h.id)).filter(host_packages::candidate.is_not_null()).into_boxed();
                        if !names.is_empty() {
                            q = q.filter(host_packages::name.eq_any(names.clone()));
                        }
                        let pending: Vec<HostPackage> = q.select(HostPackage::as_select()).load(c)?;
                        if pending.is_empty() {
                            return Err(ApiError::Conflict(format!("{} has nothing to update", h.hostname)));
                        }
                        let batch: Vec<planner::Candidate> = pending.into_iter().filter_map(|p| p.candidate.clone().map(|candidate| planner::Candidate { pkg: p, candidate })).collect();
                        let groups = planner::host_groups(c, h.id)?;
                        let pol = planner::applicable_policy(c, &h, &groups)?;
                        let exec = pol.as_ref().map(|p| planner::exec_policy(&p.def, h.has_staging, &h.criticality)).unwrap_or_default();
                        let msg = format!("requested via MCP by {actor}: {reason}");
                        let id: Uuid = diesel::insert_into(tasks::table)
                            .values((tasks::host_id.eq(h.id), tasks::kind.eq("patch"), tasks::status.eq("pending_approval"), tasks::policy_id.eq(pol.as_ref().map(|p| p.id)), tasks::packages.eq(planner::packages_json(&batch)), tasks::exec.eq(Some(serde_json::to_value(&exec).unwrap_or_default())), tasks::requested_by.eq(&actor), tasks::message.eq(&msg)))
                            .returning(tasks::id)
                            .get_result(c)?;
                        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("pending_approval"), task_events::message.eq(&msg))).execute(c)?;
                        audit::record(c, Event::new(&actor, "task.planned").target("task", &id.to_string()).host(h.id).details(json!({ "action": "REQUIRE_APPROVAL", "via": "mcp", "reason": reason, "packages": batch.len() })).ip(&ip))?;
                        Ok(json!({ "task_id": id, "host": h.hostname, "packages": batch.len(), "status": "pending_approval", "note": "A person must approve this task in CroustyPatch before anything is installed." }))
                    })
                })
                .await?)
        }
        _ => Err(CallError::Unknown),
    }
}
