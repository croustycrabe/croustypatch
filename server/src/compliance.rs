//! OpenSCAP compliance results (spec openscap): storage, threshold alert, module switches for agent data.
use crate::audit::{self, Event};
use crate::error::{ApiError, ApiResult};
use crate::schema::{compliance_reports, settings, tasks};
use croustypatch_common::protocol::ComplianceReport;
use diesel::prelude::*;
use serde_json::{json, Value};
use uuid::Uuid;

/// Default alert threshold, overridable with the `compliance` setting (`{"threshold": 85}`).
pub const DEFAULT_THRESHOLD: f64 = 80.0;

/// The `modules` setting (not encrypted), read without the app state so the gateway can filter agent data.
pub fn modules(c: &mut PgConnection) -> Value {
    settings::table.find("modules").select(settings::value).first::<Value>(c).optional().ok().flatten().unwrap_or_else(|| json!({}))
}

pub fn module_on(m: &Value, name: &str) -> bool {
    m.get(name).and_then(|v| v.as_bool()).unwrap_or(false)
}

pub fn threshold(c: &mut PgConnection) -> f64 {
    settings::table.find("compliance").select(settings::value).first::<Value>(c).optional().ok().flatten().and_then(|v| v.get("threshold").and_then(|t| t.as_f64())).unwrap_or(DEFAULT_THRESHOLD)
}

pub fn store(c: &mut PgConnection, host_id: Uuid, task_id: Uuid, r: &ComplianceReport) -> ApiResult<()> {
    let owned: i64 = tasks::table.filter(tasks::id.eq(task_id)).filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq("compliance")).count().get_result(c)?;
    if owned == 0 {
        return Err(ApiError::bad("compliance report for a task this host does not own"));
    }
    if !(0.0..=100.0).contains(&r.score) || r.profile.len() > 200 || r.failed_rules.len() > 500 {
        return Err(ApiError::bad("malformed compliance report"));
    }
    let limit = threshold(c);
    c.transaction(|c| {
        diesel::insert_into(compliance_reports::table)
            .values((
                compliance_reports::host_id.eq(host_id),
                compliance_reports::task_id.eq(Some(task_id)),
                compliance_reports::profile.eq(&r.profile),
                compliance_reports::datastream.eq(&r.datastream),
                compliance_reports::passed.eq(r.passed as i32),
                compliance_reports::failed.eq(r.failed as i32),
                compliance_reports::not_applicable.eq(r.not_applicable as i32),
                compliance_reports::other.eq(r.other as i32),
                compliance_reports::score.eq(r.score),
                compliance_reports::failed_rules.eq(serde_json::to_value(&r.failed_rules).unwrap_or_default()),
                compliance_reports::results_sha256.eq(&r.results_sha256),
                compliance_reports::collected_at.eq(r.collected_at),
            ))
            .execute(c)?;
        let details = json!({ "profile": r.profile, "score": r.score, "failed": r.failed, "threshold": limit, "results_sha256": r.results_sha256, "message": format!("score {:.1}% ({} failed rules), threshold {limit}%", r.score, r.failed) });
        let action = if r.score < limit { "compliance.score_low" } else { "compliance.reported" };
        audit::record(c, Event::new("agent", action).target("task", &task_id.to_string()).host(host_id).details(details))
    })
}
