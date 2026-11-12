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

/// The profile a host is audited with when nobody names one: its own, else the fleet default when its content has it,
/// else '' (the agent then audits with the profile it recommends for that system and kind of machine).
pub fn effective_profile(c: &mut PgConnection, host_id: Uuid) -> String {
    let (own, facts): (String, Value) = hosts::table.find(host_id).select((hosts::compliance_profile, hosts::facts)).first(c).unwrap_or_default();
    if !own.is_empty() {
        return own;
    }
    let fleet = settings::table.find("compliance").select(settings::value).first::<Value>(c).optional().ok().flatten().and_then(|v| v.get("profile").and_then(|p| p.as_str().map(str::to_owned))).unwrap_or_default();
    fleet_default_for(&fleet, facts.get("openscap"))
}

/// One fleet default for Ubuntu, Debian and Red Hat machines cannot name a profile they all have (CIS for Ubuntu 24.04
/// does not exist in Debian's content): a host whose content is known and lacks it gets its recommended profile instead.
pub fn fleet_default_for(fleet: &str, openscap: Option<&Value>) -> String {
    if fleet.is_empty() || fleet.starts_with(CUSTOM_PREFIX) {
        return fleet.to_string();
    }
    let profiles: Vec<&str> = openscap.and_then(|o| o.get("profiles")).and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|p| p.get("id").and_then(|i| i.as_str())).collect()).unwrap_or_default();
    if profiles.is_empty() || profiles.contains(&fleet) { fleet.to_string() } else { String::new() }
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

// ------------------------------------------------------------------------------------------ planned audits

/// When the fleet is audited without anyone asking: the `schedule` of the `compliance` setting. `frequency` "off" (the
/// default), "daily", "weekly" (weekday 0 = Monday) or "monthly" (day 1 to 28), at a local time in a time zone.
pub fn schedule_of(setting: &Value) -> Option<(String, i32, i32, String, String)> {
    let s = setting.get("schedule")?;
    let f = s.get("frequency").and_then(|v| v.as_str()).unwrap_or("off");
    if !matches!(f, "daily" | "weekly" | "monthly") {
        return None;
    }
    let num = |k: &str, d: i64| s.get(k).and_then(|v| v.as_i64()).unwrap_or(d) as i32;
    let txt = |k: &str, d: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    Some((f.to_string(), num("weekday", 0), num("day_of_month", 1), txt("at", "03:00"), txt("timezone", "UTC")))
}

/// What a `schedule` may say (checked when the setting is saved).
pub fn validate_schedule(v: Option<&Value>) -> Result<(), String> {
    let Some(s) = v else { return Ok(()) };
    let f = s.get("frequency").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(f, "off" | "daily" | "weekly" | "monthly") {
        return Err("schedule.frequency must be off, daily, weekly or monthly".into());
    }
    let num = |k: &str| s.get(k).map(|v| v.as_i64());
    if matches!(num("weekday"), Some(None)) || num("weekday").flatten().is_some_and(|d| !(0..=6).contains(&d)) {
        return Err("schedule.weekday must be 0 (Monday) to 6".into());
    }
    if matches!(num("day_of_month"), Some(None)) || num("day_of_month").flatten().is_some_and(|d| !(1..=28).contains(&d)) {
        return Err("schedule.day_of_month must be 1 to 28".into());
    }
    let at = s.get("at").and_then(|v| v.as_str()).unwrap_or("03:00");
    if chrono::NaiveTime::parse_from_str(at, "%H:%M").is_err() || at.len() != 5 {
        return Err("schedule.at must be HH:MM".into());
    }
    let tz = s.get("timezone").and_then(|v| v.as_str()).unwrap_or("UTC");
    if tz.parse::<chrono_tz::Tz>().is_err() {
        return Err(format!("schedule.timezone: unknown time zone {tz}"));
    }
    Ok(())
}

/// A planned moment is only caught up within this delay: a server that was down for a week does not audit the fleet
/// for a moment long past, and switching the schedule on in the afternoon does not run this morning's audit.
const CATCH_UP_HOURS: i64 = 6;

/// The moment of the planned audit due now, if one is: the latest scheduled moment, not done yet, recent enough.
pub fn planned_due(setting: &Value, last_done: Option<chrono::DateTime<chrono::Utc>>, enabled_at: Option<chrono::DateTime<chrono::Utc>>, now: chrono::DateTime<chrono::Utc>) -> Option<chrono::DateTime<chrono::Utc>> {
    let (f, wd, dom, at, tz) = schedule_of(setting)?;
    let slot = crate::reports::Recurrence { frequency: &f, weekday: wd, day_of_month: dom, at: &at, timezone: &tz }.latest(now)?;
    let fresh = now - slot <= chrono::Duration::hours(CATCH_UP_HOURS);
    let new = last_done.is_none_or(|d| slot > d);
    let after_enabling = enabled_at.is_none_or(|e| slot >= e);
    (fresh && new && after_enabling).then_some(slot)
}

/// The next planned audit (for the page), None when the schedule is off.
pub fn planned_next(setting: &Value, now: chrono::DateTime<chrono::Utc>) -> Option<chrono::DateTime<chrono::Utc>> {
    let (f, wd, dom, at, tz) = schedule_of(setting)?;
    crate::reports::Recurrence { frequency: &f, weekday: wd, day_of_month: dom, at: &at, timezone: &tz }.next(now)
}

/// The audit-log action that marks a planned audit as started (its details hold the moment and the hosts).
pub const PLANNED_ACTION: &str = "compliance.planned_run";

/// Start the planned audit when one is due: a compliance task for every host whose OpenSCAP module is on (not the
/// quarantined or not yet approved ones, nor those with an audit already open). Hosts that are offline get theirs when
/// they come back. Returns how many audits were queued.
pub fn run_planned(c: &mut PgConnection, now: chrono::DateTime<chrono::Utc>) -> ApiResult<usize> {
    use crate::schema::{audit_log, task_events};
    let row: Option<(Value, chrono::DateTime<chrono::Utc>)> = settings::table.find("compliance").select((settings::value, settings::updated_at)).first(c).optional()?;
    let Some((setting, updated_at)) = row else { return Ok(0) };
    let last: Option<Value> = audit_log::table.filter(audit_log::action.eq(PLANNED_ACTION)).order(audit_log::seq.desc()).select(audit_log::details).first(c).optional()?;
    let last_done = last.as_ref().and_then(|d| d.get("slot")).and_then(|s| s.as_str()).and_then(|s| s.parse().ok());
    // the schedule as saved: a moment before the setting was last changed belongs to a schedule that may not exist
    let Some(slot) = planned_due(&setting, last_done, Some(updated_at), now) else { return Ok(0) };
    c.transaction::<_, ApiError, _>(|c| {
        let ids: Vec<(Uuid, String)> = hosts::table.filter(hosts::status.ne_all(["quarantined", "pending"])).select((hosts::id, hosts::hostname)).load(c)?;
        let mut queued = vec![];
        for (id, name) in ids {
            if !crate::modules::for_host(c, id)?.openscap {
                continue;
            }
            let open: i64 = tasks::table.filter(tasks::host_id.eq(id)).filter(tasks::kind.eq("compliance")).filter(tasks::status.eq_any(crate::planner::OPEN)).count().get_result(c)?;
            if open > 0 {
                continue;
            }
            let Ok(exec) = exec_for(c, id, "") else { continue };
            let msg = "planned compliance audit";
            let tid: Uuid = diesel::insert_into(tasks::table)
                .values((tasks::host_id.eq(id), tasks::kind.eq("compliance"), tasks::status.eq("queued"), tasks::requested_by.eq("schedule"), tasks::message.eq(msg), tasks::exec.eq(Some(exec))))
                .returning(tasks::id)
                .get_result(c)?;
            diesel::insert_into(task_events::table).values((task_events::task_id.eq(tid), task_events::status.eq("queued"), task_events::message.eq(msg))).execute(c)?;
            queued.push(name);
        }
        audit::record(c, Event::new("system", PLANNED_ACTION).target("setting", "compliance").details(json!({ "slot": slot.to_rfc3339(), "hosts": queued.len(), "hostnames": queued.iter().take(50).collect::<Vec<_>>() })))?;
        Ok(queued.len())
    })
}
