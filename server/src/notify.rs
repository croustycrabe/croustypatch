//! Notification module: rules route audit events to channels (e-mail, signed webhook, ntfy, Gotify), instantly or
//! as a daily digest.
//!
//! Flow: `audit::record` calls [`enqueue`] in the same transaction as the audit entry, so an event is queued exactly
//! when it is recorded. Matching rules write one outbox row per channel (`pending`, or `digest` for digest rules).
//! A background [`worker`] delivers due rows with retries and sends each digest rule once a day.
use crate::audit::Event;
use crate::crypto::MasterKey;
use crate::schema::{host_group_members, hosts, notification_channels, notification_rules, notifications, roles, users};
use crate::state::AppState;
use actix_web::web;
use chrono::{DateTime, Duration, Timelike, Utc};
use diesel::prelude::*;
use hmac::{Hmac, Mac};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

pub const CHANNEL_KINDS: &[&str] = &["smtp", "webhook", "ntfy", "gotify"];
pub const SEVERITIES: &[&str] = &["info", "medium", "high", "critical"];
const MAX_ATTEMPTS: i32 = 5;

pub fn severity_rank(s: &str) -> usize {
    SEVERITIES.iter().position(|x| *x == s).unwrap_or(0)
}

#[derive(Serialize, Clone, Copy)]
pub struct EventKind {
    pub key: &'static str,
    pub severity: &'static str,
    pub title_en: &'static str,
    pub title_fr: &'static str,
}

/// Events a rule can subscribe to, with their default severity.
pub const CATALOG: &[EventKind] = &[
    EventKind { key: "host.quarantined", severity: "critical", title_en: "Host quarantined", title_fr: "Hôte mis en quarantaine" },
    EventKind { key: "host.unresponsive", severity: "critical", title_en: "Host unresponsive after planned downtime", title_fr: "Hôte muet après un arrêt planifié" },
    EventKind { key: "reboot.overdue", severity: "critical", title_en: "Host did not come back after reboot", title_fr: "Hôte non revenu après redémarrage" },
    EventKind { key: "compliance.score_low", severity: "high", title_en: "Compliance score below threshold", title_fr: "Score de conformité sous le seuil" },
    EventKind { key: "task.failed", severity: "high", title_en: "Task failed", title_fr: "Tâche en échec" },
    EventKind { key: "task.rolled_back", severity: "high", title_en: "Patch rolled back", title_fr: "Correctif annulé (rollback)" },
    EventKind { key: "host.offline", severity: "medium", title_en: "Host offline", title_fr: "Hôte hors ligne" },
    EventKind { key: "approval.required", severity: "medium", title_en: "Patch awaiting approval", title_fr: "Correctif en attente d’approbation" },
    EventKind { key: "compliance.reported", severity: "info", title_en: "Compliance scan finished", title_fr: "Analyse de conformité terminée" },
    EventKind { key: "task.succeeded", severity: "info", title_en: "Patch applied", title_fr: "Correctif appliqué" },
    EventKind { key: "reboot.scheduled", severity: "info", title_en: "Reboot scheduled", title_fr: "Redémarrage planifié" },
    EventKind { key: "reboot.started", severity: "info", title_en: "Reboot started (downtime)", title_fr: "Redémarrage lancé (arrêt planifié)" },
    EventKind { key: "reboot.completed", severity: "info", title_en: "Host back after reboot", title_fr: "Hôte revenu après redémarrage" },
    EventKind { key: "notification.test", severity: "info", title_en: "Test notification", title_fr: "Notification de test" },
];

pub fn catalog_entry(key: &str) -> Option<&'static EventKind> {
    CATALOG.iter().find(|e| e.key == key)
}

/// Map an audit action (with its details) to a notification event key, or None if it is not notifiable.
pub fn event_key(action: &str, details: &Value) -> Option<&'static str> {
    let kind = details.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    match action {
        "task.planned" if details.get("action").and_then(|v| v.as_str()) == Some("REQUIRE_APPROVAL") => Some("approval.required"),
        // A successful scan or control task is routine; patches, rollbacks and reboots are worth telling.
        "task.succeeded" if !matches!(kind, "patch" | "rollback") => None,
        a => catalog_entry(a).map(|e| e.key),
    }
}

#[derive(Queryable)]
struct Rule {
    id: Uuid,
    name: String,
    events: Value,
    min_severity: String,
    frequency: String,
    group_ids: Value,
    channel_ids: Value,
    roles: Value,
    emails: Value,
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}
fn uuids(v: &Value) -> Vec<Uuid> {
    strs(v).iter().filter_map(|s| s.parse().ok()).collect()
}

/// The message for one event, in English (titles are short; the UI shows the French catalog names).
pub fn render(kind: &EventKind, host: Option<&str>, details: &Value) -> (String, String) {
    let sev = kind.severity;
    let title = match host {
        Some(h) => format!("[CroustyPatch] {} · {} · {h}", cap(sev), kind.title_en),
        None => format!("[CroustyPatch] {} · {}", cap(sev), kind.title_en),
    };
    let mut body = vec![kind.title_en.to_string()];
    if let Some(h) = host {
        body.push(format!("Host: {h}"));
    }
    if let Some(m) = details.get("message").and_then(|v| v.as_str()).filter(|m| !m.is_empty()) {
        body.push(format!("Details: {m}"));
    }
    if let Some(k) = details.get("kind").and_then(|v| v.as_str()) {
        body.push(format!("Task: {k}"));
    }
    (title, body.join("\n"))
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Queue the notifications an audit event triggers. Runs inside the audit transaction.
pub fn enqueue(c: &mut PgConnection, ev: &Event<'_>) -> QueryResult<usize> {
    let Some(key) = event_key(ev.action, &ev.details) else { return Ok(0) };
    let kind = catalog_entry(key).expect("catalog key");
    let rules: Vec<Rule> = notification_rules::table
        .filter(notification_rules::enabled.eq(true))
        .select((notification_rules::id, notification_rules::name, notification_rules::events, notification_rules::min_severity, notification_rules::frequency, notification_rules::group_ids, notification_rules::channel_ids, notification_rules::roles, notification_rules::emails))
        .load(c)?;
    let rules: Vec<Rule> = rules.into_iter().filter(|r| strs(&r.events).iter().any(|e| e == key) && severity_rank(kind.severity) >= severity_rank(&r.min_severity)).collect();
    if rules.is_empty() {
        return Ok(0);
    }
    let host_name: Option<String> = match ev.host_id {
        Some(h) => hosts::table.find(h).select(hosts::hostname).first(c).optional()?,
        None => None,
    };
    let host_groups: Vec<Uuid> = match ev.host_id {
        Some(h) => host_group_members::table.filter(host_group_members::host_id.eq(h)).select(host_group_members::group_id).load(c)?,
        None => vec![],
    };
    let (title, body) = render(kind, host_name.as_deref(), &ev.details);
    let payload = json!({
        "event": key, "severity": kind.severity, "title": kind.title_en,
        "host": ev.host_id.map(|id| json!({ "id": id, "name": host_name })),
        "target": { "type": ev.target_type, "id": ev.target_id },
        "details": ev.details, "at": Utc::now(),
    });
    let mut n = 0;
    for r in rules {
        let scope = uuids(&r.group_ids);
        if !scope.is_empty() && !scope.iter().any(|g| host_groups.contains(g)) {
            continue;
        }
        let recipients = recipients(c, &r)?;
        let status = if r.frequency == "digest" { "digest" } else { "pending" };
        for ch in uuids(&r.channel_ids) {
            let enabled: Option<bool> = notification_channels::table.find(ch).select(notification_channels::enabled).first(c).optional()?;
            if enabled != Some(true) {
                continue;
            }
            diesel::insert_into(notifications::table)
                .values((
                    notifications::rule_id.eq(Some(r.id)),
                    notifications::channel_id.eq(ch),
                    notifications::event.eq(key),
                    notifications::severity.eq(kind.severity),
                    notifications::title.eq(&title),
                    notifications::body.eq(&body),
                    notifications::payload.eq(&payload),
                    notifications::recipients.eq(json!(recipients)),
                    notifications::status.eq(status),
                ))
                .execute(c)?;
            n += 1;
        }
        tracing::debug!(rule = %r.name, event = key, "notification queued");
    }
    Ok(n)
}

/// E-mail addresses for a rule: users of its roles (with an address) plus its extra addresses.
fn recipients(c: &mut PgConnection, r: &Rule) -> QueryResult<Vec<String>> {
    let mut out = strs(&r.emails);
    let role_names = strs(&r.roles);
    if !role_names.is_empty() {
        let mails: Vec<Option<String>> = users::table
            .inner_join(roles::table)
            .filter(roles::name.eq_any(&role_names))
            .filter(users::disabled.eq(false))
            .select(users::email)
            .load(c)?;
        out.extend(mails.into_iter().flatten().filter(|m| !m.is_empty()));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

// ------------------------------------------------------------------ delivery

#[derive(Clone)]
pub struct Message {
    pub id: Uuid,
    pub event: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub payload: Value,
    pub recipients: Vec<String>,
}

pub struct Channel {
    pub kind: String,
    pub config: Value,
    pub secret: zeroize::Zeroizing<String>,
}

impl Channel {
    pub fn load(c: &mut PgConnection, master: &MasterKey, id: Uuid) -> anyhow::Result<Self> {
        let (kind, config, secret): (String, Value, String) = notification_channels::table.find(id).select((notification_channels::kind, notification_channels::config, notification_channels::secret)).first(c)?;
        let secret = if secret.is_empty() { zeroize::Zeroizing::new(String::new()) } else { master.decrypt_string(&secret).map_err(|_| anyhow::anyhow!("channel secret cannot be decrypted (master key changed?)"))? };
        Ok(Self { kind, config, secret })
    }
}

fn cfg_str<'a>(cfg: &'a Value, k: &str) -> &'a str {
    cfg.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

/// HMAC-SHA256 signature of a webhook delivery: hex(HMAC(key, "{timestamp}.{body}")).
pub fn webhook_signature(key: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(timestamp.to_string().as_bytes());
    m.update(b".");
    m.update(body);
    hex::encode(m.finalize().into_bytes())
}

fn priority(sev: &str, scale: [i64; 4]) -> i64 {
    scale[severity_rank(sev)]
}

/// Deliver one message on one channel.
pub async fn send(ch: &Channel, m: &Message) -> anyhow::Result<()> {
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent(concat!("croustypatch-server/", env!("CARGO_PKG_VERSION"))).build()?;
    match ch.kind.as_str() {
        "webhook" => {
            let body = serde_json::to_vec(&json!({ "id": m.id, "event": m.event, "severity": m.severity, "title": m.title, "text": m.body, "data": m.payload }))?;
            let ts = Utc::now().timestamp();
            let mut req = http.post(cfg_str(&ch.config, "url")).header("content-type", "application/json").header("x-croustypatch-event", &m.event).header("x-croustypatch-delivery", m.id.to_string()).header("x-croustypatch-timestamp", ts.to_string());
            if !ch.secret.is_empty() {
                req = req.header("x-croustypatch-signature", format!("sha256={}", webhook_signature(ch.secret.as_bytes(), ts, &body)));
            }
            let r = req.body(body).send().await?;
            anyhow::ensure!(r.status().is_success(), "webhook answered {}", r.status());
        }
        "ntfy" => {
            let url = format!("{}/{}", cfg_str(&ch.config, "url").trim_end_matches('/'), cfg_str(&ch.config, "topic"));
            let mut req = http.post(url).header("title", ascii_header(&m.title)).header("priority", priority(&m.severity, [2, 3, 4, 5]).to_string()).header("tags", format!("croustypatch,{}", m.severity));
            if !ch.secret.is_empty() {
                req = req.bearer_auth(ch.secret.as_str());
            }
            let r = req.body(m.body.clone()).send().await?;
            anyhow::ensure!(r.status().is_success(), "ntfy answered {}", r.status());
        }
        "gotify" => {
            let url = format!("{}/message", cfg_str(&ch.config, "url").trim_end_matches('/'));
            let r = http.post(url).header("x-gotify-key", ch.secret.as_str()).json(&json!({ "title": m.title, "message": m.body, "priority": priority(&m.severity, [2, 5, 8, 10]) })).send().await?;
            anyhow::ensure!(r.status().is_success(), "gotify answered {}", r.status());
        }
        "smtp" => send_mail(ch, m).await?,
        k => anyhow::bail!("unknown channel kind {k}"),
    }
    Ok(())
}

/// HTTP header values must be visible ASCII: ntfy titles keep their meaning without accents.
fn ascii_header(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c } else { '?' }).collect()
}

async fn send_mail(ch: &Channel, m: &Message) -> anyhow::Result<()> {
    use lettre::message::{header::ContentType, Mailbox};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
    let cfg = &ch.config;
    let host = cfg_str(cfg, "host");
    let port = cfg.get("port").and_then(|v| v.as_u64()).unwrap_or(587) as u16;
    let mut to: Vec<String> = m.recipients.clone();
    if to.is_empty() {
        to = strs(cfg.get("to").unwrap_or(&Value::Null));
    }
    anyhow::ensure!(!to.is_empty(), "no recipient: add addresses to the channel or the rule");
    let from: Mailbox = cfg_str(cfg, "from").parse().map_err(|e| anyhow::anyhow!("bad sender address: {e}"))?;
    let mut b = lettre::Message::builder().from(from).subject(&m.title).header(ContentType::TEXT_PLAIN);
    for r in &to {
        b = b.to(r.parse().map_err(|e| anyhow::anyhow!("bad recipient {r}: {e}"))?);
    }
    let email = b.body(m.body.clone())?;
    let mut t = match cfg_str(cfg, "tls") {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(host)?,
        "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?,
    }
    .port(port)
    .timeout(Some(std::time::Duration::from_secs(20)));
    let user = cfg_str(cfg, "username");
    if !user.is_empty() {
        t = t.credentials(Credentials::new(user.to_string(), ch.secret.to_string()));
    }
    t.build().send(email).await?;
    Ok(())
}

// ------------------------------------------------------------------ worker

type Due = (Uuid, Uuid, String, String, String, String, Value, Value, i32);

/// Deliver due notifications and send digests; runs every few seconds.
pub fn worker(state: web::Data<AppState>) {
    actix_web::rt::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tick.tick().await;
            if let Err(e) = run_once(&state).await {
                tracing::warn!(error = %e, "notification worker");
            }
        }
    });
}

/// One pass of the worker (public for tests).
pub async fn run_once(state: &web::Data<AppState>) -> anyhow::Result<usize> {
    let now = Utc::now();
    let due: Vec<Due> = state
        .db(move |c| {
            Ok(notifications::table
                .filter(notifications::status.eq("pending"))
                .filter(notifications::next_attempt_at.le(now))
                .order(notifications::created_at.asc())
                .limit(50)
                .select((notifications::id, notifications::channel_id, notifications::event, notifications::severity, notifications::title, notifications::body, notifications::payload, notifications::recipients, notifications::attempts))
                .load(c)?)
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut sent = 0;
    for (id, ch, event, severity, title, body, payload, recipients, attempts) in due {
        let m = Message { id, event, severity, title, body, payload, recipients: strs(&recipients) };
        let res = deliver(state, ch, &m).await;
        let ok = res.is_ok();
        sent += ok as usize;
        let err = res.err().map(|e| format!("{e:#}")).unwrap_or_default();
        state
            .db(move |c| {
                let a = attempts + 1;
                if ok {
                    diesel::update(notifications::table.find(id)).set((notifications::status.eq("sent"), notifications::attempts.eq(a), notifications::sent_at.eq(Some(Utc::now())), notifications::last_error.eq(""))).execute(c)?;
                } else {
                    let status = if a >= MAX_ATTEMPTS { "failed" } else { "pending" };
                    let next = Utc::now() + Duration::minutes(1 << a.min(6));
                    diesel::update(notifications::table.find(id)).set((notifications::status.eq(status), notifications::attempts.eq(a), notifications::last_error.eq(&err), notifications::next_attempt_at.eq(next))).execute(c)?;
                }
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    }
    sent += send_digests(state, now).await?;
    Ok(sent)
}

async fn deliver(state: &web::Data<AppState>, channel: Uuid, m: &Message) -> anyhow::Result<()> {
    let st = state.clone();
    let ch = state.db(move |c| Channel::load(c, &st.master, channel).map_err(|e| crate::error::ApiError::bad(format!("{e:#}")))).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    send(&ch, m).await
}

/// Is it time for this rule's daily digest?
pub fn digest_due(hour: i32, last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    now.hour() as i32 == hour && last.is_none_or(|l| l.date_naive() < now.date_naive())
}

async fn send_digests(state: &web::Data<AppState>, now: DateTime<Utc>) -> anyhow::Result<usize> {
    let rules: Vec<(Uuid, String, i32, Option<DateTime<Utc>>)> = state
        .db(|c| Ok(notification_rules::table.filter(notification_rules::enabled.eq(true)).filter(notification_rules::frequency.eq("digest")).select((notification_rules::id, notification_rules::name, notification_rules::digest_hour, notification_rules::last_digest_at)).load(c)?))
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut sent = 0;
    for (rule, name, hour, last) in rules {
        if !digest_due(hour, last, now) {
            continue;
        }
        sent += send_digest(state, rule, &name).await?;
    }
    Ok(sent)
}

/// Combine a rule's queued events into one message per channel and send it now (public for the "send now" button
/// and tests).
pub async fn send_digest(state: &web::Data<AppState>, rule: Uuid, name: &str) -> anyhow::Result<usize> {
    type Row = (Uuid, Uuid, String, String, Value);
    let rows: Vec<Row> = state
        .db(move |c| Ok(notifications::table.filter(notifications::rule_id.eq(rule)).filter(notifications::status.eq("digest")).order(notifications::created_at.asc()).select((notifications::id, notifications::channel_id, notifications::severity, notifications::title, notifications::recipients)).load(c)?))
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut by_channel: std::collections::BTreeMap<Uuid, Vec<Row>> = Default::default();
    for r in rows {
        by_channel.entry(r.1).or_default().push(r);
    }
    let mut sent = 0;
    for (ch, items) in by_channel {
        let worst = items.iter().map(|r| severity_rank(&r.2)).max().unwrap_or(0);
        let lines: Vec<String> = items.iter().map(|r| format!("- {}", r.3.trim_start_matches("[CroustyPatch] "))).collect();
        let m = Message {
            id: Uuid::new_v4(),
            event: "digest".into(),
            severity: SEVERITIES[worst].into(),
            title: format!("[CroustyPatch] Daily summary · {name} · {} event(s)", items.len()),
            body: lines.join("\n"),
            payload: json!({ "rule": rule, "events": items.len() }),
            recipients: strs(&items[0].4),
        };
        let res = deliver(state, ch, &m).await;
        let ids: Vec<Uuid> = items.iter().map(|r| r.0).collect();
        let err = res.as_ref().err().map(|e| format!("{e:#}")).unwrap_or_default();
        let ok = res.is_ok();
        state
            .db(move |c| {
                if ok {
                    diesel::update(notifications::table.filter(notifications::id.eq_any(&ids))).set((notifications::status.eq("sent"), notifications::sent_at.eq(Some(Utc::now())))).execute(c)?;
                } else {
                    diesel::update(notifications::table.filter(notifications::id.eq_any(&ids))).set(notifications::last_error.eq(&err)).execute(c)?;
                }
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        sent += ok as usize;
    }
    state.db(move |c| Ok(diesel::update(notification_rules::table.find(rule)).set(notification_rules::last_digest_at.eq(Some(Utc::now()))).execute(c)?)).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(sent)
}
