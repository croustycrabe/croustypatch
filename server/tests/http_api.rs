//! Integration: the real HTTP API (routing, extractors, sessions, CSRF, RBAC, enrolment) on a real database.
mod support;

use actix_web::cookie::Cookie;
use actix_web::http::StatusCode;
use actix_web::test::TestRequest;
use croustypatch_common::crypto;
use croustypatch_common::jwt::{self, Claims};
use croustypatch_server::auth;
use serde_json::{json, Value};

use support::http::*;

#[actix_web::test]
async fn login_cookie_flags_lockout_and_forced_password_change() {
    let db = support::TestDb::new();
    let app = app(&db).await;

    let bad = login(&app, "admin", "nope").await;
    assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);
    let unknown = login(&app, "ghost", "nope").await;
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED, "same answer for an unknown user");

    let ok = login(&app, "admin", ADMIN_PW).await;
    assert_eq!(ok.status(), StatusCode::OK);
    let cookie = session(&ok);
    assert!(cookie.http_only().unwrap_or(false));
    assert!(cookie.secure().unwrap_or(false), "tls_direct => Secure");
    assert_eq!(cookie.same_site(), Some(actix_web::cookie::SameSite::Strict));

    // the bootstrap password must be changed before anything else works
    let (st, body) = call(&app, TestRequest::get().uri("/api/v1/hosts").cookie(cookie.clone())).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{body}");
    let (st, me) = call(&app, TestRequest::get().uri("/api/v1/auth/me").cookie(cookie.clone())).await;
    assert_eq!((st, me["must_change_password"].clone()), (StatusCode::OK, json!(true)));
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/auth/password").cookie(cookie.clone()).set_json(json!({"current_password": ADMIN_PW, "new_password": "short"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/auth/password").cookie(cookie.clone()).set_json(json!({"current_password": "wrong-current-pw", "new_password": ADMIN_NEW_PW}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let good = admin(&app).await;
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/hosts").cookie(good)).await;
    assert_eq!(st, StatusCode::OK);

    // five wrong passwords lock the account, even for the right one
    for _ in 0..5 {
        login(&app, "admin", "wrong-wrong-wrong").await;
    }
    assert_eq!(login(&app, "admin", ADMIN_NEW_PW).await.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[actix_web::test]
async fn unauthenticated_and_tampered_sessions_are_refused() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/hosts")).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/hosts").cookie(Cookie::new(auth::COOKIE, "a.b.c"))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // a token signed with another secret
    let now = chrono::Utc::now().timestamp();
    let forged = jwt::sign_hs256(b"not-the-secret", &Claims { sub: uuid::Uuid::new_v4().to_string(), iss: "ui".into(), iat: now, exp: now + 3600, jti: "x".into(), scope: None }).unwrap();
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/hosts").cookie(Cookie::new(auth::COOKIE, forged))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/health")).await;
    assert_eq!(st, StatusCode::OK, "health is public");
}

/// Regression: over HTTP/2 a browser sends no `Host` header (only `:authority`). The CSRF check used to see an
/// empty host and refused every write from a real browser, while curl over HTTP/1.1 worked.
#[actix_web::test]
async fn csrf_accepts_same_origin_over_http2_style_requests_and_refuses_foreign_origins() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let cookie = admin(&app).await;
    let put = |origin: &str| {
        TestRequest::put().uri("https://patch.example.com/api/v1/auth/language").cookie(cookie.clone()).insert_header(("origin", origin.to_string())).set_json(json!({"language": "en"}))
    };
    let (st, _) = call(&app, put("https://patch.example.com")).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "same origin, no Host header");
    let (st, _) = call(&app, put("https://evil.example")).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "foreign origin");
    // a cross-site fetch without an Origin header is refused through Sec-Fetch-Site
    let (st, _) = call(&app, TestRequest::put().uri("/api/v1/auth/language").cookie(cookie.clone()).insert_header(("sec-fetch-site", "cross-site")).set_json(json!({"language": "en"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // Bearer-token API clients are not subject to browser CSRF rules
    let (st, _) = call(&app, TestRequest::put().uri("/api/v1/auth/language").insert_header(("authorization", format!("Bearer {}", cookie.value()))).insert_header(("origin", "https://evil.example")).set_json(json!({"language": "fr"}))).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
}

#[actix_web::test]
async fn rbac_and_privilege_escalation_are_enforced_by_the_api() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let admin = admin(&app).await;
    let (_, roles) = call(&app, TestRequest::get().uri("/api/v1/roles").cookie(admin.clone())).await;
    let role_id = |name: &str| roles.as_array().unwrap().iter().find(|r| r["name"] == name).unwrap()["id"].as_str().unwrap().to_string();

    // an operator and a limited role manager
    let (st, rm) = call(&app, TestRequest::post().uri("/api/v1/roles").cookie(admin.clone()).set_json(json!({"name": "role-manager", "permissions": ["roles:create", "roles:read", "hosts:read"]}))).await;
    assert_eq!(st, StatusCode::CREATED);
    for (user, role) in [("op1", role_id("operator")), ("rm1", rm["id"].as_str().unwrap().to_string()), ("aud1", role_id("auditor"))] {
        let (st, _) = call(&app, TestRequest::post().uri("/api/v1/users").cookie(admin.clone()).set_json(json!({"username": user, "password": "temporary-pass-12", "role_id": role}))).await;
        assert_eq!(st, StatusCode::CREATED);
    }
    let sign_in = |user: &'static str| {
        let app = &app;
        async move {
            let c = session(&login(app, user, "temporary-pass-12").await);
            let new_pw = format!("{user}-new-password-1");
            call(app, TestRequest::post().uri("/api/v1/auth/password").cookie(c).set_json(json!({"current_password": "temporary-pass-12", "new_password": new_pw}))).await;
            session(&login(app, user, &format!("{user}-new-password-1")).await)
        }
    };
    let op = sign_in("op1").await;
    let rmc = sign_in("rm1").await;
    let aud = sign_in("aud1").await;

    let status = |c: &Cookie<'static>, method: &str, uri: &str, body: Value| {
        let req = match method {
            "GET" => TestRequest::get(),
            "POST" => TestRequest::post(),
            "DELETE" => TestRequest::delete(),
            _ => unreachable!(),
        }
        .uri(uri)
        .cookie(c.clone())
        .set_json(body);
        let app = &app;
        async move { call(app, req).await.0 }
    };
    assert_eq!(status(&op, "GET", "/api/v1/hosts", json!({})).await, StatusCode::OK);
    assert_eq!(status(&op, "GET", "/api/v1/users", json!({})).await, StatusCode::FORBIDDEN);
    assert_eq!(status(&op, "POST", "/api/v1/groups", json!({"name": "x"})).await, StatusCode::FORBIDDEN);
    assert_eq!(status(&aud, "GET", "/api/v1/audit", json!({})).await, StatusCode::OK);
    assert_eq!(status(&aud, "POST", "/api/v1/groups", json!({"name": "x"})).await, StatusCode::FORBIDDEN, "auditor is read-only");

    // the role manager can only hand out what it holds
    for (perms, expect) in [
        (json!(["hosts:read"]), StatusCode::CREATED),
        (json!(["users:write"]), StatusCode::FORBIDDEN),
        (json!(["hosts:*"]), StatusCode::FORBIDDEN),
        (json!(["*"]), StatusCode::FORBIDDEN),
        (json!(["*:read"]), StatusCode::FORBIDDEN),
        (json!(["hosts"]), StatusCode::BAD_REQUEST),
    ] {
        let name = format!("r-{}", uuid::Uuid::new_v4().simple());
        assert_eq!(status(&rmc, "POST", "/api/v1/roles", json!({"name": name, "permissions": perms.clone()})).await, expect, "{perms}");
    }
    // ... and cannot assign a role stronger than its own
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/users").cookie(rmc.clone()).set_json(json!({"username": "sneaky", "password": "temporary-pass-12", "role_id": role_id("super_admin")}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn enrolment_tokens_are_single_use_and_keys_are_validated() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let admin = admin(&app).await;
    let (st, t) = call(&app, TestRequest::post().uri("/api/v1/enrollment-tokens").cookie(admin.clone()).set_json(json!({"label": "t", "max_uses": 1}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let token = t["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("cpt_"));

    let kp = crypto::generate_rsa(2048).unwrap();
    let facts = json!({"hostname": "web-01", "os_family": "linux", "distro": "Debian", "os_version": "12", "kernel": "6.1", "arch": "x86_64", "package_manager": "apt", "is_laptop": false, "on_battery": false, "ip_addresses": [], "agent_version": "0.1.0"});
    let enroll = |token: &str, pem: &str| TestRequest::post().uri("/api/v1/agent/enroll").set_json(json!({"token": token, "facts": facts, "public_key_pem": pem}));

    let (st, _) = call(&app, enroll("cpt_wrong", &kp.public_pem)).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "unknown token");
    let (st, _) = call(&app, enroll(&token, "not a key")).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "garbage key is refused before the token is spent");
    let weak = crypto::generate_rsa(1024).unwrap();
    let (st, _) = call(&app, enroll(&token, &weak.public_pem)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "RSA < 2048 bits is refused");
    let (st, ok) = call(&app, enroll(&token, &kp.public_pem)).await;
    assert_eq!(st, StatusCode::CREATED, "{ok}");
    assert!(ok["server_public_key_pem"].as_str().unwrap().contains("BEGIN PUBLIC KEY"));
    let (st, _) = call(&app, enroll(&token, &kp.public_pem)).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "second use of a one-use token");

    // the enrolled agent authenticates with its own key: valid once, replay refused, wrong key refused
    let agent = ok["agent_id"].as_str().unwrap().to_string();
    let mint = |key: &str, jti: &str, lifetime: i64| {
        let now = chrono::Utc::now().timestamp();
        jwt::sign_rs256(key, &Claims { sub: agent.clone(), iss: "agent".into(), iat: now - 5, exp: now + lifetime, jti: jti.into(), scope: None }).unwrap()
    };
    let telemetry = |tok: String| TestRequest::post().uri("/api/v1/agent/telemetry").insert_header(("authorization", format!("Bearer {tok}"))).set_json(json!({"messages": []}));
    let good = mint(&kp.private_pem, "jti-1", 120);
    assert_eq!(call(&app, telemetry(good.clone())).await.0, StatusCode::OK);
    assert_eq!(call(&app, telemetry(good)).await.0, StatusCode::UNAUTHORIZED, "replayed jti");
    let other = crypto::generate_rsa(2048).unwrap();
    assert_eq!(call(&app, telemetry(mint(&other.private_pem, "jti-2", 120))).await.0, StatusCode::UNAUTHORIZED, "wrong key");
    assert_eq!(call(&app, telemetry(mint(&kp.private_pem, "jti-3", 86_400))).await.0, StatusCode::UNAUTHORIZED, "token lifetime too long");
    assert_eq!(call(&app, TestRequest::post().uri("/api/v1/agent/telemetry").set_json(json!({"messages": []}))).await.0, StatusCode::UNAUTHORIZED, "no token");

    // the enrolment left an audit trail
    let (_, entries) = call(&app, TestRequest::get().uri("/api/v1/audit?action=agent.").cookie(admin)).await;
    let actions: Vec<_> = entries.as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap().to_string()).collect();
    assert!(actions.contains(&"agent.enrolled".to_string()) && actions.contains(&"agent.enroll_rejected".to_string()), "{actions:?}");
}

#[actix_web::test]
async fn policies_are_validated_on_save_and_the_audit_chain_verifies_over_http() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let admin = admin(&app).await;
    let (st, body) = call(&app, TestRequest::post().uri("/api/v1/policies").cookie(admin.clone()).set_json(json!({"name": "bad", "definition": {"rules": [{"rule_id": "x", "condition": "cvss_score >=", "contextual_risk": "High"}]}}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("rule x"), "{body}");
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/policies").cookie(admin.clone()).set_json(json!({"name": "ok", "definition": {"default_action": "BLOCK"}}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/policies").cookie(admin.clone()).set_json(json!({"name": "ok", "definition": {}}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "names are unique");
    let (st, v) = call(&app, TestRequest::get().uri("/api/v1/audit/verify").cookie(admin)).await;
    assert_eq!((st, v["ok"].clone()), (StatusCode::OK, json!(true)));
    assert!(v["entries"].as_i64().unwrap() >= 4);
}
