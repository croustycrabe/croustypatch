//! Docker registries: list and replace (`GET` / `PUT /docker/registries`). Reading needs `settings:read`, changing
//! `settings:write`; passwords are never returned.
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::docker_registry::{self as reg, Incoming};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use serde::Deserialize;
use serde_json::{json, Value};

fn view(c: &mut diesel::PgConnection) -> Value {
    let regs = reg::load(c);
    let (offline, enabled) = (reg::offline(c), reg::docker_enabled(c));
    let local = reg::has_local(&regs);
    json!({
        "registries": reg::masked(&regs),
        "offline": offline, "docker_enabled": enabled, "has_local": local,
        // offline without a local registry: the Docker features are not available, and why
        "blocked": offline && !local,
        "problem": (offline && !local).then_some(reg::OFFLINE_NEEDS_LOCAL),
    })
}

/// GET /docker/registries
pub async fn list(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let out = state.db(|c| Ok(view(c))).await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize)]
pub struct PutBody {
    registries: Vec<Incoming>,
}

/// PUT /docker/registries — replaces the list. In offline mode with the Docker module on, a list without a local
/// registry is refused.
pub async fn put(state: web::Data<AppState>, user: AuthUser, body: web::Json<PutBody>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let incoming = body.into_inner().registries;
    let st = state.clone();
    let out = state
        .db(move |c| {
            let current = reg::load(c);
            let next = reg::check_set(&incoming, &current, &st.master).map_err(ApiError::bad)?;
            if reg::offline(c) && reg::docker_enabled(c) && !reg::has_local(&next) {
                return Err(ApiError::bad(format!("{} (the last local registry cannot be removed while offline with the Docker module on)", reg::OFFLINE_NEEDS_LOCAL)));
            }
            reg::save_all(c, &next)?;
            audit::record(c, Event::new(&user.username, "settings.updated").target("setting", reg::KEY).details(json!({ "registries": next.iter().map(|r| json!({ "name": r.name, "kind": r.kind, "host": r.host, "mirror_of": r.mirror_of })).collect::<Vec<_>>() })).ip(&user.ip))?;
            Ok(view(c))
        })
        .await?;
    // connected agents get the new list right away
    for h in state.hub.connected_hosts() {
        crate::modules::push_settings(&state, h).await;
    }
    Ok(HttpResponse::Ok().json(out))
}
