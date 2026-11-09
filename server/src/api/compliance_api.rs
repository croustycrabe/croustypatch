//! Compliance (OpenSCAP) endpoints.
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::schema::{compliance_reports, hosts};
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

/// Fleet score: mean of each host's latest score (None without any report).
pub fn fleet_score(c: &mut PgConnection) -> ApiResult<Option<f64>> {
    let l = latest(c)?;
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
    let rows: Vec<Row> = state.db(move |c| Ok(compliance_reports::table.filter(compliance_reports::host_id.eq(id)).order(compliance_reports::created_at.desc()).limit(20).select(COLS).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(json!({ "reports": rows.iter().enumerate().map(|(i, r)| row_json(r, i == 0)).collect::<Vec<_>>() })))
}

#[derive(Deserialize, Default)]
pub struct ScanBody {
    #[serde(default)]
    profile: String,
}

pub async fn scan(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: Option<web::Json<ScanBody>>) -> ApiResult<HttpResponse> {
    user.require("tasks:create")?;
    let mut profile = body.map(|b| b.into_inner().profile).unwrap_or_default();
    if profile.is_empty() {
        let st = state.clone();
        profile = state.db(move |c| super::admin::load_setting(c, &st, "compliance")).await?.get("profile").and_then(|p| p.as_str()).unwrap_or("").to_string();
    }
    if profile.len() > 200 || !profile.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
        return Err(ApiError::bad("profile must be an XCCDF profile id"));
    }
    let host = path.into_inner();
    if !state.db(move |c| crate::modules::for_host(c, host)).await?.openscap {
        return Err(ApiError::Conflict("the OpenSCAP module is switched off for this host (host page > Modules)".into()));
    }
    super::fleet::control_task_with(&state, user, host, "compliance", "task.compliance_requested", Some(json!({ "profile": profile }))).await
}
