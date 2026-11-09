//! AI endpoints: test the configured provider, and ask for a second opinion on a pending task.
use super::admin::load_setting;
use crate::ai::{self, AiConfig};
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::models::{Host, Task};
use crate::schema::{hosts, tasks};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use diesel::prelude::*;
use serde_json::{json, Value};
use uuid::Uuid;

async fn config(state: &web::Data<AppState>) -> ApiResult<AiConfig> {
    let st = state.clone();
    let v = state.db(move |c| load_setting(c, &st, "ai")).await?;
    AiConfig::from_setting(&v).ok_or_else(|| ApiError::Conflict("AI is switched off or not configured (Settings > AI)".into()))
}

/// Round trip to the provider with a tiny fixed question; reports latency and whether the answer is usable.
pub async fn test(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let cfg = config(&state).await?;
    let t0 = std::time::Instant::now();
    let ctx = json!({ "machine": { "type": "server", "environment": "production", "exposed_to_internet": true }, "updates": { "count": 1, "security_count": 1, "highest_cvss": 9.8, "exploited_in_the_wild": true, "packages": [{ "name": "openssl", "security": true }] }, "deterministic_baseline": { "contextual_risk": "Critical" } });
    let res = ai::verdict(&cfg, &ctx).await;
    let ms = t0.elapsed().as_millis() as u64;
    Ok(HttpResponse::Ok().json(match res {
        Ok(v) => json!({ "ok": true, "model": cfg.model, "latency_ms": ms, "contextual_risk": v.contextual_risk, "rationale": v.rationale }),
        Err(e) => json!({ "ok": false, "model": cfg.model, "latency_ms": ms, "error": format!("{e:#}") }),
    }))
}

/// Ask the AI for its view of a patch task (typically one awaiting approval). Advisory only: nothing is decided.
pub async fn review_task(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("tasks:read")?;
    let id = path.into_inner();
    let cfg = config(&state).await?;
    let (task, host): (Task, Host) = state
        .db(move |c| {
            let t: Task = tasks::table.find(id).select(Task::as_select()).first(c).map_err(|_| ApiError::NotFound("task"))?;
            let h: Host = hosts::table.find(t.host_id).select(Host::as_select()).first(c)?;
            Ok((t, h))
        })
        .await?;
    if task.kind != "patch" {
        return Err(ApiError::bad("only patch tasks can be reviewed"));
    }
    let baseline = task.decision.as_ref().and_then(|d| d.get("assessment")).cloned().unwrap_or(Value::Null);
    let ctx = json!({
        "machine": { "os": host.distro, "type": host.asset_type, "environment": host.environment, "criticality": host.criticality, "exposed_to_internet": host.internet_exposed, "has_staging_copy": host.has_staging, "tags": host.tags },
        "updates": { "packages": task.packages },
        "deterministic_baseline": { "contextual_risk": baseline.get("contextual_risk"), "stability_risk": baseline.get("stability_risk"), "matched_rule": baseline.get("matched_rule") },
    });
    let v = ai::verdict(&cfg, &ctx).await.map_err(|e| ApiError::bad(format!("AI analysis failed: {e:#}")))?;
    let review = json!({ "model": cfg.model, "contextual_risk": v.contextual_risk, "rationale": v.rationale, "confidence": v.confidence, "by": user.username, "at": chrono::Utc::now() });
    let r = review.clone();
    state
        .db(move |c| {
            c.transaction(|c| {
                let mut d = tasks::table.find(id).select(tasks::decision).first::<Option<Value>>(c)?.unwrap_or_else(|| json!({}));
                d["ai_review"] = r.clone();
                diesel::update(tasks::table.find(id)).set(tasks::decision.eq(Some(d))).execute(c)?;
                audit::record(c, Event::new(&user.username, "task.ai_reviewed").target("task", &id.to_string()).host(task.host_id).details(json!({ "model": r["model"], "contextual_risk": r["contextual_risk"] })).ip(&user.ip))
            })
        })
        .await?;
    Ok(HttpResponse::Ok().json(review))
}
