//! Reports API: scheduled summaries of the fleet (see `crate::reports`).
use super::{email_ok, name_ok};
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::reports::{self, Report, Sections, FREQUENCIES};
use crate::schema::{notification_channels, report_runs, reports as reports_t};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::{DateTime, NaiveTime, Utc};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ReportBody {
    name: String,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default = "weekly")]
    frequency: String,
    #[serde(default)]
    weekday: i32,
    #[serde(default = "one")]
    day_of_month: i32,
    #[serde(default = "eight")]
    send_at: String,
    #[serde(default = "utc")]
    timezone: String,
    #[serde(default = "fr")]
    language: String,
    #[serde(default)]
    sections: Value,
    #[serde(default)]
    group_ids: Vec<Uuid>,
    #[serde(default)]
    emails: Vec<String>,
    #[serde(default)]
    channel_ids: Vec<Uuid>,
    #[serde(default)]
    skip_if_empty: bool,
}
fn yes() -> bool {
    true
}
fn weekly() -> String {
    "weekly".into()
}
fn one() -> i32 {
    1
}
fn eight() -> String {
    "08:00".into()
}
fn utc() -> String {
    "UTC".into()
}
fn fr() -> String {
    "fr".into()
}

fn check(b: &ReportBody) -> ApiResult<Sections> {
    name_ok("name", &b.name)?;
    super::one_of("frequency", &b.frequency, FREQUENCIES)?;
    super::one_of("language", &b.language, &["fr", "en"])?;
    if !(0..=6).contains(&b.weekday) {
        return Err(ApiError::bad("weekday must be 0 (Monday) to 6 (Sunday)"));
    }
    if !(1..=28).contains(&b.day_of_month) {
        return Err(ApiError::bad("day_of_month must be 1 to 28"));
    }
    NaiveTime::parse_from_str(&b.send_at, "%H:%M").map_err(|_| ApiError::bad("send_at must be HH:MM"))?;
    b.timezone.trim().parse::<chrono_tz::Tz>().map_err(|_| ApiError::bad("unknown timezone (e.g. Europe/Paris)"))?;
    if b.emails.len() > 50 {
        return Err(ApiError::bad("at most 50 e-mail addresses"));
    }
    if let Some(bad) = b.emails.iter().find(|e| !email_ok(e)) {
        return Err(ApiError::bad(format!("not an e-mail address: {bad}")));
    }
    if b.channel_ids.len() > 20 || b.group_ids.len() > 100 {
        return Err(ApiError::bad("too many channels or groups"));
    }
    let s: Sections = if b.sections.is_null() || b.sections.as_object().is_some_and(|o| o.is_empty()) { Sections::default() } else { serde_json::from_value(b.sections.clone()).map_err(|e| ApiError::bad(format!("invalid sections: {e}")))? };
    s.validate().map_err(ApiError::BadRequest)?;
    Ok(s)
}

fn from_body(b: &ReportBody, s: &Sections) -> Report {
    let now = Utc::now();
    Report {
        id: Uuid::nil(),
        name: b.name.trim().to_string(),
        enabled: b.enabled,
        frequency: b.frequency.clone(),
        weekday: b.weekday,
        day_of_month: b.day_of_month,
        send_at: b.send_at.clone(),
        timezone: b.timezone.trim().to_string(),
        language: b.language.clone(),
        sections: serde_json::to_value(s).unwrap_or_default(),
        group_ids: json!(b.group_ids),
        emails: json!(b.emails.iter().map(|e| e.trim()).collect::<Vec<_>>()),
        channel_ids: json!(b.channel_ids),
        skip_if_empty: b.skip_if_empty,
        last_run_at: None,
        last_slot_at: None,
        attempts: 0,
        last_status: String::new(),
        last_error: String::new(),
        created_at: now,
        updated_at: now,
    }
}

fn out(r: &Report) -> Value {
    let mut v = serde_json::to_value(r).unwrap_or_default();
    v["next_run"] = json!(reports::next_slot(r, Utc::now()));
    v
}

pub async fn list(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let rows: Vec<Report> = state.db(|c| Ok(reports_t::table.order(reports_t::name.asc()).select(Report::as_select()).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows.iter().map(out).collect::<Vec<_>>()))
}

async fn save(state: web::Data<AppState>, user: AuthUser, id: Option<Uuid>, b: ReportBody) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let s = check(&b)?;
    let r = from_body(&b, &s);
    let channels = b.channel_ids.clone();
    let row: Report = state
        .db(move |c| {
            c.transaction(|c| {
                if !channels.is_empty() {
                    let found: i64 = notification_channels::table.filter(notification_channels::id.eq_any(&channels)).count().get_result(c)?;
                    if found != channels.len() as i64 {
                        return Err(ApiError::bad("one of the channels does not exist"));
                    }
                }
                let vals = (
                    reports_t::name.eq(&r.name),
                    reports_t::enabled.eq(r.enabled),
                    reports_t::frequency.eq(&r.frequency),
                    reports_t::weekday.eq(r.weekday),
                    reports_t::day_of_month.eq(r.day_of_month),
                    reports_t::send_at.eq(&r.send_at),
                    reports_t::timezone.eq(&r.timezone),
                    reports_t::language.eq(&r.language),
                    reports_t::sections.eq(&r.sections),
                    reports_t::group_ids.eq(&r.group_ids),
                    reports_t::emails.eq(&r.emails),
                    reports_t::channel_ids.eq(&r.channel_ids),
                    reports_t::skip_if_empty.eq(r.skip_if_empty),
                );
                let map = |e: diesel::result::Error| match e {
                    diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => ApiError::Conflict("a report with this name exists".into()),
                    diesel::result::Error::NotFound => ApiError::NotFound("report"),
                    e => e.into(),
                };
                let row: Report = match id {
                    // Changing the schedule starts it afresh: the moment already handled is forgotten.
                    Some(id) => diesel::update(reports_t::table.find(id)).set((vals, reports_t::updated_at.eq(Utc::now()), reports_t::attempts.eq(0))).returning(Report::as_returning()).get_result(c).map_err(map)?,
                    None => diesel::insert_into(reports_t::table).values(vals).returning(Report::as_returning()).get_result(c).map_err(map)?,
                };
                audit::record(c, Event::new(&user.username, if id.is_some() { "report.updated" } else { "report.created" }).target("report", &row.id.to_string()).details(json!({ "name": row.name, "frequency": row.frequency, "recipients": row.emails })).ip(&user.ip))?;
                Ok(row)
            })
        })
        .await?;
    Ok(if id.is_some() { HttpResponse::Ok() } else { HttpResponse::Created() }.json(out(&row)))
}

pub async fn create(state: web::Data<AppState>, user: AuthUser, body: web::Json<ReportBody>) -> ApiResult<HttpResponse> {
    save(state, user, None, body.into_inner()).await
}
pub async fn update(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<ReportBody>) -> ApiResult<HttpResponse> {
    save(state, user, Some(path.into_inner()), body.into_inner()).await
}

pub async fn delete(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            c.transaction(|c| {
                if diesel::delete(reports_t::table.find(id)).execute(c)? == 0 {
                    return Err(ApiError::NotFound("report"));
                }
                audit::record(c, Event::new(&user.username, "report.deleted").target("report", &id.to_string()).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

/// POST /reports/preview — what a report being edited would contain right now (nothing is sent or saved).
pub async fn preview(state: web::Data<AppState>, user: AuthUser, body: web::Json<ReportBody>) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let b = body.into_inner();
    let s = check(&b)?;
    let r = from_body(&b, &s);
    let built = state.db(move |c| reports::build(c, &r, Utc::now())).await?;
    Ok(HttpResponse::Ok().json(json!({ "title": built.title, "text": built.text, "html": built.html, "empty": built.empty })))
}

#[derive(Deserialize, Default)]
pub struct SendBody {
    /// Send to these addresses instead of the report's own (a test to yourself).
    #[serde(default)]
    to: Vec<String>,
}

/// POST /reports/{id}/send — build and send now; the answer says whether it worked, and why not when it did not.
pub async fn send_now(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: Option<web::Json<SendBody>>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let id = path.into_inner();
    let to = body.map(|b| b.into_inner().to).unwrap_or_default();
    if let Some(bad) = to.iter().find(|e| !email_ok(e)) {
        return Err(ApiError::bad(format!("not an e-mail address: {bad}")));
    }
    let o = reports::send(&state, id, "manual", (!to.is_empty()).then_some(to)).await?;
    let (name, ip, st) = (user.username.clone(), user.ip.clone(), o.status);
    state.db(move |c| audit::record(c, Event::new(&name, "report.sent_manually").target("report", &id.to_string()).details(json!({ "status": st })).ip(&ip))).await?;
    Ok(HttpResponse::Ok().json(json!({ "status": o.status, "error": o.error, "recipients": o.recipients, "title": o.title })))
}

pub async fn runs(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let id = path.into_inner();
    let rows: Vec<(DateTime<Utc>, String, String, String, Value, String)> = state
        .db(move |c| Ok(report_runs::table.filter(report_runs::report_id.eq(id)).order(report_runs::at.desc()).limit(20).select((report_runs::at, report_runs::trigger, report_runs::status, report_runs::error, report_runs::recipients, report_runs::title)).load(c)?))
        .await?;
    Ok(HttpResponse::Ok().json(rows.into_iter().map(|(at, trigger, status, error, recipients, title)| json!({ "at": at, "trigger": trigger, "status": status, "error": error, "recipients": recipients, "title": title })).collect::<Vec<_>>()))
}
