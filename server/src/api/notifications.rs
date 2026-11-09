//! Notification settings: channels, rules, test sends, delivery history.
use super::name_ok;
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::notify::{self, Channel, Message, CATALOG, CHANNEL_KINDS, SEVERITIES};
use crate::schema::{notification_channels, notification_rules, notifications};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub async fn catalog(user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    Ok(HttpResponse::Ok().json(json!({ "events": CATALOG, "kinds": CHANNEL_KINDS, "severities": SEVERITIES })))
}

// ------------------------------------------------------------------ channels

#[derive(Deserialize)]
pub struct ChannelBody {
    name: String,
    kind: String,
    #[serde(default)]
    config: Value,
    /// None: keep the stored secret; Some(""): clear it; Some(x): replace it.
    secret: Option<String>,
    #[serde(default = "yes")]
    enabled: bool,
}
fn yes() -> bool {
    true
}

fn http_url(v: &str, what: &str) -> ApiResult<()> {
    let ok = (v.starts_with("https://") || v.starts_with("http://")) && v.len() > 8 && v.len() <= 2048 && !v.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok { Ok(()) } else { Err(ApiError::bad(format!("{what} must be an http(s) URL"))) }
}

fn email_ok(v: &str) -> bool {
    let v = v.trim();
    v.len() <= 254 && v.contains('@') && !v.starts_with('@') && !v.ends_with('@') && !v.chars().any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>')
}

/// Keep only the known settings of each kind, validated.
pub fn clean_config(kind: &str, cfg: &Value) -> ApiResult<Value> {
    let s = |k: &str| cfg.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    match kind {
        "webhook" => {
            http_url(&s("url"), "url")?;
            Ok(json!({ "url": s("url") }))
        }
        "ntfy" => {
            http_url(&s("url"), "url")?;
            let topic = s("topic");
            if topic.is_empty() || topic.len() > 64 || !topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(ApiError::bad("topic must be 1-64 letters, digits, - or _"));
            }
            Ok(json!({ "url": s("url"), "topic": topic }))
        }
        "gotify" => {
            http_url(&s("url"), "url")?;
            Ok(json!({ "url": s("url") }))
        }
        "smtp" => {
            let host = s("host");
            if host.is_empty() || host.len() > 253 || host.chars().any(|c| c.is_whitespace() || c == '/') {
                return Err(ApiError::bad("host is required"));
            }
            let port = cfg.get("port").and_then(|v| v.as_u64()).unwrap_or(587);
            if !(1..=65535).contains(&port) {
                return Err(ApiError::bad("port must be 1-65535"));
            }
            let tls = match s("tls").as_str() { "" => "starttls".to_string(), t => t.to_string() };
            if !["starttls", "tls", "none"].contains(&tls.as_str()) {
                return Err(ApiError::bad("tls must be starttls, tls or none"));
            }
            if !email_ok(&s("from")) {
                return Err(ApiError::bad("from must be an e-mail address"));
            }
            let to: Vec<String> = cfg.get("to").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            if let Some(bad) = to.iter().find(|t| !email_ok(t)) {
                return Err(ApiError::bad(format!("not an e-mail address: {bad}")));
            }
            Ok(json!({ "host": host, "port": port, "tls": tls, "username": s("username"), "from": s("from"), "to": to }))
        }
        _ => Err(ApiError::bad(format!("kind must be one of: {}", CHANNEL_KINDS.join(", ")))),
    }
}

#[derive(Queryable, serde::Serialize)]
struct ChannelRow {
    id: Uuid,
    name: String,
    kind: String,
    config: Value,
    #[serde(skip)]
    secret: String,
    enabled: bool,
    created_at: DateTime<Utc>,
}

fn channel_json(r: ChannelRow) -> Value {
    let has_secret = !r.secret.is_empty();
    let mut v = serde_json::to_value(&r).unwrap_or_default();
    v["has_secret"] = json!(has_secret);
    v
}

const CH_COLS: (notification_channels::id, notification_channels::name, notification_channels::kind, notification_channels::config, notification_channels::secret, notification_channels::enabled, notification_channels::created_at) =
    (notification_channels::id, notification_channels::name, notification_channels::kind, notification_channels::config, notification_channels::secret, notification_channels::enabled, notification_channels::created_at);

pub async fn list_channels(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let rows: Vec<ChannelRow> = state.db(|c| Ok(notification_channels::table.order(notification_channels::name.asc()).select(CH_COLS).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows.into_iter().map(channel_json).collect::<Vec<_>>()))
}

pub async fn create_channel(state: web::Data<AppState>, user: AuthUser, body: web::Json<ChannelBody>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    let config = clean_config(&b.kind, &b.config)?;
    let sealed = b.secret.filter(|s| !s.is_empty()).map(|s| state.master.encrypt(s.as_bytes())).unwrap_or_default();
    let row: ChannelRow = state
        .db(move |c| {
            c.transaction(|c| {
                let r: ChannelRow = diesel::insert_into(notification_channels::table)
                    .values((notification_channels::name.eq(b.name.trim()), notification_channels::kind.eq(&b.kind), notification_channels::config.eq(&config), notification_channels::secret.eq(&sealed), notification_channels::enabled.eq(b.enabled)))
                    .returning(CH_COLS)
                    .get_result(c)
                    .map_err(|e| match e {
                        diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => ApiError::Conflict("a channel with this name exists".into()),
                        e => e.into(),
                    })?;
                audit::record(c, Event::new(&user.username, "notification.channel_created").target("channel", &r.id.to_string()).details(json!({ "name": r.name, "kind": r.kind })).ip(&user.ip))?;
                Ok(r)
            })
        })
        .await?;
    Ok(HttpResponse::Created().json(channel_json(row)))
}

pub async fn update_channel(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<ChannelBody>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    let config = clean_config(&b.kind, &b.config)?;
    let sealed: Option<String> = b.secret.map(|s| if s.is_empty() { String::new() } else { state.master.encrypt(s.as_bytes()) });
    let row: ChannelRow = state
        .db(move |c| {
            c.transaction(|c| {
                let target = notification_channels::table.find(id);
                let n = diesel::update(target).set((notification_channels::name.eq(b.name.trim()), notification_channels::kind.eq(&b.kind), notification_channels::config.eq(&config), notification_channels::enabled.eq(b.enabled), notification_channels::updated_at.eq(Utc::now()))).execute(c)?;
                if n == 0 {
                    return Err(ApiError::NotFound("channel"));
                }
                if let Some(s) = &sealed {
                    diesel::update(target).set(notification_channels::secret.eq(s)).execute(c)?;
                }
                audit::record(c, Event::new(&user.username, "notification.channel_updated").target("channel", &id.to_string()).details(json!({ "secret_changed": sealed.is_some() })).ip(&user.ip))?;
                Ok(target.select(CH_COLS).first(c)?)
            })
        })
        .await?;
    Ok(HttpResponse::Ok().json(channel_json(row)))
}

pub async fn delete_channel(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            c.transaction(|c| {
                if diesel::delete(notification_channels::table.find(id)).execute(c)? == 0 {
                    return Err(ApiError::NotFound("channel"));
                }
                audit::record(c, Event::new(&user.username, "notification.channel_deleted").target("channel", &id.to_string()).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

/// Send a test message now and report the outcome (the error text helps fix a wrong host or token).
pub async fn test_channel(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    let st = state.clone();
    let ch: Channel = state.db(move |c| Channel::load(c, &st.master, id).map_err(|_| ApiError::NotFound("channel"))).await?;
    let kind = notify::catalog_entry("notification.test").expect("catalog");
    let (title, body) = notify::render(kind, None, &json!({ "message": format!("Sent by {} from the notification settings.", user.username) }));
    let to = if ch.kind == "smtp" { user_email(&state, &user.username).await?.into_iter().collect() } else { vec![] };
    let m = Message { id: Uuid::new_v4(), event: "notification.test".into(), severity: "info".into(), title, body, payload: json!({ "test": true }), recipients: to };
    let res = notify::send(&ch, &m).await;
    let ok = res.is_ok();
    let err = res.err().map(|e| format!("{e:#}"));
    let u = user.username.clone();
    let ip = user.ip.clone();
    state.db(move |c| audit::record(c, Event::new(&u, "notification.channel_tested").target("channel", &id.to_string()).details(json!({ "ok": ok })).ip(&ip))).await?;
    Ok(HttpResponse::Ok().json(json!({ "ok": ok, "error": err })))
}

async fn user_email(state: &web::Data<AppState>, username: &str) -> ApiResult<Option<String>> {
    use crate::schema::users;
    let u = username.to_string();
    state.db(move |c| Ok(users::table.filter(users::username.eq(u)).select(users::email).first::<Option<String>>(c).optional()?.flatten().filter(|e| !e.is_empty()))).await
}

// ------------------------------------------------------------------ rules

#[derive(Deserialize)]
pub struct RuleBody {
    name: String,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default = "high")]
    min_severity: String,
    #[serde(default = "instant")]
    frequency: String,
    #[serde(default = "eight")]
    digest_hour: i32,
    #[serde(default)]
    group_ids: Vec<Uuid>,
    #[serde(default)]
    channel_ids: Vec<Uuid>,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    emails: Vec<String>,
}
fn high() -> String {
    "high".into()
}
fn instant() -> String {
    "instant".into()
}
fn eight() -> i32 {
    8
}

fn rule_ok(b: &RuleBody) -> ApiResult<()> {
    name_ok("name", &b.name)?;
    if b.events.is_empty() {
        return Err(ApiError::bad("choose at least one event"));
    }
    if let Some(e) = b.events.iter().find(|e| notify::catalog_entry(e).is_none()) {
        return Err(ApiError::bad(format!("unknown event {e}")));
    }
    if !SEVERITIES.contains(&b.min_severity.as_str()) {
        return Err(ApiError::bad("min_severity must be info, medium, high or critical"));
    }
    if !["instant", "digest"].contains(&b.frequency.as_str()) {
        return Err(ApiError::bad("frequency must be instant or digest"));
    }
    if !(0..=23).contains(&b.digest_hour) {
        return Err(ApiError::bad("digest_hour must be 0-23"));
    }
    if b.channel_ids.is_empty() {
        return Err(ApiError::bad("choose at least one channel"));
    }
    if let Some(bad) = b.emails.iter().find(|e| !email_ok(e)) {
        return Err(ApiError::bad(format!("not an e-mail address: {bad}")));
    }
    Ok(())
}

type RuleRow = (Uuid, String, bool, Value, String, String, i32, Value, Value, Value, Value, Option<DateTime<Utc>>);

fn rule_json(r: RuleRow) -> Value {
    json!({ "id": r.0, "name": r.1, "enabled": r.2, "events": r.3, "min_severity": r.4, "frequency": r.5, "digest_hour": r.6, "group_ids": r.7, "channel_ids": r.8, "roles": r.9, "emails": r.10, "last_digest_at": r.11 })
}

const RULE_COLS: (
    notification_rules::id,
    notification_rules::name,
    notification_rules::enabled,
    notification_rules::events,
    notification_rules::min_severity,
    notification_rules::frequency,
    notification_rules::digest_hour,
    notification_rules::group_ids,
    notification_rules::channel_ids,
    notification_rules::roles,
    notification_rules::emails,
    notification_rules::last_digest_at,
) = (
    notification_rules::id,
    notification_rules::name,
    notification_rules::enabled,
    notification_rules::events,
    notification_rules::min_severity,
    notification_rules::frequency,
    notification_rules::digest_hour,
    notification_rules::group_ids,
    notification_rules::channel_ids,
    notification_rules::roles,
    notification_rules::emails,
    notification_rules::last_digest_at,
);

pub async fn list_rules(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let rows: Vec<RuleRow> = state.db(|c| Ok(notification_rules::table.order(notification_rules::name.asc()).select(RULE_COLS).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows.into_iter().map(rule_json).collect::<Vec<_>>()))
}

pub async fn save_rule(state: web::Data<AppState>, user: AuthUser, path: Option<web::Path<Uuid>>, body: web::Json<RuleBody>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let b = body.into_inner();
    rule_ok(&b)?;
    let id = path.map(|p| p.into_inner());
    let row: RuleRow = state
        .db(move |c| {
            c.transaction(|c| {
                let vals = (
                    notification_rules::name.eq(b.name.trim().to_string()),
                    notification_rules::enabled.eq(b.enabled),
                    notification_rules::events.eq(json!(b.events)),
                    notification_rules::min_severity.eq(b.min_severity.clone()),
                    notification_rules::frequency.eq(b.frequency.clone()),
                    notification_rules::digest_hour.eq(b.digest_hour),
                    notification_rules::group_ids.eq(json!(b.group_ids)),
                    notification_rules::channel_ids.eq(json!(b.channel_ids)),
                    notification_rules::roles.eq(json!(b.roles)),
                    notification_rules::emails.eq(json!(b.emails.iter().map(|e| e.trim()).collect::<Vec<_>>())),
                );
                let dup = |e: diesel::result::Error| match e {
                    diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => ApiError::Conflict("a rule with this name exists".into()),
                    diesel::result::Error::NotFound => ApiError::NotFound("rule"),
                    e => e.into(),
                };
                let r: RuleRow = match id {
                    Some(id) => diesel::update(notification_rules::table.find(id)).set((vals, notification_rules::updated_at.eq(Utc::now()))).returning(RULE_COLS).get_result(c).map_err(dup)?,
                    None => diesel::insert_into(notification_rules::table).values(vals).returning(RULE_COLS).get_result(c).map_err(dup)?,
                };
                audit::record(c, Event::new(&user.username, if id.is_some() { "notification.rule_updated" } else { "notification.rule_created" }).target("rule", &r.0.to_string()).details(json!({ "name": r.1 })).ip(&user.ip))?;
                Ok(r)
            })
        })
        .await?;
    Ok(if id.is_some() { HttpResponse::Ok() } else { HttpResponse::Created() }.json(rule_json(row)))
}

pub async fn create_rule(state: web::Data<AppState>, user: AuthUser, body: web::Json<RuleBody>) -> ApiResult<HttpResponse> {
    save_rule(state, user, None, body).await
}
pub async fn update_rule(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<RuleBody>) -> ApiResult<HttpResponse> {
    save_rule(state, user, Some(path), body).await
}

pub async fn delete_rule(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            c.transaction(|c| {
                if diesel::delete(notification_rules::table.find(id)).execute(c)? == 0 {
                    return Err(ApiError::NotFound("rule"));
                }
                audit::record(c, Event::new(&user.username, "notification.rule_deleted").target("rule", &id.to_string()).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

/// Send a digest rule's queued events now instead of waiting for its hour.
pub async fn send_digest_now(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    let name: String = state.db(move |c| notification_rules::table.find(id).select(notification_rules::name).first(c).map_err(|_| ApiError::NotFound("rule"))).await?;
    let sent = notify::send_digest(&state, id, &name).await.map_err(|e| ApiError::bad(format!("{e:#}")))?;
    Ok(HttpResponse::Ok().json(json!({ "sent": sent })))
}

/// Recent deliveries, newest first.
pub async fn history(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    type Row = (Uuid, Option<Uuid>, Uuid, String, String, String, String, i32, String, DateTime<Utc>, Option<DateTime<Utc>>);
    let rows: Vec<Row> = state
        .db(|c| {
            Ok(notifications::table
                .order(notifications::created_at.desc())
                .limit(100)
                .select((notifications::id, notifications::rule_id, notifications::channel_id, notifications::event, notifications::severity, notifications::title, notifications::status, notifications::attempts, notifications::last_error, notifications::created_at, notifications::sent_at))
                .load(c)?)
        })
        .await?;
    Ok(HttpResponse::Ok().json(
        rows.into_iter()
            .map(|r| json!({ "id": r.0, "rule_id": r.1, "channel_id": r.2, "event": r.3, "severity": r.4, "title": r.5, "status": r.6, "attempts": r.7, "last_error": r.8, "created_at": r.9, "sent_at": r.10 }))
            .collect::<Vec<_>>(),
    ))
}
