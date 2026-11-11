//! Compliance (OpenSCAP) endpoints.
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::audit::{self, Event};
use crate::schema::{compliance_profiles, compliance_reports, hosts};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

type Row = (Uuid, Uuid, String, i32, i32, i32, f64, Value, String, DateTime<Utc>);
const COLS: (compliance_reports::id, compliance_reports::host_id, compliance_reports::profile, compliance_reports::passed, compliance_reports::failed, compliance_reports::not_applicable, compliance_reports::score, compliance_reports::failed_rules, compliance_reports::results_sha256, compliance_reports::collected_at) =
    (compliance_reports::id, compliance_reports::host_id, compliance_reports::profile, compliance_reports::passed, compliance_reports::failed, compliance_reports::not_applicable, compliance_reports::score, compliance_reports::failed_rules, compliance_reports::results_sha256, compliance_reports::collected_at);

fn row_json(r: &Row, with_rules: bool) -> Value {
    let mut v = json!({ "id": r.0, "host_id": r.1, "profile": r.2, "passed": r.3, "failed": r.4, "not_applicable": r.5, "score": r.6, "results_sha256": r.8, "collected_at": r.9 });
    if with_rules {
        v["failed_rules"] = r.7.clone();
    }
    v
}

/// Latest report per host.
fn latest(c: &mut PgConnection) -> QueryResult<Vec<Row>> {
    let all: Vec<Row> = compliance_reports::table.order((compliance_reports::host_id, compliance_reports::created_at.desc())).select(COLS).load(c)?;
    let mut seen = std::collections::HashSet::new();
    Ok(all.into_iter().filter(|r| seen.insert(r.1)).collect())
}

/// Fleet score: mean of each host's latest score (None without any report). An audit in which no rule applied
/// (content for another release) has no score and is left out, rather than counting as 0 %.
pub fn fleet_score(c: &mut PgConnection) -> ApiResult<Option<f64>> {
    let l: Vec<Row> = latest(c)?.into_iter().filter(|r| r.3 + r.4 > 0).collect();
    Ok((!l.is_empty()).then(|| (l.iter().map(|r| r.6).sum::<f64>() / l.len() as f64 * 10.0).round() / 10.0))
}

pub async fn fleet(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let out = state
        .db(|c| {
            let l = latest(c)?;
            let names: Vec<(Uuid, String)> = hosts::table.select((hosts::id, hosts::hostname)).load(c)?;
            Ok(json!({
                "score": fleet_score(c)?,
                "threshold": crate::compliance::threshold(c),
                "hosts": l.iter().map(|r| { let mut v = row_json(r, false); v["hostname"] = json!(names.iter().find(|n| n.0 == r.1).map(|n| n.1.clone())); v }).collect::<Vec<_>>(),
            }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

pub async fn host(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let id = path.into_inner();
    let out = state
        .db(move |c| {
            let rows: Vec<Row> = compliance_reports::table.filter(compliance_reports::host_id.eq(id)).order(compliance_reports::created_at.desc()).limit(20).select(COLS).load(c)?;
            let (facts, profile, module): (Value, String, Value) = hosts::table.find(id).select((hosts::facts, hosts::compliance_profile, hosts::modules)).first(c).map_err(|_| ApiError::NotFound("host"))?;
            let effective = crate::compliance::effective_profile(c, id);
            Ok(json!({
                "reports": rows.iter().enumerate().map(|(i, r)| row_json(r, i == 0)).collect::<Vec<_>>(),
                // What the agent found: oscap version, the SCAP content it will use and that content's profiles.
                "openscap": facts.get("openscap").cloned().unwrap_or(Value::Null),
                "module": crate::compliance::module_on(&module, "openscap"),
                "profile": profile,
                "effective_profile": effective,
                "auto_install": crate::compliance::auto_install(c),
            }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize, Default)]
pub struct ScanBody {
    #[serde(default)]
    profile: String,
}

pub async fn scan(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: Option<web::Json<ScanBody>>) -> ApiResult<HttpResponse> {
    user.require("tasks:create")?;
    let requested = body.map(|b| b.into_inner().profile).unwrap_or_default();
    if !crate::compliance::valid_profile_ref(requested.trim()) {
        return Err(ApiError::bad("profile must be an XCCDF profile id or a custom profile"));
    }
    let host = path.into_inner();
    // The profile asked for in the request, else the host's own, else the fleet default; a custom one becomes a tailoring.
    let exec = state
        .db(move |c| {
            if !crate::compliance::for_host_openscap(c, host)? {
                return Err(ApiError::Conflict("the OpenSCAP module is switched off for this host (host page > Modules)".into()));
            }
            crate::compliance::exec_for(c, host, &requested)
        })
        .await?;
    super::fleet::control_task_with(&state, user, host, "compliance", "task.compliance_requested", Some(exec)).await
}

// ---- profiles --------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ProfileBody {
    name: String,
    #[serde(default)]
    description: String,
    base_profile: String,
    #[serde(default)]
    disabled_rules: Vec<String>,
}

type ProfileRow = (Uuid, String, String, String, Value, DateTime<Utc>, DateTime<Utc>);

fn profile_json(r: &ProfileRow, used_by: i64) -> Value {
    json!({ "id": r.0, "name": r.1, "description": r.2, "base_profile": r.3, "disabled_rules": r.4, "created_at": r.5, "updated_at": r.6, "used_by": used_by })
}

fn clean_profile(b: &ProfileBody) -> ApiResult<(String, String, Vec<String>)> {
    super::name_ok("name", &b.name)?;
    if b.name.trim().len() > 80 {
        return Err(ApiError::bad("name must be at most 80 characters"));
    }
    if b.description.len() > 500 || b.description.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ApiError::bad("description must be at most 500 printable characters"));
    }
    if !b.base_profile.starts_with("xccdf_") || !crate::compliance::valid_id(&b.base_profile) {
        return Err(ApiError::bad("base_profile must be a profile of the SCAP content (xccdf_…)"));
    }
    let mut rules: Vec<String> = b.disabled_rules.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
    rules.sort();
    rules.dedup();
    if rules.len() > 2000 || rules.iter().any(|r| !r.starts_with("xccdf_") || !crate::compliance::valid_id(r)) {
        return Err(ApiError::bad("disabled_rules: at most 2000 rule ids (xccdf_…)"));
    }
    Ok((b.name.trim().to_string(), b.description.trim().to_string(), rules))
}

/// How many hosts audit with this custom profile (their own choice, plus the fleet default counts for hosts without one).
fn users_of(c: &mut PgConnection, id: Uuid) -> QueryResult<i64> {
    let me = format!("{}{id}", crate::compliance::CUSTOM_PREFIX);
    let own: i64 = hosts::table.filter(hosts::compliance_profile.eq(&me)).count().get_result(c)?;
    let default_is_me = crate::schema::settings::table.find("compliance").select(crate::schema::settings::value).first::<Value>(c).optional()?.and_then(|v| v.get("profile").and_then(|p| p.as_str().map(|p| p == me))).unwrap_or(false);
    if default_is_me {
        let without: i64 = hosts::table.filter(hosts::compliance_profile.eq("")).count().get_result(c)?;
        return Ok(own + without);
    }
    Ok(own)
}

/// Custom profiles, and the profiles of the SCAP content found on the hosts (with how many hosts offer each).
pub async fn profiles(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let out = state
        .db(|c| {
            let rows: Vec<ProfileRow> = compliance_profiles::table
                .order(compliance_profiles::name.asc())
                .select((compliance_profiles::id, compliance_profiles::name, compliance_profiles::description, compliance_profiles::base_profile, compliance_profiles::disabled_rules, compliance_profiles::created_at, compliance_profiles::updated_at))
                .load(c)?;
            let mut custom = vec![];
            for r in &rows {
                custom.push(profile_json(r, users_of(c, r.0)?));
            }
            let facts: Vec<Value> = hosts::table.select(hosts::facts).load(c)?;
            let mut available: std::collections::BTreeMap<String, (String, i64)> = Default::default();
            let mut with_oscap = 0;
            for f in &facts {
                let Some(o) = f.get("openscap").filter(|o| !o.is_null()) else { continue };
                with_oscap += 1;
                for p in o.get("profiles").and_then(|p| p.as_array()).into_iter().flatten() {
                    let (id, title) = (p.get("id").and_then(|v| v.as_str()).unwrap_or(""), p.get("title").and_then(|v| v.as_str()).unwrap_or(""));
                    if !id.is_empty() {
                        available.entry(id.to_string()).or_insert_with(|| (title.to_string(), 0)).1 += 1;
                    }
                }
            }
            let default = crate::schema::settings::table.find("compliance").select(crate::schema::settings::value).first::<Value>(c).optional()?.and_then(|v| v.get("profile").and_then(|p| p.as_str().map(str::to_owned))).unwrap_or_default();
            Ok(json!({
                "custom": custom,
                "available": available.iter().map(|(id, (title, hosts))| json!({ "id": id, "title": title, "hosts": hosts })).collect::<Vec<_>>(),
                "hosts_with_openscap": with_oscap,
                "default": default,
            }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

fn unique_name(c: &mut PgConnection, name: &str, except: Option<Uuid>) -> ApiResult<()> {
    let names: Vec<(Uuid, String)> = compliance_profiles::table.select((compliance_profiles::id, compliance_profiles::name)).load(c)?;
    if names.iter().any(|(i, n)| Some(*i) != except && n.to_lowercase() == name.to_lowercase()) {
        return Err(ApiError::Conflict(format!("a custom profile named '{name}' already exists")));
    }
    Ok(())
}

pub async fn profile_create(state: web::Data<AppState>, user: AuthUser, body: web::Json<ProfileBody>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let (name, description, rules) = clean_profile(&body)?;
    let base = body.base_profile.clone();
    let out = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                unique_name(c, &name, None)?;
                let row: ProfileRow = diesel::insert_into(compliance_profiles::table)
                    .values((compliance_profiles::name.eq(&name), compliance_profiles::description.eq(&description), compliance_profiles::base_profile.eq(&base), compliance_profiles::disabled_rules.eq(json!(rules))))
                    .returning((compliance_profiles::id, compliance_profiles::name, compliance_profiles::description, compliance_profiles::base_profile, compliance_profiles::disabled_rules, compliance_profiles::created_at, compliance_profiles::updated_at))
                    .get_result(c)?;
                audit::record(c, Event::new(&user.username, "compliance.profile_created").target("compliance_profile", &row.0.to_string()).details(json!({ "name": name, "base": base, "disabled_rules": rules.len() })).ip(&user.ip))?;
                Ok(profile_json(&row, 0))
            })
        })
        .await?;
    Ok(HttpResponse::Created().json(out))
}

pub async fn profile_update(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<ProfileBody>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    let (name, description, rules) = clean_profile(&body)?;
    let base = body.base_profile.clone();
    let out = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                unique_name(c, &name, Some(id))?;
                let row: ProfileRow = diesel::update(compliance_profiles::table.find(id))
                    .set((compliance_profiles::name.eq(&name), compliance_profiles::description.eq(&description), compliance_profiles::base_profile.eq(&base), compliance_profiles::disabled_rules.eq(json!(rules)), compliance_profiles::updated_at.eq(Utc::now())))
                    .returning((compliance_profiles::id, compliance_profiles::name, compliance_profiles::description, compliance_profiles::base_profile, compliance_profiles::disabled_rules, compliance_profiles::created_at, compliance_profiles::updated_at))
                    .get_result(c)
                    .map_err(|_| ApiError::NotFound("custom profile"))?;
                audit::record(c, Event::new(&user.username, "compliance.profile_updated").target("compliance_profile", &id.to_string()).details(json!({ "name": name, "base": base, "disabled_rules": rules.len() })).ip(&user.ip))?;
                let used = users_of(c, id)?;
                Ok(profile_json(&row, used))
            })
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

pub async fn profile_delete(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                let name: String = compliance_profiles::table.find(id).select(compliance_profiles::name).first(c).map_err(|_| ApiError::NotFound("custom profile"))?;
                let used = users_of(c, id)?;
                if used > 0 {
                    return Err(ApiError::Conflict(format!("{used} host(s) are audited with '{name}': give them another profile first")));
                }
                diesel::delete(compliance_profiles::table.find(id)).execute(c)?;
                audit::record(c, Event::new(&user.username, "compliance.profile_deleted").target("compliance_profile", &id.to_string()).details(json!({ "name": name })).ip(&user.ip))?;
                Ok(())
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

/// Rules that fail on the fleet right now (latest audit of each host): the candidates to switch off in a custom profile.
pub async fn failing_rules(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let out = state
        .db(|c| {
            let mut by_rule: std::collections::BTreeMap<String, (String, i64)> = Default::default();
            for r in latest(c)? {
                for f in r.7.as_array().into_iter().flatten() {
                    let id = f.get("rule_id").and_then(|v| v.as_str()).unwrap_or("");
                    if !id.is_empty() {
                        let e = by_rule.entry(id.to_string()).or_insert_with(|| (f.get("severity").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(), 0));
                        e.1 += 1;
                    }
                }
            }
            let rank = |s: &str| match s { "high" => 0, "medium" => 1, "low" => 2, _ => 3 };
            let mut v: Vec<_> = by_rule.into_iter().collect();
            v.sort_by(|a, b| rank(&a.1 .0).cmp(&rank(&b.1 .0)).then(b.1 .1.cmp(&a.1 .1)).then(a.0.cmp(&b.0)));
            v.truncate(500);
            Ok(json!({ "rules": v.into_iter().map(|(id, (sev, hosts))| json!({ "rule_id": id, "severity": sev, "hosts": hosts })).collect::<Vec<_>>() }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}
