use crate::audit;
use crate::auth::{perms_allow, AuthUser};
use crate::error::ApiResult;
use crate::models::AuditEntry;
use crate::schema::{audit_log, host_packages, hosts, tasks, vulnerabilities, waivers};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::Utc;
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub async fn health(state: web::Data<AppState>) -> HttpResponse {
    match state.db(|c| Ok(diesel::sql_query("SELECT 1").execute(c)?)).await {
        Ok(_) => HttpResponse::Ok().json(json!({"status": "ok"})),
        Err(_) => HttpResponse::ServiceUnavailable().json(json!({"status": "database_unavailable"})),
    }
}

pub async fn server_info(state: web::Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "name": "CroustyPatch", "version": env!("CARGO_PKG_VERSION"),
        "tls": state.tls_direct, "protocol": croustypatch_common::PROTOCOL_VERSION,
    }))
}

// ------------------------------------------------------------------------ audit

#[derive(Deserialize)]
pub struct AuditQuery {
    host_id: Option<Uuid>,
    actor: Option<String>,
    action: Option<String>,
    before: Option<i64>,
    limit: Option<i64>,
}

pub async fn list_audit(state: web::Data<AppState>, user: AuthUser, q: web::Query<AuditQuery>) -> ApiResult<HttpResponse> {
    user.require("audit:read")?;
    let q = q.into_inner();
    let rows: Vec<AuditEntry> = state
        .db(move |c| {
            let mut query = audit_log::table.into_boxed();
            if let Some(h) = q.host_id {
                query = query.filter(audit_log::host_id.eq(h));
            }
            if let Some(a) = q.actor.filter(|a| !a.is_empty()) {
                query = query.filter(audit_log::actor.eq(a));
            }
            if let Some(a) = q.action.filter(|a| !a.is_empty()) {
                query = query.filter(audit_log::action.like(format!("{}%", a.replace('%', "").replace('_', "\\_"))));
            }
            if let Some(b) = q.before {
                query = query.filter(audit_log::seq.lt(b));
            }
            Ok(query.order(audit_log::seq.desc()).limit(q.limit.unwrap_or(100).clamp(1, 500)).select(AuditEntry::as_select()).load(c)?)
        })
        .await?;
    Ok(HttpResponse::Ok().json(rows))
}

pub async fn verify_audit(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("audit:read")?;
    let v = state.db(audit::verify).await?;
    Ok(HttpResponse::Ok().json(v))
}

// -------------------------------------------------------------------- dashboard

pub async fn dashboard(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("dashboard:read")?;
    let perms = user.permissions.clone();
    let st = state.clone();
    let out: Value = state
        .db(move |c| {
            let mut fleet: HashMap<String, i64> = HashMap::new();
            let counts: Vec<(String, i64)> = hosts::table.group_by(hosts::status).select((hosts::status, diesel::dsl::count_star())).load(c)?;
            let total: i64 = counts.iter().map(|(_, n)| n).sum();
            for (s, n) in counts {
                fleet.insert(s, n);
            }
            let pending: Vec<(Uuid, bool, Value, String)> = host_packages::table.filter(host_packages::candidate.is_not_null()).select((host_packages::host_id, host_packages::is_security, host_packages::cves, host_packages::name)).load(c)?;
            let mut hosts_sec: HashSet<Uuid> = HashSet::new();
            let mut hosts_upd: HashSet<Uuid> = HashSet::new();
            let mut cves: HashSet<String> = HashSet::new();
            let mut sec_updates = 0i64;
            let mut outdated: HashMap<String, (HashSet<Uuid>, bool)> = HashMap::new();
            for (h, sec, cv, name) in &pending {
                hosts_upd.insert(*h);
                let e = outdated.entry(name.clone()).or_default();
                e.0.insert(*h);
                if *sec {
                    hosts_sec.insert(*h);
                    sec_updates += 1;
                    e.1 = true;
                }
                cves.extend(cv.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)));
            }
            let mut top: Vec<(String, usize, bool)> = outdated.iter().map(|(n, (hs, sec))| (n.clone(), hs.len(), *sec)).collect();
            top.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
            let os_rows: Vec<(String, i64)> = hosts::table.group_by(hosts::distro).select((hosts::distro, diesel::dsl::count_star())).order(diesel::dsl::count_star().desc()).load(c)?;
            let critical: i64 = if cves.is_empty() { 0 } else { vulnerabilities::table.filter(vulnerabilities::cve_id.eq_any(cves.into_iter().collect::<Vec<_>>())).filter(vulnerabilities::cvss.ge(9.0)).count().get_result(c)? };
            let awaiting: i64 = tasks::table.filter(tasks::status.eq("pending_approval")).count().get_result(c)?;
            let running: i64 = tasks::table.filter(tasks::status.eq_any(["sent", "running", "soaking", "rolling_back"])).count().get_result(c)?;
            let waivers_open: i64 = waivers::table.filter(waivers::status.eq("pending")).count().get_result(c)?;
            let reboot: i64 = hosts::table.filter(hosts::reboot_required.eq(true)).count().get_result(c)?;
            let recent: Vec<(Uuid, Uuid, String, String, String, String, chrono::DateTime<Utc>)> = tasks::table
                .inner_join(hosts::table)
                .order(tasks::updated_at.desc())
                .limit(12)
                .select((tasks::id, tasks::host_id, hosts::hostname, tasks::kind, tasks::status, tasks::message, tasks::updated_at))
                .load(c)?;
            let modules = crate::api::admin::load_setting(c, &st, "modules")?;
            Ok(json!({
                "fleet": { "total": total, "by_status": fleet, "reboot_required": reboot, "hosts_needing_updates": hosts_upd.len(), "os": os_rows.into_iter().map(|(name, count)| json!({"name": if name.is_empty() { "?" } else { name.as_str() }, "count": count})).collect::<Vec<_>>() },
                "vulnerabilities": { "hosts_with_security_updates": hosts_sec.len(), "security_updates": sec_updates, "critical_cves_unpatched": critical, "outdated_packages": outdated.len() },
                "top_outdated": top.into_iter().take(6).map(|(name, hosts, security)| json!({"name": name, "hosts": hosts, "security": security})).collect::<Vec<_>>(),
                "tasks": { "awaiting_approval": awaiting, "in_progress": running, "waivers_pending": waivers_open },
                "activity": recent.into_iter().map(|(id, host_id, hostname, kind, status, message, ts)| json!({"id": id, "host_id": host_id, "hostname": hostname, "kind": kind, "status": status, "message": message, "ts": ts})).collect::<Vec<_>>(),
                // Optional modules are reported as inactive until they exist and are switched on.
                "compliance": { "enabled": modules.get("openscap").and_then(|v| v.as_bool()).unwrap_or(false), "score": crate::api::compliance_api::fleet_score(c)? },
                "portal": { "enabled": modules.get("portal").and_then(|v| v.as_bool()).unwrap_or(false), "installs": null },
                "can": { "approve": perms_allow(&perms, "tasks:write") },
            }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}
