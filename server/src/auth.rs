//! Authentication (local accounts, Argon2id, JWT session cookie) and RBAC.
use crate::audit::{self, Event};
use crate::config::Config;
use crate::crypto::{hash_password, verify_password};
use crate::error::{ApiError, ApiResult};
use crate::schema::{api_tokens, dashboard_layouts, roles, users};
use crate::state::AppState;
use actix_web::cookie::{time::Duration as CookieDuration, Cookie, SameSite};
use actix_web::{dev::Payload, web, FromRequest, HttpRequest, HttpResponse};
use chrono::{Duration, Utc};
use croustypatch_common::crypto::random_token;
use croustypatch_common::jwt::{self, Claims};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::OnceLock;
use uuid::Uuid;

pub const COOKIE: &str = "cp_session";
/// Personal API tokens start with this, so they are recognised (and found by secret scanners).
pub const API_TOKEN_PREFIX: &str = "cpat_";
const SESSION_HOURS: i64 = 8;
const MAX_FAILED: i32 = 5;
const LOCK_MINUTES: i64 = 15;
pub const MIN_PASSWORD_LEN: usize = 12;

// ----------------------------------------------------------------- permissions

/// `have` is a granted pattern ("*", "hosts:*", "*:read", "hosts:read"); `need` is "module:action".
pub fn pattern_allows(have: &str, need: &str) -> bool {
    if have == "*" || have == need {
        return true;
    }
    match (have.split_once(':'), need.split_once(':')) {
        (Some((hm, ha)), Some((nm, na))) => (hm == "*" || hm == nm) && (ha == "*" || ha == na),
        _ => false,
    }
}

pub fn perms_allow(perms: &[String], need: &str) -> bool {
    perms.iter().any(|p| pattern_allows(p, need))
}

/// (name, description, permissions)
pub struct BuiltinRole(pub &'static str, pub &'static str, pub &'static [&'static str]);

pub const BUILTIN_ROLES: &[BuiltinRole] = &[
    BuiltinRole("super_admin", "Full access, including accounts and roles", &["*"]),
    BuiltinRole(
        "admin",
        "Operates the platform; cannot manage accounts or roles",
        &["hosts:*", "groups:*", "policies:*", "tasks:*", "vulnerabilities:*", "waivers:*", "enrollment:*", "settings:*", "audit:read", "dashboard:*", "users:read", "roles:read"],
    ),
    BuiltinRole(
        "operator",
        "Runs scans and patches, approves tasks",
        &["hosts:read", "hosts:write", "groups:read", "policies:read", "tasks:read", "tasks:create", "tasks:write", "vulnerabilities:read", "waivers:read", "waivers:create", "audit:read", "dashboard:*"],
    ),
    BuiltinRole("auditor", "Read-only access to everything", &["*:read", "dashboard:*"]),
    BuiltinRole("ciso", "Security officer: read-only plus waiver approval", &["*:read", "waivers:create", "waivers:approve_ciso", "dashboard:*"]),
    BuiltinRole("business_owner", "Business approver for waivers", &["hosts:read", "groups:read", "tasks:read", "waivers:read", "waivers:create", "waivers:approve_business", "dashboard:*"]),
];

// --------------------------------------------------------------------- bootstrap

/// Create the built-in roles (idempotent; their permissions are refreshed on every start).
pub fn ensure_roles(conn: &mut PgConnection) -> ApiResult<()> {
    for BuiltinRole(name, desc, perms) in BUILTIN_ROLES {
        diesel::insert_into(roles::table)
            .values((roles::name.eq(*name), roles::description.eq(*desc), roles::permissions.eq(json!(perms)), roles::builtin.eq(true)))
            .on_conflict(roles::name)
            .do_update()
            .set((roles::description.eq(*desc), roles::permissions.eq(json!(perms)), roles::builtin.eq(true)))
            .execute(conn)?;
    }
    Ok(())
}

/// On a brand-new install create the fallback super-admin. Returns the temporary password.
/// `Bootstrap::password` is only set when we generated it (a preset password is never echoed).
pub struct Bootstrap {
    pub password: Option<String>,
}

pub fn ensure_fallback_admin(conn: &mut PgConnection, preset: Option<String>) -> ApiResult<Option<Bootstrap>> {
    let count: i64 = users::table.count().get_result(conn)?;
    if count > 0 {
        return Ok(None);
    }
    let role: Uuid = roles::table.filter(roles::name.eq("super_admin")).select(roles::id).first(conn)?;
    let preset = preset.filter(|p| p.len() >= MIN_PASSWORD_LEN);
    let generated = preset.is_none();
    let password = preset.unwrap_or_else(|| random_token(15));
    diesel::insert_into(users::table)
        .values((
            users::username.eq("admin"),
            users::display_name.eq("Fallback super administrator"),
            users::password_hash.eq(hash_password(&password)),
            users::role_id.eq(role),
            users::is_fallback_admin.eq(true),
            users::must_change_password.eq(true),
        ))
        .execute(conn)?;
    audit::record(conn, Event::new("system", "bootstrap.fallback_admin_created").target("user", "admin"))?;
    Ok(Some(Bootstrap { password: generated.then_some(password) }))
}

/// `croustypatch-server admin disable|enable --super-admin-fallback`.
pub fn set_fallback_disabled(conn: &mut PgConnection, disabled: bool) -> ApiResult<usize> {
    if disabled {
        // Refuse to lock everyone out: another enabled account must be able to manage users.
        let others: Vec<Value> = users::table
            .inner_join(roles::table)
            .filter(users::disabled.eq(false))
            .filter(users::is_fallback_admin.eq(false))
            .select(roles::permissions)
            .load(conn)?;
        let ok = others.iter().any(|p| perms_allow(&json_strings(p), "users:write"));
        if !ok {
            return Err(ApiError::bad("no other enabled account can manage users; create one first (or configure SSO)"));
        }
    }
    let n = diesel::update(users::table.filter(users::is_fallback_admin.eq(true))).set(users::disabled.eq(disabled)).execute(conn)?;
    audit::record(conn, Event::new("cli", if disabled { "admin.fallback_disabled" } else { "admin.fallback_enabled" }))?;
    Ok(n)
}

pub fn reset_password(conn: &mut PgConnection, username: &str, new_password: &str) -> ApiResult<()> {
    check_password_policy(new_password)?;
    let n = diesel::update(users::table.filter(users::username.eq(username)).filter(users::auth_source.eq("local")))
        .set((users::password_hash.eq(hash_password(new_password)), users::must_change_password.eq(true), users::failed_logins.eq(0), users::locked_until.eq(None::<chrono::DateTime<Utc>>)))
        .execute(conn)?;
    if n == 0 {
        return Err(ApiError::NotFound("user"));
    }
    audit::record(conn, Event::new("cli", "admin.password_reset").target("user", username))?;
    Ok(())
}

pub fn check_password_policy(p: &str) -> ApiResult<()> {
    if p.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::bad(format!("password must be at least {MIN_PASSWORD_LEN} characters")));
    }
    Ok(())
}

pub fn json_strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------- request info

/// Peer IP, or the left-most X-Forwarded-For entry when the peer is a trusted proxy.
pub fn client_ip(req: &HttpRequest, cfg: &Config) -> String {
    let peer = req.peer_addr().map(|a| a.ip());
    if let Some(p) = peer {
        if is_trusted_proxy(p, cfg) {
            if let Some(xff) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
                if let Some(first) = xff.split(',').next().map(str::trim).filter(|s| s.parse::<IpAddr>().is_ok()) {
                    return first.to_string();
                }
            }
        }
        return p.to_string();
    }
    String::new()
}

fn is_trusted_proxy(ip: IpAddr, cfg: &Config) -> bool {
    cfg.trusted_proxies.iter().any(|t| t.parse::<IpAddr>().map(|x| x == ip).unwrap_or(false))
}

/// Did the *client* reach us over HTTPS (directly, or via a trusted proxy)?
pub fn is_https(req: &HttpRequest, state: &AppState) -> bool {
    if state.tls_direct {
        return true;
    }
    let trusted = req.peer_addr().map(|a| is_trusted_proxy(a.ip(), &state.cfg)).unwrap_or(false);
    trusted && req.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Public host as seen by the browser: X-Forwarded-Host from a trusted proxy, else `Host`, else the
/// request authority. HTTP/2 has no `Host` header (the browser sends `:authority`), so the last
/// fallback is what keeps CSRF checks working for every real browser over TLS.
pub fn public_host(req: &HttpRequest, cfg: &Config) -> String {
    let trusted = req.peer_addr().map(|a| is_trusted_proxy(a.ip(), cfg)).unwrap_or(false);
    let forwarded = if trusted { req.headers().get("x-forwarded-host").and_then(|v| v.to_str().ok()) } else { None };
    let host = forwarded
        .or_else(|| req.headers().get("host").and_then(|v| v.to_str().ok()))
        .or_else(|| req.uri().authority().map(|a| a.as_str()))
        .unwrap_or("");
    host.split(',').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// Cookie-authenticated unsafe requests must come from our own origin.
pub fn csrf_ok(req: &HttpRequest, cfg: &Config) -> bool {
    if let Some(origin) = req.headers().get("origin").and_then(|v| v.to_str().ok()) {
        let host = origin.split("://").nth(1).unwrap_or("").to_ascii_lowercase();
        return host == public_host(req, cfg) || cfg.cors_origin.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(origin));
    }
    matches!(req.headers().get("sec-fetch-site").and_then(|v| v.to_str().ok()), None | Some("same-origin") | Some("none"))
}

// ---------------------------------------------------------------------- extractor

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: String,
    pub permissions: Vec<String>,
    pub must_change_password: bool,
    pub language: String,
    pub ip: String,
}

impl AuthUser {
    pub fn can(&self, perm: &str) -> bool {
        perms_allow(&self.permissions, perm)
    }
    pub fn require(&self, perm: &str) -> ApiResult<()> {
        if self.can(perm) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!("missing permission {perm}")))
        }
    }
}

type Row = (Uuid, String, bool, bool, String, String, Value);

impl FromRequest for AuthUser {
    type Error = ApiError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, ApiError>>>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            let state = req.app_data::<web::Data<AppState>>().ok_or_else(|| ApiError::internal("missing state"))?.clone();
            let bearer = req.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::to_owned);
            let from_cookie = bearer.is_none();
            let token = bearer.or_else(|| req.cookie(COOKIE).map(|c| c.value().to_owned())).ok_or(ApiError::Unauthorized)?;
            if from_cookie && !matches!(req.method().as_str(), "GET" | "HEAD" | "OPTIONS") && !csrf_ok(&req, &state.cfg) {
                return Err(ApiError::forbidden("cross-origin request refused"));
            }
            let uid: Uuid = if let Some(api) = token.strip_prefix(API_TOKEN_PREFIX).filter(|_| !from_cookie) {
                // Personal API token: acts as its user. Checked by hash; expired and revoked tokens fail.
                let hash = croustypatch_common::crypto::sha256_hex(format!("{API_TOKEN_PREFIX}{api}").as_bytes());
                let now = Utc::now();
                state
                    .db(move |c| {
                        let found: Option<(Uuid, Uuid)> = api_tokens::table
                            .filter(api_tokens::token_hash.eq(&hash))
                            .filter(api_tokens::revoked.eq(false))
                            .filter(api_tokens::expires_at.is_null().or(api_tokens::expires_at.gt(now)))
                            .select((api_tokens::id, api_tokens::user_id))
                            .first(c)
                            .optional()?;
                        let (tid, uid) = found.ok_or(ApiError::Unauthorized)?;
                        diesel::update(api_tokens::table.find(tid)).set(api_tokens::last_used_at.eq(Some(now))).execute(c)?;
                        Ok(uid)
                    })
                    .await?
            } else {
                let claims = jwt::verify_claims_hs256(&state.session_secret, &token, Utc::now().timestamp(), 5).map_err(|_| ApiError::Unauthorized)?;
                if claims.iss != "ui" {
                    return Err(ApiError::Unauthorized);
                }
                claims.sub.parse().map_err(|_| ApiError::Unauthorized)?
            };
            let row: Row = state
                .db(move |c| {
                    users::table
                        .inner_join(roles::table)
                        .filter(users::id.eq(uid))
                        .filter(users::disabled.eq(false))
                        .select((users::id, users::username, users::disabled, users::must_change_password, users::language, roles::name, roles::permissions))
                        .first(c)
                        .optional()?
                        .ok_or(ApiError::Unauthorized)
                })
                .await?;
            let user = AuthUser {
                id: row.0,
                username: row.1,
                must_change_password: row.3,
                language: row.4,
                role: row.5,
                permissions: json_strings(&row.6),
                ip: client_ip(&req, &state.cfg),
            };
            let exempt = matches!(req.path(), "/api/v1/auth/me" | "/api/v1/auth/password" | "/api/v1/auth/logout");
            if user.must_change_password && !exempt {
                return Err(ApiError::forbidden("password change required"));
            }
            Ok(user)
        })
    }
}

// ------------------------------------------------------------------------ handlers

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
}

fn dummy_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| hash_password("dummy-password-for-timing"))
}

fn session_cookie<'a>(value: String, secure: bool, max_age_secs: i64) -> Cookie<'a> {
    Cookie::build(COOKIE, value).path("/").http_only(true).secure(secure).same_site(SameSite::Strict).max_age(CookieDuration::seconds(max_age_secs)).finish()
}

pub async fn login(req: HttpRequest, state: web::Data<AppState>, body: web::Json<LoginBody>) -> ApiResult<HttpResponse> {
    let ip = client_ip(&req, &state.cfg);
    let secure = is_https(&req, &state);
    let username = body.username.trim().to_string();
    let password = body.password.clone();
    let st = state.clone();
    let ip2 = ip.clone();

    let outcome = state
        .db(move |c| {
            type L = (Uuid, Option<String>, bool, i32, Option<chrono::DateTime<Utc>>, String, bool, String, Value);
            let found: Option<L> = users::table
                .inner_join(roles::table)
                .filter(users::username.eq(&username))
                .filter(users::auth_source.eq("local"))
                .select((users::id, users::password_hash, users::disabled, users::failed_logins, users::locked_until, users::language, users::must_change_password, roles::name, roles::permissions))
                .first(c)
                .optional()?;
            let now = Utc::now();
            let Some((id, hash, disabled, fails, locked, lang, must_change, role, perms)) = found else {
                let _ = verify_password(&password, dummy_hash()); // equalise timing
                audit::record(c, Event::new(&username, "auth.login_failed").details(json!({"reason": "unknown_user"})).ip(&ip2))?;
                return Err(ApiError::Unauthorized);
            };
            if locked.is_some_and(|l| l > now) {
                return Err(ApiError::TooManyRequests);
            }
            let valid = verify_password(&password, hash.as_deref().unwrap_or(dummy_hash())) && hash.is_some() && !disabled;
            if !valid {
                let fails = fails + 1;
                let lock = (fails >= MAX_FAILED).then(|| now + Duration::minutes(LOCK_MINUTES));
                diesel::update(users::table.find(id)).set((users::failed_logins.eq(if lock.is_some() { 0 } else { fails }), users::locked_until.eq(lock))).execute(c)?;
                audit::record(c, Event::new(&username, "auth.login_failed").target("user", &id.to_string()).details(json!({"locked": lock.is_some()})).ip(&ip2))?;
                return Err(ApiError::Unauthorized);
            }
            diesel::update(users::table.find(id)).set((users::failed_logins.eq(0), users::locked_until.eq(None::<chrono::DateTime<Utc>>), users::last_login_at.eq(now))).execute(c)?;
            audit::record(c, Event::new(&username, "auth.login").target("user", &id.to_string()).ip(&ip2))?;
            Ok((id, username, lang, must_change, role, perms))
        })
        .await?;

    let (id, username, lang, must_change, role, perms) = outcome;
    let now = Utc::now().timestamp();
    let claims = Claims { sub: id.to_string(), iss: "ui".into(), iat: now, exp: now + SESSION_HOURS * 3600, jti: Uuid::new_v4().to_string(), scope: None };
    let token = jwt::sign_hs256(&st.session_secret, &claims).map_err(ApiError::internal)?;
    Ok(HttpResponse::Ok().cookie(session_cookie(token, secure, SESSION_HOURS * 3600)).json(json!({
        "id": id, "username": username, "role": role, "permissions": json_strings(&perms),
        "language": lang, "must_change_password": must_change
    })))
}

pub async fn logout(req: HttpRequest, state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    let u = user.clone();
    state.db(move |c| audit::record(c, Event::new(&u.username, "auth.logout").ip(&u.ip))).await?;
    Ok(HttpResponse::NoContent().cookie(session_cookie(String::new(), is_https(&req, &state), 0)).finish())
}

pub async fn me(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    let id = user.id;
    let email: Option<String> = state.db(move |c| Ok(users::table.find(id).select(users::email).first::<Option<String>>(c).optional()?.flatten())).await?;
    Ok(HttpResponse::Ok().json(json!({
        "id": user.id, "username": user.username, "role": user.role, "permissions": user.permissions,
        "language": user.language, "must_change_password": user.must_change_password, "email": email
    })))
}

#[derive(Deserialize)]
pub struct ProfileBody {
    /// Empty clears the address.
    email: String,
}

/// PATCH /auth/profile — a user sets their own e-mail address (used by role-based notifications and as the default
/// target of test messages).
pub async fn update_profile(state: web::Data<AppState>, user: AuthUser, body: web::Json<ProfileBody>) -> ApiResult<HttpResponse> {
    let email = body.into_inner().email.trim().to_string();
    if !email.is_empty() && !crate::api::email_ok(&email) {
        return Err(ApiError::bad("not an e-mail address"));
    }
    let value: Option<String> = (!email.is_empty()).then_some(email);
    let (id, name, ip) = (user.id, user.username.clone(), user.ip.clone());
    state
        .db(move |c| {
            c.transaction(|c| {
                diesel::update(users::table.find(id)).set(users::email.eq(&value)).execute(c)?;
                audit::record(c, Event::new(&name, "user.profile_updated").target("user", &id.to_string()).details(json!({ "email_set": value.is_some() })).ip(&ip))
            })
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Deserialize)]
pub struct PasswordBody {
    current_password: String,
    new_password: String,
}

pub async fn change_password(state: web::Data<AppState>, user: AuthUser, body: web::Json<PasswordBody>) -> ApiResult<HttpResponse> {
    check_password_policy(&body.new_password)?;
    if body.new_password == body.current_password {
        return Err(ApiError::bad("the new password must differ from the current one"));
    }
    let b = body.into_inner();
    state
        .db(move |c| {
            let hash: Option<String> = users::table.find(user.id).select(users::password_hash).first(c)?;
            if !hash.as_deref().is_some_and(|h| verify_password(&b.current_password, h)) {
                return Err(ApiError::forbidden("current password is incorrect"));
            }
            diesel::update(users::table.find(user.id)).set((users::password_hash.eq(hash_password(&b.new_password)), users::must_change_password.eq(false))).execute(c)?;
            audit::record(c, Event::new(&user.username, "auth.password_changed").target("user", &user.id.to_string()).ip(&user.ip))
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Deserialize)]
pub struct LanguageBody {
    language: String,
}

pub async fn set_language(state: web::Data<AppState>, user: AuthUser, body: web::Json<LanguageBody>) -> ApiResult<HttpResponse> {
    if !matches!(body.language.as_str(), "fr" | "en") {
        return Err(ApiError::bad("language must be 'fr' or 'en'"));
    }
    let lang = body.language.clone();
    state.db(move |c| Ok(diesel::update(users::table.find(user.id)).set(users::language.eq(lang)).execute(c)?)).await?;
    Ok(HttpResponse::NoContent().finish())
}

// ------------------------------------------------------------- dashboard layout

pub async fn get_layout(state: web::Data<AppState>, user: AuthUser) -> ApiResult<HttpResponse> {
    let layout: Option<Value> = state.db(move |c| Ok(dashboard_layouts::table.find(user.id).select(dashboard_layouts::layout).first(c).optional()?)).await?;
    Ok(HttpResponse::Ok().json(layout.unwrap_or_else(default_layout)))
}

pub fn default_layout() -> Value {
    json!({ "widgets": [
        { "id": "fleet", "visible": true, "size": "md" },
        { "id": "os", "visible": true, "size": "md" },
        { "id": "packages", "visible": true, "size": "md" },
        { "id": "vulns", "visible": true, "size": "md" },
        { "id": "activity", "visible": true, "size": "lg" },
        { "id": "portal", "visible": true, "size": "md" }
    ]})
}

pub async fn put_layout(state: web::Data<AppState>, user: AuthUser, body: web::Json<Value>) -> ApiResult<HttpResponse> {
    let layout = body.into_inner();
    let widgets = layout.get("widgets").and_then(|w| w.as_array()).ok_or_else(|| ApiError::bad("layout.widgets must be an array"))?;
    let valid = widgets.len() <= 20
        && widgets.iter().all(|w| {
            w.get("id").and_then(|i| i.as_str()).is_some_and(|i| i.len() <= 40)
                && w.get("visible").is_some_and(|v| v.is_boolean())
                && w.get("size").and_then(|s| s.as_str()).is_some_and(|s| matches!(s, "sm" | "md" | "lg"))
        });
    if !valid {
        return Err(ApiError::bad("invalid widget definition"));
    }
    state
        .db(move |c| {
            diesel::insert_into(dashboard_layouts::table)
                .values((dashboard_layouts::user_id.eq(user.id), dashboard_layouts::layout.eq(&layout)))
                .on_conflict(dashboard_layouts::user_id)
                .do_update()
                .set((dashboard_layouts::layout.eq(&layout), dashboard_layouts::updated_at.eq(Utc::now())))
                .execute(c)?;
            Ok(())
        })
        .await?;
    Ok(HttpResponse::NoContent().finish())
}
