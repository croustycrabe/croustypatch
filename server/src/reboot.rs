//! Reboot module (server side): downtime window while a host restarts, completion when it comes back, an alert when
//! it does not, and automatic reboots after patches when the policy asks for them.
use crate::audit::{self, Event};
use crate::engine::policy::PolicyDef;
use crate::error::ApiResult;
use crate::planner::OPEN;
use crate::schema::{hosts, policies, task_events, tasks};
use chrono::{DateTime, Duration, Utc};
use croustypatch_common::model::TaskStatus;
use diesel::prelude::*;
use serde_json::{json, Value};
use uuid::Uuid;

/// How long a restarting host may stay silent after its warning delay before it counts as down.
pub const TIMEOUT_MINUTES: i64 = 15;

/// Warning given to logged-in users before the restart: people work on workstations and laptops, not on servers.
pub fn default_delay(asset_type: &str) -> u32 {
    if asset_type == "server" { 0 } else { 10 }
}

fn delay_of(exec: Option<&Value>) -> i64 {
    exec.and_then(|e| e.get("delay_minutes")).and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 24 * 60)
}

/// End of the downtime window opened when the agent starts a reboot.
pub fn downtime_end(now: DateTime<Utc>, delay_minutes: i64) -> DateTime<Utc> {
    now + Duration::minutes(delay_minutes + TIMEOUT_MINUTES)
}

/// Progress reported by the agent for a reboot task.
pub fn on_update(c: &mut PgConnection, host_id: Uuid, task_id: Uuid, status: TaskStatus, exec: Option<&Value>, message: &str, now: DateTime<Utc>) -> ApiResult<()> {
    let tid = task_id.to_string();
    match status {
        TaskStatus::Running => {
            let until = downtime_end(now, delay_of(exec));
            diesel::update(hosts::table.find(host_id)).set(hosts::downtime_until.eq(Some(until))).execute(c)?;
            audit::record(c, Event::new("agent", "reboot.started").target("task", &tid).host(host_id).details(json!({ "downtime_until": until, "message": message })))?;
        }
        TaskStatus::Succeeded => {
            diesel::update(hosts::table.find(host_id)).set((hosts::downtime_until.eq(None::<DateTime<Utc>>), hosts::reboot_required.eq(false))).execute(c)?;
            diesel::update(hosts::table.find(host_id).filter(hosts::status.eq_any(["maintenance", "alert"]))).set(hosts::status.eq("online")).execute(c)?;
            audit::record(c, Event::new("agent", "reboot.completed").target("task", &tid).host(host_id).details(json!({ "message": message })))?;
        }
        TaskStatus::Failed => {
            diesel::update(hosts::table.find(host_id)).set(hosts::downtime_until.eq(None::<DateTime<Utc>>)).execute(c)?;
        }
        _ => {}
    }
    Ok(())
}

/// The host stayed silent past its downtime window: fail the reboot it was doing and raise the alarm.
pub fn overdue(c: &mut PgConnection, host_id: Uuid, now: DateTime<Utc>) -> ApiResult<()> {
    let open: Vec<Uuid> = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq("reboot")).filter(tasks::status.eq_any(["sent", "running"])).select(tasks::id).load(c)?;
    for id in open {
        let msg = format!("the host did not come back within {TIMEOUT_MINUTES} minutes of its reboot");
        diesel::update(tasks::table.find(id)).set((tasks::status.eq("failed"), tasks::message.eq(&msg), tasks::finished_at.eq(now), tasks::updated_at.eq(now))).execute(c)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("failed"), task_events::message.eq(&msg))).execute(c)?;
        audit::record(c, Event::new("system", "reboot.overdue").target("task", &id.to_string()).host(host_id).details(json!({ "message": msg })))?;
    }
    Ok(())
}

/// First moment at or after `now` inside the policy's maintenance window (now when there is no window).
pub fn next_slot(def: &PolicyDef, now: DateTime<Utc>) -> DateTime<Utc> {
    match &def.maintenance_window {
        Some(w) => w.next_start(now),
        None => now,
    }
}

/// A patch finished and the host needs a restart: schedule it if the policy says so ("auto", or "user_prompt",
/// which gives users a longer warning).
pub fn after_patch(c: &mut PgConnection, host_id: Uuid, patch_task: Uuid, now: DateTime<Utc>) -> ApiResult<()> {
    let policy_id: Option<Uuid> = tasks::table.find(patch_task).select(tasks::policy_id).first(c)?;
    let Some(pid) = policy_id else { return Ok(()) };
    let Some(def) = policies::table.find(pid).select(policies::definition).first::<Value>(c).optional()?.and_then(|v| serde_json::from_value::<PolicyDef>(v).ok()) else { return Ok(()) };
    let mode = def.reboot.mode.as_str();
    if mode != "auto" && mode != "user_prompt" {
        return Ok(());
    }
    let open: i64 = tasks::table.filter(tasks::host_id.eq(host_id)).filter(tasks::kind.eq("reboot")).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
    if open > 0 {
        return Ok(());
    }
    let asset: String = hosts::table.find(host_id).select(hosts::asset_type).first(c)?;
    let delay = if mode == "user_prompt" { 30 } else { default_delay(&asset) };
    let when = next_slot(&def, now);
    let exec = json!({ "delay_minutes": delay, "message": "" });
    let id: Uuid = diesel::insert_into(tasks::table)
        .values((tasks::host_id.eq(host_id), tasks::kind.eq("reboot"), tasks::status.eq("queued"), tasks::policy_id.eq(Some(pid)), tasks::parent_task_id.eq(Some(patch_task)), tasks::requested_by.eq("policy"), tasks::message.eq("reboot after patch (policy)"), tasks::exec.eq(Some(exec)), tasks::scheduled_for.eq(when)))
        .returning(tasks::id)
        .get_result(c)?;
    diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(format!("scheduled by policy for {when}")))).execute(c)?;
    audit::record(c, Event::new("system", "reboot.scheduled").target("task", &id.to_string()).host(host_id).details(json!({ "after_task": patch_task, "at": when, "mode": mode })))?;
    Ok(())
}
