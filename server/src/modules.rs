//! Optional modules, switched on per host (docker, helper scripts, OpenSCAP, app center, eco mode).
//!
//! The `modules` setting (Settings > Modules) holds the defaults given to new hosts; the "Add agent" wizard can
//! override them per enrolment token; afterwards each host keeps its own switches (`hosts.modules`). Agents that
//! advertise `capability::SETTINGS` receive their switches, so they stop collecting data for modules that are off
//! and follow the eco switch.
use crate::error::{ApiError, ApiResult};
use crate::schema::{hosts, settings};
use crate::state::AppState;
use actix_web::web;
use croustypatch_common::protocol::{capability, AgentSettings, HostModules, ServerMessage};
use diesel::prelude::*;
use serde_json::{json, Value};
use uuid::Uuid;

/// Read a module map leniently: unknown keys are ignored, missing ones are off.
pub fn parse(v: &Value) -> HostModules {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

/// Strict check for module maps coming from the API.
pub fn validate(v: &Value) -> ApiResult<HostModules> {
    let o = v.as_object().ok_or_else(|| ApiError::bad("modules must be an object of booleans"))?;
    if let Some((k, _)) = o.iter().find(|(k, x)| !HostModules::NAMES.contains(&k.as_str()) || !x.is_boolean()) {
        return Err(ApiError::bad(format!("unknown module or non-boolean value: {k} (modules: {})", HostModules::NAMES.join(", "))));
    }
    Ok(parse(v))
}

/// Defaults for new hosts (Settings > Modules). Eco mode is on unless switched off.
pub fn defaults(c: &mut PgConnection) -> HostModules {
    let mut v = settings::table.find("modules").select(settings::value).first::<Value>(c).optional().ok().flatten().unwrap_or_else(|| json!({}));
    if v.get("eco").is_none() {
        v["eco"] = json!(true);
    }
    parse(&v)
}

pub fn for_host(c: &mut PgConnection, host_id: Uuid) -> ApiResult<HostModules> {
    let v: Value = hosts::table.find(host_id).select(hosts::modules).first(c).map_err(|_| ApiError::NotFound("host"))?;
    Ok(parse(&v))
}

pub fn has_capability(facts: &Value, cap: &str) -> bool {
    facts.get("capabilities").and_then(|c| c.as_array()).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(cap)))
}

pub fn host_has_capability(c: &mut PgConnection, host_id: Uuid, cap: &str) -> ApiResult<bool> {
    let facts: Value = hosts::table.find(host_id).select(hosts::facts).first(c).map_err(|_| ApiError::NotFound("host"))?;
    Ok(has_capability(&facts, cap))
}

/// What to send to the agent, or None if it would not understand it.
pub fn settings_for(c: &mut PgConnection, host_id: Uuid) -> ApiResult<Option<AgentSettings>> {
    let (facts, modules): (Value, Value) = hosts::table.find(host_id).select((hosts::facts, hosts::modules)).first(c).map_err(|_| ApiError::NotFound("host"))?;
    if !has_capability(&facts, capability::SETTINGS) {
        return Ok(None);
    }
    let isolation = settings::table.find("isolation").select(settings::value).first::<Value>(c).optional()?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let offline = settings::table.find("connectivity").select(settings::value).first::<Value>(c).optional()?.and_then(|v| v.get("mode").and_then(|m| m.as_str()).map(|m| m == "air_gapped")).unwrap_or(false);
    // Arch only: AUR packages of app center recipes, checked by the agent for a newer version (app updates in the web UI)
    let pm: String = hosts::table.find(host_id).select(hosts::package_manager).first(c)?;
    let watch_aur = if pm == "pacman" {
        let recipes: Vec<Value> = crate::schema::portal_apps::table.filter(crate::schema::portal_apps::enabled.eq(true)).select(crate::schema::portal_apps::recipes).load(c)?;
        let mut v: Vec<String> = recipes.iter().flat_map(aur_watched).collect();
        v.sort();
        v.dedup();
        v
    } else {
        vec![]
    };
    Ok(Some(AgentSettings { modules: parse(&modules), isolation, offline, watch_aur }))
}

/// The AUR packages an app's recipes name as where their latest version is published (`command.latest.aur`).
pub fn aur_watched(recipes: &Value) -> Vec<String> {
    recipes
        .as_object()
        .map(|o| o.values().filter_map(|r| r.pointer("/command/latest/aur").and_then(|a| a.as_str())).filter(|a| croustypatch_common::protocol::valid_aur_name(a)).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Send the host's settings over its tunnel right away (push-mode agents get them with their next telemetry).
pub async fn push_settings(state: &web::Data<AppState>, host_id: Uuid) {
    if !state.hub.connected_hosts().contains(&host_id) {
        return;
    }
    if let Ok(Some(s)) = state.db(move |c| settings_for(c, host_id)).await {
        state.hub.send(host_id, ServerMessage::Settings(s));
    }
}
