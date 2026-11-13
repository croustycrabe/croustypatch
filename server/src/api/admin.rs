use super::name_ok;
use crate::audit::{self, Event};
use crate::auth::{check_password_policy, json_strings, AuthUser};
use crate::crypto::hash_password;
use crate::error::{ApiError, ApiResult};
use crate::models::{Role, User};
use crate::schema::{roles, settings, users};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::Utc;
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

// ------------------------------------------------------------------------ users

pub async fn list_users(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("users:read")?;
    let rows: Vec<User> = state.db(|c| Ok(users::table.order(users::username.asc()).select(User::as_select()).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows))
}

#[derive(Deserialize)]
pub struct NewUser {
    username: String,
    #[serde(default)]
    display_name: String,
    email: Option<String>,
    password: String,
    role_id: Uuid,
}

pub async fn create_user(state: web::Data<AppState>, user: AuthUser, body: web::Json<NewUser>) -> ApiResult<HttpResponse> {
    user.require("users:create")?;
    let b = body.into_inner();
    name_ok("username", &b.username)?;
    if b.username.contains(char::is_whitespace) {
        return Err(ApiError::bad("username cannot contain spaces"));
    }
    check_password_policy(&b.password)?;
    let created: User = state
        .db(move |c| {
            ensure_can_assign_role(c, &user, b.role_id)?;
            let u: User = diesel::insert_into(users::table)
                .values((
                    users::username.eq(b.username.trim()),
                    users::display_name.eq(&b.display_name),
                    users::email.eq(&b.email),
                    users::password_hash.eq(hash_password(&b.password)),
                    users::role_id.eq(b.role_id),
                    users::must_change_password.eq(true),
                ))
                .returning(User::as_returning())
                .get_result(c)?;
            audit::record(c, Event::new(&user.username, "user.created").target("user", &u.id.to_string()).details(json!({"username": u.username, "role_id": u.role_id})).ip(&user.ip))?;
            Ok(u)
        })
        .await?;
    Ok(HttpResponse::Created().json(created))
}

#[derive(Deserialize)]
pub struct UserPatch {
    display_name: Option<String>,
    email: Option<String>,
    role_id: Option<Uuid>,
    disabled: Option<bool>,
    password: Option<String>,
}

pub async fn update_user(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<UserPatch>) -> ApiResult<HttpResponse> {
    user.require("users:write")?;
    let id = path.into_inner();
    let b = body.into_inner();
    if let Some(p) = &b.password {
        check_password_policy(p)?;
    }
    if id == user.id && (b.disabled == Some(true) || b.role_id.is_some()) {
        return Err(ApiError::bad("you cannot disable yourself or change your own role"));
    }
    let updated: User = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                if let Some(r) = b.role_id {
                    ensure_can_assign_role(c, &user, r)?;
                }
                if let Some(v) = &b.display_name {
                    diesel::update(users::table.find(id)).set(users::display_name.eq(v)).execute(c)?;
                }
                if let Some(v) = &b.email {
                    diesel::update(users::table.find(id)).set(users::email.eq(v)).execute(c)?;
                }
                if let Some(v) = b.role_id {
                    diesel::update(users::table.find(id)).set(users::role_id.eq(v)).execute(c)?;
                }
                if let Some(v) = b.disabled {
                    diesel::update(users::table.find(id)).set(users::disabled.eq(v)).execute(c)?;
                }
                if let Some(p) = &b.password {
                    diesel::update(users::table.find(id)).set((users::password_hash.eq(hash_password(p)), users::must_change_password.eq(true), users::failed_logins.eq(0), users::locked_until.eq(None::<chrono::DateTime<Utc>>))).execute(c)?;
                }
                let u: User = users::table.find(id).select(User::as_select()).first(c).map_err(|_| ApiError::NotFound("user"))?;
                audit::record(c, Event::new(&user.username, "user.updated").target("user", &id.to_string()).details(json!({"role_id": b.role_id, "disabled": b.disabled, "password_reset": b.password.is_some()})).ip(&user.ip))?;
                Ok(u)
            })
        })
        .await?;
    Ok(HttpResponse::Ok().json(updated))
}

pub async fn delete_user(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("users:delete")?;
    let id = path.into_inner();
    if id == user.id {
        return Err(ApiError::bad("you cannot delete yourself"));
    }
    state
        .db(move |c| {
            let n = diesel::delete(users::table.find(id)).execute(c)?;
            if n == 0 {
                return Err(ApiError::NotFound("user"));
            }
            audit::record(c, Event::new(&user.username, "user.deleted").target("user", &id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

// ------------------------------------------------------------------------ roles

pub async fn list_roles(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("roles:read")?;
    let rows: Vec<Role> = state.db(|c| Ok(roles::table.order(roles::name.asc()).select(Role::as_select()).load(c)?)).await?;
    Ok(HttpResponse::Ok().json(rows))
}

#[derive(Deserialize)]
pub struct RoleBody {
    name: String,
    #[serde(default)]
    description: String,
    permissions: Vec<String>,
}

fn check_permissions(perms: &[String]) -> ApiResult<()> {
    let re = regex::Regex::new(r"^(\*|[a-z_*]+:[a-z_*]+)$").unwrap();
    if perms.len() > 200 || perms.iter().any(|p| !re.is_match(p)) {
        return Err(ApiError::bad("permissions must look like 'module:action', 'module:*', '*:action' or '*'"));
    }
    Ok(())
}

/// You cannot hand out more than you hold (a pattern is covered only by an equal or broader one).
fn ensure_can_grant(user: &AuthUser, perms: &[String]) -> ApiResult<()> {
    match perms.iter().find(|p| !user.can(p)) {
        Some(p) => Err(ApiError::forbidden(format!("you do not hold permission {p}"))),
        None => Ok(()),
    }
}

/// A user may only be given a role whose permissions the caller already holds.
fn ensure_can_assign_role(c: &mut PgConnection, user: &AuthUser, role_id: Uuid) -> ApiResult<()> {
    let perms: Value = roles::table.find(role_id).select(roles::permissions).first(c).map_err(|_| ApiError::bad("unknown role"))?;
    ensure_can_grant(user, &json_strings(&perms))
}

pub async fn create_role(state: web::Data<AppState>, user: AuthUser, body: web::Json<RoleBody>) -> ApiResult<HttpResponse> {
    user.require("roles:create")?;
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    check_permissions(&b.permissions)?;
    ensure_can_grant(&user, &b.permissions)?;
    let role: Role = state
        .db(move |c| {
            let r: Role = diesel::insert_into(roles::table)
                .values((roles::name.eq(b.name.trim()), roles::description.eq(&b.description), roles::permissions.eq(json!(b.permissions))))
                .returning(Role::as_returning())
                .get_result(c)?;
            audit::record(c, Event::new(&user.username, "role.created").target("role", &r.id.to_string()).details(json!({"name": r.name, "permissions": r.permissions})).ip(&user.ip))?;
            Ok(r)
        })
        .await?;
    Ok(HttpResponse::Created().json(role))
}

pub async fn update_role(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>, body: web::Json<RoleBody>) -> ApiResult<HttpResponse> {
    user.require("roles:write")?;
    let id = path.into_inner();
    let b = body.into_inner();
    name_ok("name", &b.name)?;
    check_permissions(&b.permissions)?;
    ensure_can_grant(&user, &b.permissions)?;
    let role: Role = state
        .db(move |c| {
            let builtin: bool = roles::table.find(id).select(roles::builtin).first(c).map_err(|_| ApiError::NotFound("role"))?;
            if builtin {
                return Err(ApiError::bad("built-in roles cannot be edited; create a custom role instead"));
            }
            let r: Role = diesel::update(roles::table.find(id)).set((roles::name.eq(b.name.trim()), roles::description.eq(&b.description), roles::permissions.eq(json!(b.permissions)))).returning(Role::as_returning()).get_result(c)?;
            audit::record(c, Event::new(&user.username, "role.updated").target("role", &id.to_string()).details(json!({"permissions": r.permissions})).ip(&user.ip))?;
            Ok(r)
        })
        .await?;
    Ok(HttpResponse::Ok().json(role))
}

pub async fn delete_role(state: web::Data<AppState>, user: AuthUser, path: web::Path<Uuid>) -> ApiResult<HttpResponse> {
    user.require("roles:delete")?;
    let id = path.into_inner();
    state
        .db(move |c| {
            let builtin: bool = roles::table.find(id).select(roles::builtin).first(c).map_err(|_| ApiError::NotFound("role"))?;
            if builtin {
                return Err(ApiError::bad("built-in roles cannot be deleted"));
            }
            diesel::delete(roles::table.find(id)).execute(c)?;
            audit::record(c, Event::new(&user.username, "role.deleted").target("role", &id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

// ---------------------------------------------------------------------- settings

/// Settings whose value is encrypted at rest and never returned in clear.
const SECRET_KEYS: &[&str] = &["ai"];
const KNOWN_KEYS: &[&str] = &["modules", "ai", "connectivity", "agent", "compliance", "isolation"];

pub fn default_setting(key: &str) -> Value {
    match key {
        "modules" => json!({ "docker": false, "helper_script": false, "openscap": false, "portal": false, "portal_updates": false, "notifications": false, "eco": true }),
        "ai" => json!({ "enabled": false, "provider": "openai_compatible", "base_url": "", "model": "", "api_key": "" }),
        "connectivity" => json!({ "mode": "connected" }),
        "agent" => json!({ "heartbeat_secs": 60 }),
        "compliance" => json!({ "threshold": crate::compliance::DEFAULT_THRESHOLD, "profile": "", "auto_install": true, "schedule": { "frequency": "off", "weekday": 6, "day_of_month": 1, "at": "03:00", "timezone": "UTC" } }),
        "isolation" => serde_json::to_value(croustypatch_common::protocol::IsolationSettings::default()).unwrap_or_default(),
        _ => json!({}),
    }
}

/// A setting that is never encrypted, as saved (None when it was never saved).
pub fn load_setting_plain(c: &mut PgConnection, key: &str) -> Option<Value> {
    settings::table.find(key).filter(settings::encrypted.eq(false)).select(settings::value).first(c).optional().ok().flatten()
}

/// Load a setting (decrypting if needed), falling back to its default.
pub fn load_setting(c: &mut PgConnection, state: &AppState, key: &str) -> ApiResult<Value> {
    let row: Option<(Value, bool)> = settings::table.find(key).select((settings::value, settings::encrypted)).first(c).optional()?;
    match row {
        None => Ok(default_setting(key)),
        // A setting saved by an older version lacks the keys added since: they read as their default, which is also
        // what the code that uses them assumes.
        Some((mut v, false)) if key == "compliance" => {
            if let (Some(o), Some(d)) = (v.as_object_mut(), default_setting(key).as_object()) {
                for (k, dv) in d {
                    o.entry(k.clone()).or_insert_with(|| dv.clone());
                }
            }
            Ok(v)
        }
        Some((v, false)) => Ok(v),
        Some((v, true)) => {
            let ct = v.get("ct").and_then(|x| x.as_str()).ok_or_else(|| ApiError::internal("malformed encrypted setting"))?;
            let plain = state.master.decrypt_string(ct).map_err(ApiError::internal)?;
            serde_json::from_str(&plain).map_err(ApiError::internal)
        }
    }
}

fn mask(key: &str, mut v: Value) -> Value {
    if key == "ai" {
        if let Some(o) = v.as_object_mut() {
            let set = o.get("api_key").and_then(|k| k.as_str()).is_some_and(|k| !k.is_empty());
            o.insert("api_key".into(), json!(""));
            o.insert("api_key_set".into(), json!(set));
        }
    }
    v
}

pub async fn modules(state: web::Data<AppState>, _user: AuthUser) -> ApiResult<HttpResponse> {
    let st = state.clone();
    let v = state.db(move |c| load_setting(c, &st, "modules")).await?;
    Ok(HttpResponse::Ok().json(v))
}

pub async fn get_settings(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    user.require("settings:read")?;
    let st = state.clone();
    let out = state
        .db(move |c| {
            let mut m = serde_json::Map::new();
            for k in KNOWN_KEYS {
                m.insert((*k).into(), mask(k, load_setting(c, &st, k)?));
            }
            Ok(Value::Object(m))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

pub async fn put_setting(state: web::Data<AppState>, user: AuthUser, path: web::Path<String>, body: web::Json<Value>) -> ApiResult<HttpResponse> {
    user.require("settings:write")?;
    let key = path.into_inner();
    if !KNOWN_KEYS.contains(&key.as_str()) {
        return Err(ApiError::bad("unknown setting"));
    }
    let mut value = body.into_inner();
    if !value.is_object() {
        return Err(ApiError::bad("setting must be a JSON object"));
    }
    validate_setting(&key, &value)?;
    // Agents carry the isolation rules themselves (a double failure isolates the host without asking the server).
    let push_to_agents = key == "isolation" || key == "connectivity";
    let st = state.clone();
    state
        .db(move |c| {
            // offline mode and the Docker module need a local registry (Settings > Docker registries)
            crate::docker_registry::check_setting_change(c, &key, &value)?;
            // An empty api_key in an update means "keep the stored one".
            if key == "ai" {
                let kept = value.get("api_key").and_then(|k| k.as_str()).is_none_or(str::is_empty);
                if kept {
                    let old = load_setting(c, &st, "ai")?;
                    value["api_key"] = old.get("api_key").cloned().unwrap_or(json!(""));
                }
                value.as_object_mut().map(|o| o.remove("api_key_set"));
            }
            let encrypted = SECRET_KEYS.contains(&key.as_str());
            let stored = if encrypted { json!({ "ct": st.master.encrypt(value.to_string().as_bytes()) }) } else { value.clone() };
            diesel::insert_into(settings::table)
                .values((settings::key.eq(&key), settings::value.eq(&stored), settings::encrypted.eq(encrypted)))
                .on_conflict(settings::key)
                .do_update()
                .set((settings::value.eq(&stored), settings::encrypted.eq(encrypted), settings::updated_at.eq(Utc::now())))
                .execute(c)?;
            // Log which keys changed, never the values (they may be secrets).
            let fields: Vec<String> = value.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            audit::record(c, Event::new(&user.username, "settings.updated").target("setting", &key).details(json!({"fields": fields})).ip(&user.ip))
        })
        .await?;
    if push_to_agents {
        for h in state.hub.connected_hosts() {
            crate::modules::push_settings(&state, h).await;
        }
    }
    Ok(HttpResponse::NoContent().finish())
}

fn validate_setting(key: &str, v: &Value) -> ApiResult<()> {
    match key {
        "modules" => {
            let o = v.as_object().unwrap();
            if o.values().any(|x| !x.is_boolean()) {
                return Err(ApiError::bad("module flags must be booleans"));
            }
        }
        "connectivity" => super::one_of("mode", v.get("mode").and_then(|m| m.as_str()).unwrap_or(""), &["connected", "air_gapped"])?,
        "compliance" => {
            let t = v.get("threshold").and_then(|h| h.as_f64()).unwrap_or(-1.0);
            if !(0.0..=100.0).contains(&t) {
                return Err(ApiError::bad("threshold must be 0..=100"));
            }
            let p = v.get("profile").and_then(|h| h.as_str()).unwrap_or("");
            if !crate::compliance::valid_profile_ref(p) {
                return Err(ApiError::bad("profile must be an XCCDF profile id or a custom profile"));
            }
            if v.get("auto_install").is_some_and(|a| !a.is_boolean()) {
                return Err(ApiError::bad("auto_install must be true or false"));
            }
            crate::compliance::validate_schedule(v.get("schedule")).map_err(ApiError::bad)?;
        }
        "isolation" => {
            let iso: croustypatch_common::protocol::IsolationSettings = serde_json::from_value(v.clone()).map_err(|_| ApiError::bad("isolation needs keep_ssh (bool), ssh_port (1-65535) and ssh_sources (list)"))?;
            if iso.ssh_port == 0 || iso.ssh_sources.len() > 50 {
                return Err(ApiError::bad("ssh_port must be 1-65535 and ssh_sources at most 50 entries"));
            }
            if let Some(bad) = iso.ssh_sources.iter().find(|s| croustypatch_common::protocol::parse_cidr(s).is_none()) {
                return Err(ApiError::bad(format!("not an address or network: {bad} (e.g. 10.0.10.0/24)")));
            }
        }
        "agent" => {
            let hb = v.get("heartbeat_secs").and_then(|h| h.as_i64()).unwrap_or(0);
            if !(10..=3600).contains(&hb) {
                return Err(ApiError::bad("heartbeat_secs must be 10..=3600"));
            }
        }
        _ => {}
    }
    Ok(())
}
