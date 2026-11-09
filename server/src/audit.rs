//! Append-only, hash-chained audit trail (ISO 27001 change management / SOC 2).
//!
//! Each entry commits to the previous one: `hash = SHA-256(prev_hash | fields…)`. The database
//! additionally refuses UPDATE/DELETE/TRUNCATE on the table, so tampering requires bypassing both.
use crate::error::{ApiError, ApiResult};
use crate::models::AuditEntry;
use crate::schema::audit_log;
use chrono::{DateTime, Utc};
use croustypatch_common::crypto::sha256_hex;
use diesel::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub const GENESIS: &str = "GENESIS";
/// Serialises chain appends across server instances.
const LOCK_ID: i64 = 0x4352_4f55_5354_59; // "CROUSTY"

#[allow(clippy::too_many_arguments)]
pub fn entry_hash(prev: &str, ts: DateTime<Utc>, actor: &str, action: &str, ttype: &str, tid: &str, host: Option<Uuid>, details: &Value, ip: &str) -> String {
    // serde_json::Value keeps keys sorted, so this is canonical; jsonb round-trips it unchanged.
    let body = format!(
        "{prev}|{}|{actor}|{action}|{ttype}|{tid}|{}|{}|{ip}",
        ts.timestamp_micros(),
        host.map(|h| h.to_string()).unwrap_or_default(),
        serde_json::to_string(details).unwrap_or_default()
    );
    sha256_hex(body.as_bytes())
}

pub struct Event<'a> {
    pub actor: &'a str,
    pub action: &'a str,
    pub target_type: &'a str,
    pub target_id: &'a str,
    pub host_id: Option<Uuid>,
    pub details: Value,
    pub ip: &'a str,
}

impl<'a> Event<'a> {
    pub fn new(actor: &'a str, action: &'a str) -> Self {
        Self { actor, action, target_type: "", target_id: "", host_id: None, details: json!({}), ip: "" }
    }
    pub fn target(mut self, ttype: &'a str, id: &'a str) -> Self {
        self.target_type = ttype;
        self.target_id = id;
        self
    }
    pub fn host(mut self, id: Uuid) -> Self {
        self.host_id = Some(id);
        self
    }
    pub fn details(mut self, d: Value) -> Self {
        self.details = d;
        self
    }
    pub fn ip(mut self, ip: &'a str) -> Self {
        self.ip = ip;
        self
    }
}

/// Append an entry. Safe to call inside another transaction (the advisory lock is transaction-scoped).
pub fn record(conn: &mut PgConnection, ev: Event<'_>) -> ApiResult<()> {
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        diesel::sql_query(format!("SELECT pg_advisory_xact_lock({LOCK_ID})")).execute(conn)?;
        let prev: String = audit_log::table.select(audit_log::hash).order(audit_log::seq.desc()).first(conn).optional()?.unwrap_or_else(|| GENESIS.to_string());
        let ts = DateTime::from_timestamp_micros(Utc::now().timestamp_micros()).unwrap_or_else(Utc::now);
        let hash = entry_hash(&prev, ts, ev.actor, ev.action, ev.target_type, ev.target_id, ev.host_id, &ev.details, ev.ip);
        diesel::insert_into(audit_log::table)
            .values((
                audit_log::ts.eq(ts),
                audit_log::actor.eq(ev.actor),
                audit_log::action.eq(ev.action),
                audit_log::target_type.eq(ev.target_type),
                audit_log::target_id.eq(ev.target_id),
                audit_log::host_id.eq(ev.host_id),
                audit_log::details.eq(&ev.details),
                audit_log::ip.eq(ev.ip),
                audit_log::prev_hash.eq(&prev),
                audit_log::hash.eq(&hash),
            ))
            .execute(conn)?;
        // Queue the notifications this event triggers, atomically with the entry. A broken rule must never block
        // the audit trail: the savepoint is rolled back and the error logged.
        if crate::notify::event_key(ev.action, &ev.details).is_some() {
            if let Err(e) = conn.transaction(|c| crate::notify::enqueue(c, &ev)) {
                tracing::warn!(action = ev.action, error = %e, "notification enqueue failed");
            }
        }
        Ok(())
    })
    .map_err(ApiError::from)
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Verification {
    pub ok: bool,
    pub entries: i64,
    pub first_bad_seq: Option<i64>,
    pub head_hash: String,
}

/// Re-compute the whole chain.
pub fn verify(conn: &mut PgConnection) -> ApiResult<Verification> {
    let mut prev = GENESIS.to_string();
    let mut last_seq = 0i64;
    let mut count = 0i64;
    loop {
        let rows: Vec<AuditEntry> = audit_log::table.filter(audit_log::seq.gt(last_seq)).order(audit_log::seq.asc()).limit(500).select(AuditEntry::as_select()).load(conn)?;
        if rows.is_empty() {
            break;
        }
        for r in rows {
            let expect = entry_hash(&prev, r.ts, &r.actor, &r.action, &r.target_type, &r.target_id, r.host_id, &r.details, &r.ip);
            if r.prev_hash != prev || r.hash != expect {
                return Ok(Verification { ok: false, entries: count, first_bad_seq: Some(r.seq), head_hash: prev });
            }
            prev = r.hash;
            last_seq = r.seq;
            count += 1;
        }
    }
    Ok(Verification { ok: true, entries: count, first_bad_seq: None, head_hash: prev })
}
