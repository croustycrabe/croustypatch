//! Tests for `auth`.
use croustypatch_server::config::Config;
use croustypatch_server::auth::*;

#[test]
fn permission_patterns() {
    assert!(pattern_allows("*", "hosts:read"));
    assert!(pattern_allows("hosts:*", "hosts:write"));
    assert!(!pattern_allows("hosts:*", "tasks:write"));
    assert!(pattern_allows("*:read", "audit:read"));
    assert!(!pattern_allows("*:read", "audit:write"));
    assert!(pattern_allows("waivers:approve_ciso", "waivers:approve_ciso"));
    assert!(!pattern_allows("waivers:*", "waivers"), "malformed needs never match a pattern");
    assert!(!pattern_allows("hosts:read", "hosts:readwrite"));
}

#[test]
fn builtin_roles_are_sane() {
    let auditor = BUILTIN_ROLES.iter().find(|r| r.0 == "auditor").unwrap();
    let perms: Vec<String> = auditor.2.iter().map(|s| s.to_string()).collect();
    assert!(perms_allow(&perms, "hosts:read"));
    assert!(!perms_allow(&perms, "tasks:create"));
    let admin: Vec<String> = BUILTIN_ROLES.iter().find(|r| r.0 == "admin").unwrap().2.iter().map(|s| s.to_string()).collect();
    assert!(!perms_allow(&admin, "users:write"), "admin must not manage accounts");
    assert!(perms_allow(&admin, "tasks:create"));
}

#[test]
fn public_host_falls_back_to_authority_for_http2() {
    let cfg = Config {
        database_url: String::new(),
        listen: String::new(),
        data_dir: ".".into(),
        master_key: None,
        tls_mode: croustypatch_server::config::TlsMode::Direct,
        tls_cert: None,
        tls_key: None,
        tls_san: vec![],
        trusted_proxies: vec!["127.0.0.1".into()],
        static_dir: ".".into(),
        agent_dir: vec![".".into()],
        cors_origin: None,
    };
    // HTTP/2 style: absolute-form URI, no Host header.
    let h2 = actix_web::test::TestRequest::default().uri("https://patch.example.com:8443/api/v1/x").to_http_request();
    assert_eq!(public_host(&h2, &cfg), "patch.example.com:8443");
    assert!(csrf_ok(&actix_web::test::TestRequest::default().uri("https://patch.example.com:8443/x").insert_header(("origin", "https://patch.example.com:8443")).to_http_request(), &cfg));
    // HTTP/1.1 style
    let h1 = actix_web::test::TestRequest::default().uri("/x").insert_header(("host", "Patch.Example.com")).to_http_request();
    assert_eq!(public_host(&h1, &cfg), "patch.example.com");
    // A foreign origin is still refused
    let evil = actix_web::test::TestRequest::default().uri("https://patch.example.com/x").insert_header(("origin", "https://evil.example")).to_http_request();
    assert!(!csrf_ok(&evil, &cfg));
    // Forwarded host is ignored unless the peer is a trusted proxy (no peer address in tests => untrusted)
    let spoof = actix_web::test::TestRequest::default().uri("/x").insert_header(("host", "a.example")).insert_header(("x-forwarded-host", "evil.example")).to_http_request();
    assert_eq!(public_host(&spoof, &cfg), "a.example");
}

#[test]
fn password_policy() {
    assert!(check_password_policy("short").is_err());
    assert!(check_password_policy("long-enough-pass").is_ok());
}
