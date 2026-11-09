//! The auto-patch pipeline: inventory + vulnerabilities + policies -> a decision -> a task.
use crate::audit::{self, Event};
use crate::engine::expr;
use crate::engine::policy::PolicyDef;
use crate::engine::risk::{self, AssessInput, Assessment};
use crate::error::{ApiError, ApiResult};
use crate::models::{Host, HostPackage};
use crate::schema::{host_group_members, host_packages, hosts, policies, policy_groups, task_events, tasks, vulnerabilities, waivers};
use chrono::{DateTime, Duration, Utc};
use croustypatch_common::crypto::sha256_hex;
use croustypatch_common::model::PolicyAction;
use croustypatch_common::protocol::{ExecPolicy, HostFacts};
use diesel::prelude::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Statuses in which a task still needs attention.
pub const OPEN: [&str; 6] = ["pending_approval", "queued", "sent", "running", "soaking", "rolling_back"];

pub fn batch_key(pkgs: &[(String, String)]) -> String {
    let mut v: Vec<String> = pkgs.iter().map(|(n, c)| format!("{n}={c}")).collect();
    v.sort();
    sha256_hex(v.join("\n").as_bytes())[..32].to_string()
}

/// When a task approved/created now should actually run.
pub fn compute_schedule(def: Option<&PolicyDef>, ring: i32, now: DateTime<Utc>) -> DateTime<Utc> {
    let Some(def) = def else { return now };
    let base = def.maintenance_window.as_ref().map(|w| w.next_start(now)).unwrap_or(now);
    base + Duration::minutes(ring.max(0) as i64 * def.rings.delay_minutes as i64)
}

pub fn exec_policy(def: &PolicyDef, has_staging: bool, criticality: &str) -> ExecPolicy {
    ExecPolicy {
        // Hardware without a staging twin must always have a restore point (spec §5).
        snapshot_required: def.rollback.snapshot_required || (!has_staging && criticality == "high"),
        soak_minutes: def.rollback.soak_minutes,
        probes: def.rollback.probes.clone(),
        auto_rollback: def.rollback.auto_rollback,
    }
}

pub struct Candidate {
    pub pkg: HostPackage,
    pub candidate: String,
}

/// Build the engine input for a host and an update batch.
pub fn assess_input(host: &Host, batch: &[Candidate], cve_info: &HashMap<String, (Option<f64>, bool)>) -> (AssessInput, Vec<String>) {
    let mut notes = vec![];
    let mut max_cvss = 0f64;
    let mut exploited = false;
    let mut unknown_security = false;
    for c in batch {
        let cves: Vec<&str> = c.pkg.cves.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
        let mut known = false;
        for cve in cves {
            if let Some((score, expl)) = cve_info.get(cve) {
                if let Some(s) = score {
                    max_cvss = max_cvss.max(*s);
                    known = true;
                }
                exploited |= *expl;
            }
        }
        if c.pkg.is_security && !known {
            unknown_security = true;
        }
    }
    if unknown_security && max_cvss < 5.0 {
        // A security update with no scored CVE: assume Medium rather than ignoring it.
        max_cvss = 5.0;
        notes.push("security update without a scored CVE: assumed CVSS 5.0".to_string());
    }
    let facts: HostFacts = serde_json::from_value(host.facts.clone()).unwrap_or_default();
    let critical: Vec<String> = batch.iter().filter(|c| risk::is_sensitive_package(&c.pkg.name)).map(|c| c.pkg.name.clone()).collect();
    let input = AssessInput {
        max_cvss,
        any_exploited: exploited,
        security_count: batch.iter().filter(|c| c.pkg.is_security).count(),
        package_count: batch.len(),
        includes_kernel: batch.iter().any(|c| risk::is_kernel_package(&c.pkg.name)),
        critical_packages: critical,
        asset_environment: host.environment.clone(),
        asset_type: host.asset_type.clone(),
        criticality: host.criticality.clone(),
        os_family: host.os_family.clone(),
        distro: host.distro.clone(),
        hostname: host.hostname.clone(),
        internet_exposed: host.internet_exposed,
        has_staging: host.has_staging,
        ring: host.ring,
        tags: host.tags.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
        modules: facts.modules,
    };
    (input, notes)
}

pub fn packages_json(batch: &[Candidate]) -> Value {
    json!(batch.iter().map(|c| json!({"name": c.pkg.name, "from": c.pkg.version, "to": c.candidate, "cves": c.pkg.cves, "is_security": c.pkg.is_security})).collect::<Vec<_>>())
}

pub fn load_cve_info(c: &mut PgConnection, batch: &[Candidate]) -> ApiResult<HashMap<String, (Option<f64>, bool)>> {
    let ids: HashSet<String> = batch.iter().flat_map(|c| c.pkg.cves.as_array().cloned().unwrap_or_default()).filter_map(|v| v.as_str().map(str::to_owned)).collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, Option<f64>, bool)> = vulnerabilities::table.filter(vulnerabilities::cve_id.eq_any(ids.into_iter().collect::<Vec<_>>())).select((vulnerabilities::cve_id, vulnerabilities::cvss, vulnerabilities::exploited)).load(c)?;
    Ok(rows.into_iter().map(|(id, s, e)| (id, (s, e))).collect())
}

/// Packages covered by an approved, unexpired waiver for this host.
fn waived(c: &mut PgConnection, host_id: Uuid, groups: &[Uuid], now: DateTime<Utc>) -> ApiResult<(HashSet<String>, HashSet<String>)> {
    let rows: Vec<(Option<String>, Option<String>)> = waivers::table
        .filter(waivers::status.eq("approved"))
        .filter(waivers::expires_at.gt(now))
        .filter(waivers::host_id.eq(host_id).or(waivers::group_id.eq_any(groups.to_vec())))
        .select((waivers::package, waivers::cve_id))
        .load(c)?;
    let pkgs = rows.iter().filter_map(|(p, _)| p.clone()).collect();
    let cves = rows.iter().filter_map(|(_, v)| v.clone()).collect();
    Ok((pkgs, cves))
}

/// Result of looking for an applicable policy.
pub struct Applicable {
    pub id: Uuid,
    pub name: String,
    pub def: PolicyDef,
}

pub fn applicable_policy(c: &mut PgConnection, host: &Host, groups: &[Uuid]) -> ApiResult<Option<Applicable>> {
    if groups.is_empty() {
        return Ok(None);
    }
    let rows: Vec<(Uuid, String, Value)> = policies::table
        .inner_join(policy_groups::table)
        .filter(policies::enabled.eq(true))
        .filter(policy_groups::group_id.eq_any(groups.to_vec()))
        .order((policies::priority.asc(), policies::name.asc()))
        .select((policies::id, policies::name, policies::definition))
        .load(c)?; // a policy linked to several of the host's groups appears twice; first match wins, so that is harmless
    let (scope_input, _) = assess_input(host, &[], &HashMap::new());
    let ctx = scope_input.context();
    for (id, name, def) in rows {
        let Ok(def) = serde_json::from_value::<PolicyDef>(def) else {
            tracing::warn!(policy = %name, "stored policy no longer parses; skipped");
            continue;
        };
        let in_scope = def.target_scope.trim().is_empty() || expr::eval_str(&def.target_scope, &ctx).unwrap_or(false);
        if in_scope {
            return Ok(Some(Applicable { id, name, def }));
        }
    }
    Ok(None)
}

pub fn host_groups(c: &mut PgConnection, host_id: Uuid) -> ApiResult<Vec<Uuid>> {
    Ok(host_group_members::table.filter(host_group_members::host_id.eq(host_id)).select(host_group_members::group_id).load(c)?)
}

/// Look at one host; create a task when its pending updates call for it.
pub fn plan_for_host(c: &mut PgConnection, host_id: Uuid, now: DateTime<Utc>) -> ApiResult<Option<Uuid>> {
    plan_for_host_with(c, host_id, now, None)
}

/// Same, with an AI advisor for policies in AI mode.
pub fn plan_for_host_with(c: &mut PgConnection, host_id: Uuid, now: DateTime<Utc>, advisor: Option<&dyn crate::ai::Advisor>) -> ApiResult<Option<Uuid>> {
    let host: Host = hosts::table.find(host_id).select(Host::as_select()).first(c)?;
    if matches!(host.status.as_str(), "pending" | "quarantined") {
        return Ok(None);
    }
    let open: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq_any(["patch", "rollback"])).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
    if open > 0 {
        return Ok(None);
    }
    let pending: Vec<HostPackage> = host_packages::table.filter(host_packages::host_id.eq(host_id)).filter(host_packages::candidate.is_not_null()).select(HostPackage::as_select()).load(c)?;
    if pending.is_empty() {
        return Ok(None);
    }
    let groups = host_groups(c, host_id)?;
    let Some(policy) = applicable_policy(c, &host, &groups)? else { return Ok(None) };
    let (waived_pkgs, waived_cves) = waived(c, host_id, &groups, now)?;

    let mut batch: Vec<Candidate> = vec![];
    for p in pending {
        if policy.def.security_only && !p.is_security {
            continue;
        }
        let cves: Vec<&str> = p.cves.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
        if waived_pkgs.contains(&p.name) || (!cves.is_empty() && cves.iter().all(|x| waived_cves.contains(*x))) {
            continue;
        }
        if let Some(candidate) = p.candidate.clone() {
            batch.push(Candidate { pkg: p, candidate });
        }
    }
    if batch.is_empty() {
        return Ok(None);
    }
    batch.sort_by(|a, b| a.pkg.name.cmp(&b.pkg.name));
    let key = batch_key(&batch.iter().map(|b| (b.pkg.name.clone(), b.candidate.clone())).collect::<Vec<_>>());
    let seen: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::batch_key.eq(&key)).filter(tasks::status.ne_all(["cancelled", "expired"])).count().get_result(c)?;
    if seen > 0 {
        return Ok(None); // already handled (or deliberately blocked/rejected/failed): never loop
    }

    let cve_info = load_cve_info(c, &batch)?;
    let (input, notes) = assess_input(&host, &batch, &cve_info);
    let mut a: Assessment = risk::assess_deterministic(&policy.def, &input);
    a.rationale.splice(0..0, notes);
    if policy.def.ai_mode_enabled {
        let pkgs: Vec<Value> = batch
            .iter()
            .map(|b| {
                let cves: Vec<Value> = b.pkg.cves.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|id| { let (cvss, kev) = cve_info.get(id).cloned().unwrap_or((None, false)); json!({ "id": id, "cvss": cvss, "exploited": kev }) }).collect()).unwrap_or_default();
                json!({ "name": b.pkg.name, "from": b.pkg.version, "to": b.candidate, "security": b.pkg.is_security, "cves": cves })
            })
            .collect();
        let ctx = crate::ai::context(&input, &pkgs, &a);
        a = crate::ai::apply(&policy.def, a, advisor, &ctx);
    }
    let (status, scheduled) = match a.action {
        PolicyAction::AutoDeploy => ("queued", compute_schedule(Some(&policy.def), host.ring, now)),
        PolicyAction::RequireApproval => ("pending_approval", now),
        PolicyAction::Block => ("blocked", now),
    };
    let exec = exec_policy(&policy.def, host.has_staging, &host.criticality);
    let decision = json!({ "policy_id": policy.id, "policy_name": policy.name, "assessment": a });
    let task_id: Uuid = diesel::insert_into(tasks::table)
        .values((
            tasks::host_id.eq(host_id),
            tasks::kind.eq("patch"),
            tasks::status.eq(status),
            tasks::policy_id.eq(Some(policy.id)),
            tasks::packages.eq(packages_json(&batch)),
            tasks::decision.eq(Some(decision)),
            tasks::exec.eq(Some(serde_json::to_value(&exec).unwrap_or_default())),
            tasks::message.eq(format!("{} ({}/{})", a.action, a.contextual_risk, a.stability_risk)),
            tasks::requested_by.eq(format!("policy:{}", policy.name)),
            tasks::scheduled_for.eq(scheduled),
            tasks::batch_key.eq(Some(&key)),
            tasks::finished_at.eq((status == "blocked").then_some(now)),
        ))
        .returning(tasks::id)
        .get_result(c)?;
    diesel::insert_into(task_events::table).values((task_events::task_id.eq(task_id), task_events::status.eq(status), task_events::message.eq(format!("planned by policy '{}': {}", policy.name, a.action)))).execute(c)?;
    audit::record(
        c,
        Event::new(&format!("policy:{}", policy.name), "task.planned").target("task", &task_id.to_string()).host(host_id).details(json!({
            "action": a.action, "contextual_risk": a.contextual_risk, "stability_risk": a.stability_risk, "packages": batch.len(), "engine": a.engine
        })),
    )?;
    Ok(Some(task_id))
}

/// A policy "what if" for the UI: evaluate without creating anything.
pub fn simulate(c: &mut PgConnection, host_id: Uuid, def: &PolicyDef) -> ApiResult<Value> {
    let host: Host = hosts::table.find(host_id).select(Host::as_select()).first(c).map_err(|_| ApiError::NotFound("host"))?;
    let pending: Vec<HostPackage> = host_packages::table.filter(host_packages::host_id.eq(host_id)).filter(host_packages::candidate.is_not_null()).select(HostPackage::as_select()).load(c)?;
    let batch: Vec<Candidate> = pending.into_iter().filter(|p| !def.security_only || p.is_security).filter_map(|p| p.candidate.clone().map(|candidate| Candidate { pkg: p, candidate })).collect();
    let info = load_cve_info(c, &batch)?;
    let (input, notes) = assess_input(&host, &batch, &info);
    let mut a = risk::assess_deterministic(def, &input);
    a.rationale.splice(0..0, notes);
    Ok(json!({ "packages": batch.len(), "assessment": a }))
}
