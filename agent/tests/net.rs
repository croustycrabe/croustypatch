//! Tests for `net`.
use croustypatch_agent::config::Config;
use croustypatch_agent::net::*;

#[test]
fn urls() {
    let mut c = Config { server_url: "https://patch.example.com/".into(), ..Default::default() };
    assert_eq!(api_url(&c, "/agent/enroll"), "https://patch.example.com/api/v1/agent/enroll");
    assert_eq!(ws_url(&c), "wss://patch.example.com/api/v1/agent/ws");
    c.server_url = "http://127.0.0.1:8080".into();
    assert_eq!(ws_url(&c), "ws://127.0.0.1:8080/api/v1/agent/ws");
}
