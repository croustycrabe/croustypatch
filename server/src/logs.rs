//! Logs (Administration > Logs): one time-ordered view of what the server did, beside the audit trail (who changed
//! what). Four sources, merged newest first:
//! - `tasks`: every status change of every task, with its host (and, on the last one, the agent's log tail);
//! - `notifications`: deliveries, with the error of the failed ones;
//! - `reports`: each sending of a report;
//! - `server`: the server's own log lines (warnings and errors, and its own info lines), kept in memory since it
//!   started (the last [`SERVER_LINES`]); the full log stays on the container's standard output.
use crate::error::ApiResult;
use crate::schema::{hosts, notification_channels, notifications, report_runs, reports, task_events, tasks};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;
use uuid::Uuid;

pub const SOURCES: &[&str] = &["tasks", "notifications", "reports", "server"];
pub const SERVER_LINES: usize = 3000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LogEntry {
    pub ts: DateTime<Utc>,
    pub source: &'static str,
    /// "info", "warn" or "error".
    pub level: &'static str,
    pub title: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// The task, report or notification the line is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<Uuid>,
}

// ------------------------------------------------------------------------------ the server's own lines

static SERVER: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());

/// Keep a line of the server's log (called by [`CaptureLayer`]; public for tests).
pub fn push_server(ts: DateTime<Utc>, level: &'static str, target: &str, message: String) {
    let mut q = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    if q.len() >= SERVER_LINES {
        q.pop_front();
    }
    q.push_back(LogEntry { ts, source: "server", level, title: target.to_string(), message, host_id: None, hostname: None, ref_id: None });
}

fn server_lines() -> Vec<LogEntry> {
    SERVER.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

/// A tracing layer that copies the server's warnings and errors, and the info lines of CroustyPatch itself, into the
/// in-memory log. Request noise from libraries stays out.
pub struct CaptureLayer;

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let v = format!("{value:?}");
        if field.name() == "message" {
            self.0 = if self.0.is_empty() { v } else { format!("{v} {}", self.0) };
        } else {
            if !self.0.is_empty() {
                self.0.push(' ');
            }
            self.0 += &format!("{}={v}", field.name());
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = if self.0.is_empty() { value.to_string() } else { format!("{value} {}", self.0) };
        } else {
            self.record_debug(field, &value);
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let m = event.metadata();
        let level = match *m.level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO if m.target().starts_with("croustypatch") => "info",
            _ => return,
        };
        let mut f = Fields(String::new());
        event.record(&mut f);
        push_server(Utc::now(), level, m.target(), f.0);
    }
}

// ----------------------------------------------------------------------------------------- query

#[derive(Debug, Default, Deserialize)]
pub struct LogQuery {
    /// One of [`SOURCES`]; empty: all.
    #[serde(default)]
    pub source: String,
    /// "warn" (warnings and errors) or "error"; empty: everything.
    #[serde(default)]
    pub level: String,
    /// Words searched in the title and the message (case does not matter).
    #[serde(default)]
    pub q: String,
    pub host_id: Option<Uuid>,
    /// Lines strictly older than this (the `ts` of the last line of the previous page).
    pub before: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

fn rank(level: &str) -> u8 {
    match level {
        "error" => 2,
        "warn" => 1,
        _ => 0,
    }
}

fn task_level(status: &str) -> &'static str {
    match status {
        "failed" | "expired" => "error",
        "rolled_back" | "quarantined" | "cancelled" => "warn",
        _ => "info",
    }
}

/// Does a line pass the filters that are not applied in SQL?
pub fn keep(e: &LogEntry, q: &LogQuery) -> bool {
    let min = match q.level.as_str() {
        "error" => 2,
        "warn" => 1,
        _ => 0,
    };
    if rank(e.level) < min {
        return false;
    }
    if q.before.is_some_and(|b| e.ts >= b) {
        return false;
    }
    if q.host_id.is_some() && e.host_id != q.host_id {
        return false;
    }
    let words = q.q.to_lowercase();
    let hay = format!("{} {} {}", e.title, e.message, e.hostname.as_deref().unwrap_or("")).to_lowercase();
    words.split_whitespace().all(|w| hay.contains(w))
}

/// One page of the merged log, newest first.
pub fn query(c: &mut PgConnection, q: &LogQuery) -> ApiResult<Vec<LogEntry>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    // each source gives at most a few pages more than asked, the text filter being applied afterwards
    let fetch = (limit * 5) as i64;
    let want = |s: &str| q.source.is_empty() || q.source == s;
    let before = q.before.unwrap_or_else(|| Utc::now() + chrono::Duration::days(1));
    let mut out: Vec<LogEntry> = vec![];

    if want("tasks") {
        let mut sql = task_events::table.inner_join(tasks::table.inner_join(hosts::table)).filter(task_events::ts.lt(before)).into_boxed();
        if let Some(h) = q.host_id {
            sql = sql.filter(tasks::host_id.eq(h));
        }
        if q.level == "error" {
            sql = sql.filter(task_events::status.eq_any(["failed", "expired"]));
        } else if q.level == "warn" {
            sql = sql.filter(task_events::status.eq_any(["failed", "expired", "rolled_back", "quarantined", "cancelled"]));
        }
        let rows: Vec<(DateTime<Utc>, String, String, Uuid, String, Uuid, String)> =
            sql.order(task_events::ts.desc()).limit(fetch).select((task_events::ts, task_events::status, task_events::message, tasks::id, tasks::kind, hosts::id, hosts::hostname)).load(c)?;
        out.extend(rows.into_iter().map(|(ts, status, message, task, kind, host, hostname)| LogEntry {
            ts,
            source: "tasks",
            level: task_level(&status),
            title: format!("{kind} · {status}"),
            message,
            host_id: Some(host),
            hostname: Some(hostname),
            ref_id: Some(task),
        }));
    }
    if want("notifications") && q.host_id.is_none() {
        let rows: Vec<(DateTime<Utc>, Option<DateTime<Utc>>, String, String, String, i32, String, Uuid, String)> = notifications::table
            .inner_join(notification_channels::table.on(notification_channels::id.eq(notifications::channel_id)))
            .filter(notifications::created_at.lt(before))
            .order(notifications::created_at.desc())
            .limit(fetch)
            .select((notifications::created_at, notifications::sent_at, notifications::event, notifications::status, notifications::title, notifications::attempts, notifications::last_error, notifications::id, notification_channels::name))
            .load(c)?;
        out.extend(rows.into_iter().map(|(created, sent, event, status, title, attempts, err, id, channel)| {
            let level = if status == "failed" { "error" } else if !err.is_empty() { "warn" } else { "info" };
            let message = match status.as_str() {
                "sent" if attempts > 1 => format!("{title} — sent after {attempts} attempts ({err})"),
                "sent" => format!("{title} — sent"),
                "failed" => format!("{title} — failed after {attempts} attempt(s): {err}"),
                other if !err.is_empty() => format!("{title} — {other}, attempt {attempts}: {err}"),
                other => format!("{title} — {other}"),
            };
            LogEntry { ts: sent.filter(|s| *s < before).unwrap_or(created), source: "notifications", level, title: format!("{event} → {channel}"), message, host_id: None, hostname: None, ref_id: Some(id) }
        }));
    }
    if want("reports") && q.host_id.is_none() {
        let rows: Vec<(DateTime<Utc>, String, String, String, String, Uuid, String)> = report_runs::table
            .inner_join(reports::table)
            .filter(report_runs::at.lt(before))
            .order(report_runs::at.desc())
            .limit(fetch)
            .select((report_runs::at, report_runs::trigger, report_runs::status, report_runs::error, report_runs::title, reports::id, reports::name))
            .load(c)?;
        out.extend(rows.into_iter().map(|(at, trigger, status, error, title, id, name)| LogEntry {
            ts: at,
            source: "reports",
            level: if status == "failed" { "error" } else { "info" },
            title: format!("{name} · {status} ({trigger})"),
            message: if error.is_empty() { title } else { format!("{title}: {error}") },
            host_id: None,
            hostname: None,
            ref_id: Some(id),
        }));
    }
    if want("server") && q.host_id.is_none() {
        out.extend(server_lines());
    }
    out.retain(|e| keep(e, q));
    out.sort_by(|a, b| b.ts.cmp(&a.ts));
    out.truncate(limit);
    Ok(out)
}
