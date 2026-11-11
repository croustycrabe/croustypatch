//! OpenSCAP compliance results (spec openscap): storage, threshold alert, module switches for agent data.
use crate::audit::{self, Event};
use crate::error::{ApiError, ApiResult};
use crate::schema::{compliance_profiles, compliance_reports, hosts, settings, tasks};
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
        // Content written for another release marks every rule "not applicable": there is no score to compare with the
        // threshold, and a 0 % that is not a finding must not raise an alert.
        let applicable = r.passed + r.failed;
        let message = if applicable == 0 {
            format!("no rule applies to this system ({} not applicable): the SCAP content ({}) does not cover this release", r.not_applicable, r.datastream.rsplit('/').next().unwrap_or(""))
        } else {
            format!("score {:.1}% ({} failed rules), threshold {limit}%", r.score, r.failed)
        };
        let details = json!({ "profile": r.profile, "score": r.score, "failed": r.failed, "threshold": limit, "results_sha256": r.results_sha256, "message": message });
        let action = if applicable == 0 { "compliance.no_applicable_rules" } else if r.score < limit { "compliance.score_low" } else { "compliance.reported" };
        audit::record(c, Event::new("agent", action).target("task", &task_id.to_string()).host(host_id).details(details))
    })
}

/// Prefix of a profile reference that points at a custom profile (`custom:<uuid>`).
pub const CUSTOM_PREFIX: &str = "custom:";

/// Ids of profiles and rules: letters, digits, `_ . -` only (they end up in a command line and in an XML file on the host).
pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 300 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// What a host or the settings may name as audit profile: nothing (the default), an XCCDF profile id, or a custom profile.
pub fn valid_profile_ref(s: &str) -> bool {
    match s.strip_prefix(CUSTOM_PREFIX) {
        Some(id) => Uuid::parse_str(id).is_ok(),
        None => s.is_empty() || (s.len() <= 200 && valid_id(s)),
    }
}

/// Should the agent install OpenSCAP and its content when they are missing? On unless the `compliance` setting says no.
pub fn auto_install(c: &mut PgConnection) -> bool {
    settings::table.find("compliance").select(settings::value).first::<Value>(c).optional().ok().flatten().and_then(|v| v.get("auto_install").and_then(|t| t.as_bool())).unwrap_or(true)
}

/// The profile a host is audited with when nobody names one: its own, else the fleet default, else '' (SSG "standard").
pub fn effective_profile(c: &mut PgConnection, host_id: Uuid) -> String {
    let own: String = hosts::table.find(host_id).select(hosts::compliance_profile).first(c).unwrap_or_default();
    if !own.is_empty() {
        return own;
    }
    settings::table.find("compliance").select(settings::value).first::<Value>(c).optional().ok().flatten().and_then(|v| v.get("profile").and_then(|p| p.as_str().map(str::to_owned))).unwrap_or_default()
}

/// The `exec` of a compliance task: the profile to evaluate, the tailoring when it is a custom one, and whether the
/// agent may install what is missing. `requested` is the profile asked for in the request ('' = the host's own).
pub fn exec_for(c: &mut PgConnection, host_id: Uuid, requested: &str) -> ApiResult<Value> {
    let requested = requested.trim();
    if !valid_profile_ref(requested) {
        return Err(ApiError::bad("profile must be an XCCDF profile id or a custom profile"));
    }
    let wanted = if requested.is_empty() { effective_profile(c, host_id) } else { requested.to_string() };
    let auto = auto_install(c);
    let Some(id) = wanted.strip_prefix(CUSTOM_PREFIX) else {
        return Ok(json!({ "profile": wanted, "auto_install": auto }));
    };
    let id = Uuid::parse_str(id).map_err(|_| ApiError::bad("invalid custom profile"))?;
    let (name, base, disabled): (String, String, Value) = compliance_profiles::table
        .find(id)
        .select((compliance_profiles::name, compliance_profiles::base_profile, compliance_profiles::disabled_rules))
        .first(c)
        .map_err(|_| ApiError::Conflict("the custom audit profile chosen for this host no longer exists (Compliance > Profiles)".into()))?;
    Ok(json!({ "profile": base, "auto_install": auto, "tailoring": { "id": id, "name": name, "base": base, "disabled_rules": disabled } }))
}

/// Is the OpenSCAP module on for this host?
pub fn for_host_openscap(c: &mut PgConnection, host_id: Uuid) -> ApiResult<bool> {
    Ok(crate::modules::for_host(c, host_id)?.openscap)
}
