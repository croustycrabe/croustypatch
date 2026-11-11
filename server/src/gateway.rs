//! Everything that talks to agents: enrolment, JWT authentication, the WebSocket tunnel (pull mode),
//! push-mode telemetry, task state tracking and signed command dispatch.
use crate::audit::{self, Event};
use crate::auth::client_ip;
use crate::error::{ApiError, ApiResult};
use crate::schema::{enrollment_tokens, host_group_members, host_packages, hosts, task_events, tasks, vulnerabilities};
use crate::state::AppState;
use actix_web::{dev::Payload, web, Error, FromRequest, HttpRequest, HttpResponse};
use chrono::{DateTime, Duration, Utc};
use croustypatch_common::crypto::{self, sha256_hex};
use croustypatch_common::jwt;
use croustypatch_common::model::{TaskKind, TaskStatus};
use croustypatch_common::protocol::*;
use diesel::prelude::*;
use futures_util::StreamExt;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc;
use uuid::Uuid;

pub const DEFAULT_HEARTBEAT_SECS: u64 = 60;
const COMMAND_TTL_MINUTES: i64 = 10;
const MAX_AGENT_JWT_LIFETIME: i64 = 600;

// ------------------------------------------------------------------------- hub

struct Session {
    id: u64,
    tx: mpsc::UnboundedSender<ServerMessage>,
}

/// Registry of live WebSocket tunnels.
#[derive(Default)]
pub struct Hub {
    sessions: Mutex<HashMap<Uuid, Session>>,
    next: Mutex<u64>,
}

impl Hub {
    pub fn register(&self, host: Uuid, tx: mpsc::UnboundedSender<ServerMessage>) -> u64 {
        let id = {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            *n
        };
        // A reconnecting agent replaces its stale session.
        self.sessions.lock().unwrap().insert(host, Session { id, tx });
        id
    }
    /// Remove only if the session is still the one that is closing; returns true if removed.
    pub fn unregister(&self, host: Uuid, id: u64) -> bool {
        let mut s = self.sessions.lock().unwrap();
        if s.get(&host).is_some_and(|x| x.id == id) {
            s.remove(&host);
            true
        } else {
            false
        }
    }
    pub fn send(&self, host: Uuid, msg: ServerMessage) -> bool {
        self.sessions.lock().unwrap().get(&host).is_some_and(|s| s.tx.send(msg).is_ok())
    }
    pub fn connected_hosts(&self) -> Vec<Uuid> {
        self.sessions.lock().unwrap().keys().copied().collect()
    }
}

// ------------------------------------------------------------------ enrolment

pub fn token_hash(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

pub async fn enroll(req: HttpRequest, state: web::Data<AppState>, body: web::Json<EnrollRequest>) -> ApiResult<HttpResponse> {
    let ip = client_ip(&req, &state.cfg);
    let b = body.into_inner();
    crypto::check_public_pem(&b.public_key_pem, 2048).map_err(|e| ApiError::bad(format!("invalid public key: {e}")))?;
    if b.facts.hostname.trim().is_empty() || b.facts.hostname.len() > 255 {
        return Err(ApiError::bad("hostname is required"));
    }
    let server_pub = state.server_public_pem.clone();
    let hash = token_hash(&b.token);
    let st = state.clone();
    let resp = state
        .db(move |c| {
            // Heartbeat interval comes from the "agent" setting (Settings page), clamped to a sane range.
            let heartbeat = crate::api::admin_setting_heartbeat(c, &st);
            c.transaction::<_, ApiError, _>(|c| {
                let now = Utc::now();
                // Atomic claim: only succeeds while the token is valid and has uses left.
                let claimed: Option<(Uuid, Option<Uuid>, String, String, Option<Value>)> = diesel::update(
                    enrollment_tokens::table
                        .filter(enrollment_tokens::token_hash.eq(&hash))
                        .filter(enrollment_tokens::revoked.eq(false))
                        .filter(enrollment_tokens::expires_at.gt(now))
                        .filter(enrollment_tokens::used_count.lt(enrollment_tokens::max_uses)),
                )
                .set(enrollment_tokens::used_count.eq(enrollment_tokens::used_count + 1))
                .returning((enrollment_tokens::id, enrollment_tokens::group_id, enrollment_tokens::environment, enrollment_tokens::comm_mode, enrollment_tokens::modules))
                .get_result(c)
                .optional()?;
                let Some((token_id, group_id, environment, comm_mode, token_modules)) = claimed else {
                    // Returned as Ok(None), not Err: an Err would roll the transaction back and erase this audit entry.
                    audit::record(c, Event::new("agent", "agent.enroll_rejected").details(json!({"hostname": b.facts.hostname})).ip(&ip))?;
                    return Ok(None);
                };
                let f = &b.facts;
                // Modules chosen when the token was made, else today's defaults.
                let modules = token_modules.map(|m| crate::modules::parse(&m)).unwrap_or_else(|| crate::modules::defaults(c));
                // Detected at enrolment only: administrators may change it afterwards.
                let asset_type = if f.is_laptop { "laptop" } else if f.is_desktop { "workstation" } else { "server" };
                let host_id: Uuid = diesel::insert_into(hosts::table)
                    .values((
                        hosts::hostname.eq(&f.hostname),
                        hosts::status.eq("online"),
                        hosts::public_key_pem.eq(&b.public_key_pem),
                        hosts::facts.eq(serde_json::to_value(f).unwrap_or_default()),
                        hosts::os_family.eq(&f.os_family),
                        hosts::distro.eq(&f.distro),
                        hosts::package_manager.eq(f.package_manager.map(|p| p.as_str()).unwrap_or("unknown")),
                        hosts::asset_type.eq(asset_type),
                        hosts::environment.eq(environment),
                        hosts::comm_mode.eq(comm_mode),
                        hosts::last_seen_at.eq(now),
                        hosts::heartbeat_secs.eq(heartbeat as i32),
                        hosts::modules.eq(serde_json::to_value(modules).unwrap_or_default()),
                    ))
                    .returning(hosts::id)
                    .get_result(c)?;
                if let Some(g) = group_id {
                    diesel::insert_into(host_group_members::table).values((host_group_members::host_id.eq(host_id), host_group_members::group_id.eq(g))).on_conflict_do_nothing().execute(c)?;
                }
                audit::record(c, Event::new("agent", "agent.enrolled").target("host", &host_id.to_string()).host(host_id).details(json!({"hostname": f.hostname, "token": token_id, "distro": f.distro})).ip(&ip))?;
                Ok(Some(EnrollResponse { agent_id: host_id, server_public_key_pem: server_pub, heartbeat_secs: heartbeat }))
            })
        })
        .await?
        .ok_or(ApiError::Unauthorized)?;
    Ok(HttpResponse::Created().json(resp))
}

// --------------------------------------------------------------------- agent auth

pub struct AgentAuth {
    pub host_id: Uuid,
    pub ip: String,
}

impl FromRequest for AgentAuth {
    type Error = ApiError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, ApiError>>>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            let state = req.app_data::<web::Data<AppState>>().ok_or_else(|| ApiError::internal("missing state"))?.clone();
            let token = req.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).ok_or(ApiError::Unauthorized)?.to_owned();
            let host_id: Uuid = jwt::peek_subject(&token).and_then(|s| s.parse().ok()).ok_or(ApiError::Unauthorized)?;
            let key: Option<String> = state.db(move |c| Ok(hosts::table.find(host_id).select(hosts::public_key_pem).first(c).optional()?)).await?;
            let key = key.ok_or(ApiError::Unauthorized)?;
            let now = Utc::now().timestamp();
            let claims = jwt::verify_claims_rs256(&key, &token, now, 30).map_err(|_| ApiError::Unauthorized)?;
            if claims.iss != "agent" || claims.sub != host_id.to_string() || claims.exp - claims.iat > MAX_AGENT_JWT_LIFETIME {
                return Err(ApiError::Unauthorized);
            }
            // Replay protection: a given jti is accepted once.
            {
                let mut seen = state.seen_jti.lock().unwrap();
                seen.retain(|_, exp| *exp > now);
                if seen.insert(claims.jti.clone(), claims.exp).is_some() {
                    return Err(ApiError::Unauthorized);
                }
            }
            Ok(AgentAuth { host_id, ip: client_ip(&req, &state.cfg) })
        })
    }
}

// ------------------------------------------------------------ message processing

fn cve_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^CVE-\d{4}-\d{4,}$").unwrap())
}

#[derive(Insertable)]
#[diesel(table_name = host_packages)]
struct NewPkg<'a> {
    host_id: Uuid,
    name: &'a str,
    source: &'a str,
    version: &'a str,
    candidate: Option<&'a str>,
    is_security: bool,
    cves: Value,
}

#[derive(Insertable)]
#[diesel(table_name = vulnerabilities)]
struct NewCve<'a> {
    cve_id: &'a str,
    source: &'a str,
}

fn host_status(current: &str, eco: bool, downtime: Option<DateTime<Utc>>, now: DateTime<Utc>) -> &'static str {
    if current == "quarantined" {
        return "quarantined";
    }
    if downtime.is_some_and(|d| d > now) {
        return "maintenance";
    }
    if eco {
        "eco"
    } else {
        "online"
    }
}

fn apply_facts(c: &mut PgConnection, host_id: Uuid, f: &HostFacts) -> ApiResult<()> {
    diesel::update(hosts::table.find(host_id))
        .set((
            hosts::facts.eq(serde_json::to_value(f).unwrap_or_default()),
            hosts::os_family.eq(&f.os_family),
            hosts::distro.eq(&f.distro),
            hosts::package_manager.eq(f.package_manager.map(|p| p.as_str()).unwrap_or("unknown")),
            hosts::hostname.eq(&f.hostname),
        ))
        .execute(c)?;
    Ok(())
}

fn touch(c: &mut PgConnection, host_id: Uuid, eco: Option<bool>, now: DateTime<Utc>) -> ApiResult<()> {
    let (status, current_eco, downtime): (String, bool, Option<DateTime<Utc>>) = hosts::table.find(host_id).select((hosts::status, hosts::eco, hosts::downtime_until)).first(c)?;
    let eco = eco.unwrap_or(current_eco);
    let new_status = host_status(&status, eco, downtime, now);
    diesel::update(hosts::table.find(host_id)).set((hosts::last_seen_at.eq(now), hosts::eco.eq(eco), hosts::status.eq(new_status))).execute(c)?;
    if status != new_status && status != "pending" && (new_status == "eco" || status == "eco") {
        audit::record(c, Event::new("agent", if new_status == "eco" { "host.eco_entered" } else { "host.eco_left" }).target("host", &host_id.to_string()).host(host_id))?;
    }
    Ok(())
}

fn store_inventory(c: &mut PgConnection, host_id: Uuid, inv: &Inventory, now: DateTime<Utc>) -> ApiResult<()> {
    c.transaction::<_, ApiError, _>(|c| {
        apply_facts(c, host_id, &inv.facts)?;
        // The inventory is complete each time: replace every source. Module data (containers, helper scripts) is
        // kept only while its module is switched on for this host.
        diesel::delete(host_packages::table.filter(host_packages::host_id.eq(host_id))).execute(c)?;
        let modules = crate::modules::for_host(c, host_id)?;
        let keep = |src: &str| match src {
            "docker" => modules.docker,
            "helper-script" => modules.helper_script,
            _ => true,
        };
        // (name, source) is the key: the last entry wins on duplicates (e.g. multi-arch).
        let mut dedup: HashMap<(&str, &str), &PackageEntry> = HashMap::new();
        for p in inv.packages.iter().filter(|p| keep(&p.source)) {
            dedup.insert((p.name.as_str(), p.source.as_str()), p);
        }
        let rows: Vec<NewPkg> = dedup
            .values()
            .filter(|p| !p.name.is_empty() && p.name.len() <= 255)
            .map(|p| NewPkg {
                host_id,
                name: &p.name,
                source: &p.source,
                version: &p.version,
                candidate: p.candidate.as_deref().filter(|cand| *cand != p.version),
                is_security: p.is_security && p.candidate.is_some(),
                cves: json!(p.cves.iter().filter(|x| cve_re().is_match(x)).collect::<Vec<_>>()),
            })
            .collect();
        for chunk in rows.chunks(2000) {
            diesel::insert_into(host_packages::table).values(chunk).execute(c)?;
        }
        let cves: HashSet<&str> = inv.packages.iter().flat_map(|p| p.cves.iter().map(|s| s.as_str())).filter(|x| cve_re().is_match(x)).collect();
        let new: Vec<NewCve> = cves.into_iter().map(|cve_id| NewCve { cve_id, source: "agent" }).collect();
        for chunk in new.chunks(2000) {
            diesel::insert_into(vulnerabilities::table).values(chunk).on_conflict_do_nothing().execute(c)?;
        }
        diesel::update(hosts::table.find(host_id)).set((hosts::reboot_required.eq(inv.reboot_required), hosts::last_inventory_at.eq(now))).execute(c)?;
        Ok(())
    })
}

fn can_agent_set(s: TaskStatus) -> bool {
    matches!(s, TaskStatus::Running | TaskStatus::Soaking | TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::RollingBack | TaskStatus::RolledBack | TaskStatus::Quarantined)
}

fn store_task_update(c: &mut PgConnection, host_id: Uuid, u: &TaskUpdate, now: DateTime<Utc>) -> ApiResult<()> {
    if !can_agent_set(u.status) {
        return Err(ApiError::bad("status not allowed from agent"));
    }
    c.transaction::<_, ApiError, _>(|c| {
        let found: Option<(String, Option<Value>, String)> = tasks::table.filter(tasks::id.eq(u.task_id)).filter(tasks::host_id.eq(host_id)).select((tasks::status, tasks::exec, tasks::kind)).for_update().first(c).optional()?;
        let Some((cur, exec, kind)) = found else {
            tracing::warn!(task = %u.task_id, %host_id, "update for unknown task ignored");
            return Ok(());
        };
        if TaskStatus::parse(&cur).is_some_and(|s| s.is_terminal()) {
            return Ok(()); // late or duplicate message
        }
        let soak_minutes = exec.as_ref().and_then(|e| e.get("soak_minutes")).and_then(|v| v.as_u64()).unwrap_or(0) as i64;
        let soak_until = (u.status == TaskStatus::Soaking).then(|| now + Duration::minutes(soak_minutes.max(1)));
        let result = json!({ "changes": u.changes, "reboot_required": u.reboot_required, "log_tail": u.log.iter().rev().take(200).rev().collect::<Vec<_>>() });
        diesel::update(tasks::table.find(u.task_id))
            .set((
                tasks::status.eq(u.status.as_str()),
                tasks::message.eq(&u.message),
                tasks::result.eq(Some(result)),
                tasks::updated_at.eq(now),
                tasks::soak_until.eq(soak_until),
            ))
            .execute(c)?;
        if u.status == TaskStatus::Running {
            diesel::update(tasks::table.find(u.task_id).filter(tasks::started_at.is_null())).set(tasks::started_at.eq(now)).execute(c)?;
        }
        if u.status.is_terminal() {
            diesel::update(tasks::table.find(u.task_id)).set(tasks::finished_at.eq(now)).execute(c)?;
        }
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(u.task_id), task_events::status.eq(u.status.as_str()), task_events::message.eq(&u.message))).execute(c)?;
        if u.reboot_required {
            diesel::update(hosts::table.find(host_id)).set(hosts::reboot_required.eq(true)).execute(c)?;
        }
        let tid = u.task_id.to_string();
        match u.status {
            TaskStatus::Quarantined => {
                diesel::update(hosts::table.find(host_id)).set(hosts::status.eq("quarantined")).execute(c)?;
                audit::record(c, Event::new("agent", "host.quarantined").target("task", &tid).host(host_id).details(json!({"message": u.message, "kind": kind})))?;
            }
            s if s.is_terminal() || s == TaskStatus::RollingBack => {
                audit::record(c, Event::new("agent", &format!("task.{}", s.as_str())).target("task", &tid).host(host_id).details(json!({"message": u.message, "kind": kind, "packages": u.changes.len()})))?;
            }
            _ => {}
        }
        if kind == TaskKind::Reboot.as_str() {
            crate::reboot::on_update(c, host_id, u.task_id, u.status, exec.as_ref(), &u.message, now)?;
        } else if kind == TaskKind::Patch.as_str() && u.status == TaskStatus::Succeeded && u.reboot_required {
            crate::reboot::after_patch(c, host_id, u.task_id, now)?;
        }
        if u.status == TaskStatus::Succeeded {
            if kind == TaskKind::Release.as_str() {
                diesel::update(hosts::table.find(host_id)).set(hosts::status.eq("online")).execute(c)?;
            } else if kind == TaskKind::Quarantine.as_str() {
                diesel::update(hosts::table.find(host_id)).set(hosts::status.eq("quarantined")).execute(c)?;
            }
        }
        Ok(())
    })
}

/// Process a batch of agent messages. Failures on one message don't drop the others.
pub async fn handle_messages(state: &web::Data<AppState>, host_id: Uuid, msgs: Vec<AgentMessage>) {
    for m in msgs {
        let now = Utc::now();
        let res = state
            .db(move |c| match m {
                AgentMessage::Hello { facts, .. } => {
                    apply_facts(c, host_id, &facts)?;
                    touch(c, host_id, Some(facts.on_battery && facts.is_laptop), now)
                }
                AgentMessage::Heartbeat(h) => touch(c, host_id, Some(h.eco), now),
                AgentMessage::Inventory(inv) => {
                    store_inventory(c, host_id, &inv, now)?;
                    touch(c, host_id, Some(inv.facts.on_battery && inv.facts.is_laptop), now)
                }
                AgentMessage::TaskUpdate(u) => {
                    store_task_update(c, host_id, &u, now)?;
                    touch(c, host_id, None, now)
                }
                AgentMessage::Compliance { task_id, report } => crate::compliance::store(c, host_id, task_id, &report),
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(%host_id, error = ?e, "agent message rejected");
        }
    }
}

// ----------------------------------------------------------------- dispatching

type DueTask = (Uuid, String, Value, Option<Value>, Option<Uuid>);

/// Atomically move due `queued` tasks to `sent` and return their signed commands.
pub fn claim_due_commands(c: &mut PgConnection, state: &AppState, host_id: Uuid) -> ApiResult<Vec<SignedCommand>> {
    let now = Utc::now();
    let due: Vec<DueTask> = tasks::table
        .filter(tasks::host_id.eq(host_id))
        .filter(tasks::status.eq("queued"))
        .filter(tasks::scheduled_for.le(now))
        .filter(tasks::deadline_at.is_null().or(tasks::deadline_at.gt(now)))
        .order(tasks::created_at.asc())
        .select((tasks::id, tasks::kind, tasks::packages, tasks::exec, tasks::parent_task_id))
        .limit(10)
        .load(c)?;
    let mut out = vec![];
    for (id, kind, packages, exec, parent) in due {
        let action = match TaskKind::parse(&kind) {
            Some(TaskKind::Scan) => CommandAction::Scan,
            Some(TaskKind::Patch) => CommandAction::ApplyUpdates {
                packages: packages.as_array().map(|a| a.iter().filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_owned)).collect()).unwrap_or_default(),
                exec: exec.and_then(|e| serde_json::from_value(e).ok()).unwrap_or_default(),
            },
            Some(TaskKind::Rollback) => match parent {
                Some(target_task) => CommandAction::Rollback { target_task },
                None => {
                    mark_failed(c, id, "rollback task has no target", now)?;
                    continue;
                }
            },
            Some(TaskKind::Quarantine) => CommandAction::Quarantine,
            Some(TaskKind::Release) => CommandAction::Release,
            Some(TaskKind::App) => {
                let e = exec.clone().unwrap_or_default();
                CommandAction::App {
                    install: e.get("install").and_then(|v| v.as_bool()).unwrap_or(true),
                    packages: e.get("packages").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
                    command: e.get("command").filter(|v| v.is_object()).and_then(|v| serde_json::from_value(v.clone()).ok()),
                    user: e.get("user").and_then(|v| v.as_str()).map(str::to_owned),
                }
            }
            Some(TaskKind::AgentUpdate) => {
                let e = exec.clone().unwrap_or_default();
                let field = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                CommandAction::UpdateAgent { platform: field("platform"), sha256: field("sha256"), version: field("version"), gui_sha256: e.get("gui_sha256").and_then(|v| v.as_str()).map(str::to_owned) }
            }
            Some(TaskKind::Compliance) => CommandAction::ComplianceScan { profile: exec.as_ref().and_then(|e| e.get("profile")).and_then(|v| v.as_str()).unwrap_or("").to_string() },
            Some(TaskKind::Reboot) => {
                let e = exec.unwrap_or_default();
                CommandAction::Reboot {
                    delay_minutes: e.get("delay_minutes").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    message: e.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                }
            }
            None => {
                mark_failed(c, id, "unknown task kind", now)?;
                continue;
            }
        };
        // Claim first; if another dispatcher won the race the update matches 0 rows.
        let claimed = diesel::update(tasks::table.find(id).filter(tasks::status.eq("queued"))).set((tasks::status.eq("sent"), tasks::sent_at.eq(now), tasks::updated_at.eq(now))).execute(c)?;
        if claimed == 0 {
            continue;
        }
        let payload = CommandPayload { task_id: id, agent_id: host_id, issued_at: now, expires_at: now + Duration::minutes(COMMAND_TTL_MINUTES), nonce: crypto::random_token(16), action };
        let cmd = sign_command(&state.server_private_pem, &payload).map_err(ApiError::internal)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("sent"), task_events::message.eq("command signed and sent to agent"))).execute(c)?;
        out.push(cmd);
    }
    Ok(out)
}

fn mark_failed(c: &mut PgConnection, id: Uuid, msg: &str, now: DateTime<Utc>) -> ApiResult<()> {
    diesel::update(tasks::table.find(id)).set((tasks::status.eq("failed"), tasks::message.eq(msg), tasks::finished_at.eq(now), tasks::updated_at.eq(now))).execute(c)?;
    Ok(())
}

/// Push due commands to a connected tunnel right away (used after creating/approving a task).
pub async fn push_pending(state: &web::Data<AppState>, host_id: Uuid) {
    if !state.hub.connected_hosts().contains(&host_id) {
        return;
    }
    let st = state.clone();
    match state.db(move |c| claim_due_commands(c, &st, host_id)).await {
        Ok(cmds) => {
            for cmd in cmds {
                state.hub.send(host_id, ServerMessage::Command(cmd));
            }
        }
        Err(e) => tracing::warn!(%host_id, error = ?e, "dispatch failed"),
    }
}

// ----------------------------------------------------------------- push telemetry

pub async fn telemetry(state: web::Data<AppState>, agent: AgentAuth, body: web::Json<TelemetryRequest>) -> ApiResult<HttpResponse> {
    let host_id = agent.host_id;
    let msgs = body.into_inner().messages;
    handle_messages(&state, host_id, msgs).await;
    let st = state.clone();
    let (commands, hb, settings) = state
        .db(move |c| {
            let cmds = claim_due_commands(c, &st, host_id)?;
            let hb: i32 = hosts::table.find(host_id).select(hosts::heartbeat_secs).first(c)?;
            Ok((cmds, hb, crate::modules::settings_for(c, host_id)?))
        })
        .await?;
    Ok(HttpResponse::Ok().json(TelemetryResponse { commands, heartbeat_secs: hb.max(10) as u64, settings }))
}

// ---------------------------------------------------------------- websocket tunnel

pub async fn ws_tunnel(req: HttpRequest, body: web::Payload, state: web::Data<AppState>, agent: AgentAuth) -> Result<HttpResponse, Error> {
    let host_id = agent.host_id;
    let (resp, mut session, stream) = actix_ws::handle(&req, body)?;
    let mut stream = stream.max_frame_size(16 * 1024 * 1024).aggregate_continuations().max_continuation_size(16 * 1024 * 1024);
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
    let sid = state.hub.register(host_id, tx);
    let ip = agent.ip.clone();

    let st = state.clone();
    let ip2 = ip.clone();
    let _ = st
        .db(move |c| {
            diesel::update(hosts::table.find(host_id)).set((hosts::connected.eq(true), hosts::comm_mode.eq("pull"), hosts::last_seen_at.eq(Utc::now()))).execute(c)?;
            audit::record(c, Event::new("agent", "agent.tunnel_connected").target("host", &host_id.to_string()).host(host_id).ip(&ip2))
        })
        .await;

    let st = state.clone();
    actix_web::rt::spawn(async move {
        let hb = st.db(move |c| Ok(hosts::table.find(host_id).select(hosts::heartbeat_secs).first::<i32>(c)?)).await.unwrap_or(60);
        let _ = session.text(serde_json::to_string(&ServerMessage::Welcome { heartbeat_secs: hb.max(10) as u64 }).unwrap()).await;
        push_pending(&st, host_id).await;
        let mut ping = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                incoming = stream.next() => match incoming {
                    Some(Ok(actix_ws::AggregatedMessage::Text(text))) => match serde_json::from_str::<AgentMessage>(&text) {
                        Ok(m) => {
                            let hello = matches!(m, AgentMessage::Hello { .. });
                            let had_update = hello || matches!(m, AgentMessage::TaskUpdate(_));
                            handle_messages(&st, host_id, vec![m]).await;
                            // The hello carries the agent's capabilities: only then do we know it reads its settings.
                            if hello { crate::modules::push_settings(&st, host_id).await; }
                            if had_update { push_pending(&st, host_id).await; }
                        }
                        Err(e) => tracing::warn!(%host_id, error = %e, "bad agent message"),
                    },
                    Some(Ok(actix_ws::AggregatedMessage::Ping(b))) => { let _ = session.pong(&b).await; }
                    Some(Ok(actix_ws::AggregatedMessage::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                },
                outgoing = rx.recv() => match outgoing {
                    Some(m) => { if session.text(serde_json::to_string(&m).unwrap()).await.is_err() { break; } }
                    None => break,
                },
                _ = ping.tick() => { if session.ping(b"").await.is_err() { break; } }
            }
        }
        if st.hub.unregister(host_id, sid) {
            let _ = st
                .db(move |c| {
                    diesel::update(hosts::table.find(host_id)).set(hosts::connected.eq(false)).execute(c)?;
                    audit::record(c, Event::new("agent", "agent.tunnel_closed").target("host", &host_id.to_string()).host(host_id))
                })
                .await;
        }
        let _ = session.close(None).await;
    });
    Ok(resp)
}
