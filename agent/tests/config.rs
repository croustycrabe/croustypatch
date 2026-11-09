//! Tests for `config`.
use croustypatch_agent::config::*;

#[test]
fn https_required_except_loopback() {
    let mut c = Config { server_url: "http://example.com".into(), ..Default::default() };
    assert!(c.validate().is_err());
    c.server_url = "http://127.0.0.1:8080".into();
    assert!(c.validate().is_ok());
    c.server_url = "https://patch.example.com".into();
    assert!(c.validate().is_ok());
    c.mode = "sideways".into();
    assert!(c.validate().is_err());
}

#[test]
fn partial_toml_uses_defaults() {
    let c: Config = toml::from_str("server_url = \"https://x\"\nmode = \"push\"").unwrap();
    assert_eq!(c.heartbeat_secs, 60);
    assert_eq!(c.mode, "push");
}
