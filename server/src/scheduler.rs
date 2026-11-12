//! Background loop: plan tasks from policies, push due commands, mark silent hosts, expire stale work.
use crate::audit::{self, Event};
use crate::error::ApiResult;
use crate::gateway;
use crate::planner;
use crate::schema::{hosts, task_events, tasks, waivers, host_packages};
use crate::state::AppState;
use actix_web::web;
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

const TICK_SECS: u64 = 20;
const PLAN_EVERY_N_TICKS: u64 = 3;
const ECO_HEARTBEAT_SECS: i64 = 30 * 60;

pub fn spawn(state: web::Data<AppState>) {
    actix_web::rt::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(TICK_SECS));
        let mut n = 0u64;
        loop {
            tick.tick().await;
            n += 1;
            if let Err(e) = state.db(housekeeping).await {
                tracing::warn!(error = ?e, "housekeeping failed");
            }
            if n % PLAN_EVERY_N_TICKS == 1 {
                let st = state.clone();
                match state.db(move |c| plan_all(c, &st)).await {
                    Ok(created) if created > 0 => tracing::info!(created, "policy planner created tasks"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = ?e, "planner failed"),
                }
                match state.db(|c| crate::compliance::run_planned(c, Utc::now())).await {
                    Ok(n) if n > 0 => tracing::info!(queued = n, "planned compliance audit started"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = ?e, "planned compliance audit failed"),
                }
            }
            for host in state.hub.connected_hosts() {
                gateway::push_pending(&state, host).await;
            }
        }
    });
}

fn plan_all(c: &mut PgConnection, state: &AppState) -> ApiResult<usize> {
    let now = Utc::now();
    let ai = crate::api::admin::load_setting(c, state, "ai").ok().and_then(|v| crate::ai::AiConfig::from_setting(&v)).map(crate::ai::Provider);
    let ids: Vec<Uuid> = host_packages::table.filter(host_packages::candidate.is_not_null()).select(host_packages::host_id).distinct().load(c)?;
    let mut created = 0;
    for id in ids {
        match planner::plan_for_host_with(c, id, now, ai.as_ref().map(|p| p as &dyn crate::ai::Advisor)) {
            Ok(Some(_)) => created += 1,
            Ok(None) => {}
            Err(e) => tracing::warn!(host = %id, error = ?e, "planning failed for host"),
        }
    }
    Ok(created)
}

/// Seconds of silence tolerated before a host is considered gone.
pub fn silence_limit(status: &str, heartbeat_secs: i32) -> i64 {
    let base = (heartbeat_secs.max(10) as i64) * 3;
    // Eco hosts report rarely on purpose; do not raise false alarms.
    if status == "eco" { base.max(ECO_HEARTBEAT_SECS * 3) } else { base.max(180) }
}

pub fn housekeeping(c: &mut PgConnection) -> ApiResult<()> {
    let now = Utc::now();
    mark_silent_hosts(c, now)?;
    expire_tasks(c, now)?;
    expire_waivers(c, now)?;
    // Resource history is kept two weeks.
    diesel::delete(crate::schema::host_metrics::table.filter(crate::schema::host_metrics::at.lt(now - Duration::days(14)))).execute(c)?;
    Ok(())
}

fn mark_silent_hosts(c: &mut PgConnection, now: DateTime<Utc>) -> ApiResult<()> {
    type R = (Uuid, String, i32, Option<DateTime<Utc>>, Option<DateTime<Utc>>, bool);
    let rows: Vec<R> = hosts::table
        .filter(hosts::status.eq_any(["online", "eco", "maintenance"]))
        .select((hosts::id, hosts::status, hosts::heartbeat_secs, hosts::last_seen_at, hosts::downtime_until, hosts::connected))
        .load(c)?;
    for (id, status, hb, seen, downtime, connected) in rows {
        if connected {
            continue;
        }
        let silent = seen.map(|s| (now - s).num_seconds() > silence_limit(&status, hb)).unwrap_or(true);
        if !silent {
            continue;
        }
        // Planned downtime that ran out of time is a real outage; silence outside of it is merely offline.
        let (new, action) = match downtime {
            Some(d) if d <= now => ("alert", "host.unresponsive"),
            Some(_) => continue,
            None => ("offline", "host.offline"),
        };
        diesel::update(hosts::table.find(id)).set(hosts::status.eq(new)).execute(c)?;
        audit::record(c, Event::new("system", action).target("host", &id.to_string()).host(id).details(json!({"previous": status})))?;
        if action == "host.unresponsive" {
            crate::reboot::overdue(c, id, now)?;
        }
    }
    Ok(())
}

fn expire_tasks(c: &mut PgConnection, now: DateTime<Utc>) -> ApiResult<()> {
    let fail = |c: &mut PgConnection, id: Uuid, host: Uuid, msg: &str| -> ApiResult<()> {
        diesel::update(tasks::table.find(id)).set((tasks::status.eq("failed"), tasks::message.eq(msg), tasks::finished_at.eq(now), tasks::updated_at.eq(now))).execute(c)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("failed"), task_events::message.eq(msg))).execute(c)?;
        audit::record(c, Event::new("system", "task.timeout").target("task", &id.to_string()).host(host).details(json!({"message": msg})))
    };
    let stale: Vec<(Uuid, Uuid)> = tasks::table.filter(tasks::status.eq_any(["sent", "running", "rolling_back"])).filter(tasks::updated_at.lt(now - Duration::minutes(90))).select((tasks::id, tasks::host_id)).load(c)?;
    for (id, host) in stale {
        fail(c, id, host, "agent stopped reporting")?;
    }
    let soak: Vec<(Uuid, Uuid)> = tasks::table.filter(tasks::status.eq("soaking")).filter(tasks::soak_until.lt(now - Duration::minutes(15))).select((tasks::id, tasks::host_id)).load(c)?;
    for (id, host) in soak {
        fail(c, id, host, "soak window ended without a verdict from the agent")?;
    }
    // A task planned for a patch day that ended before its host could run it: say so (it is re-planned for the next
    // patch day by the planner) instead of letting it run at an arbitrary hour.
    let missed: Vec<(Uuid, Uuid)> = tasks::table.filter(tasks::status.eq("queued")).filter(tasks::deadline_at.lt(now)).select((tasks::id, tasks::host_id)).load(c)?;
    for (id, host) in missed {
        let msg = "the patch day ended before the host could run it (offline?): it will be planned again for the next one";
        diesel::update(tasks::table.find(id)).set((tasks::status.eq("expired"), tasks::message.eq(msg), tasks::finished_at.eq(now), tasks::updated_at.eq(now))).execute(c)?;
        diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("expired"), task_events::message.eq(msg))).execute(c)?;
        audit::record(c, Event::new("system", "task.missed_window").target("task", &id.to_string()).host(host).details(json!({"message": msg})))?;
    }
    // Queued work counts from when it was due (a task approved for next month's patch day is not stale), approvals
    // from when they were requested.
    for (status, age) in [("queued", Duration::days(7)), ("pending_approval", Duration::days(14))] {
        let q = tasks::table.filter(tasks::status.eq(status)).into_boxed();
        let q = if status == "queued" { q.filter(tasks::scheduled_for.lt(now - age)) } else { q.filter(tasks::created_at.lt(now - age)) };
        let ids: Vec<Uuid> = q.select(tasks::id).load(c)?;
        for id in ids {
            diesel::update(tasks::table.find(id)).set((tasks::status.eq("expired"), tasks::finished_at.eq(now), tasks::updated_at.eq(now))).execute(c)?;
            diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("expired"), task_events::message.eq("not executed in time"))).execute(c)?;
        }
    }
    Ok(())
}

fn expire_waivers(c: &mut PgConnection, now: DateTime<Utc>) -> ApiResult<()> {
    let ids: Vec<Uuid> = waivers::table.filter(waivers::status.eq_any(["approved", "pending"])).filter(waivers::expires_at.le(now)).select(waivers::id).load(c)?;
    for id in ids {
        diesel::update(waivers::table.find(id)).set(waivers::status.eq("expired")).execute(c)?;
        audit::record(c, Event::new("system", "waiver.expired").target("waiver", &id.to_string()))?;
    }
    Ok(())
}
