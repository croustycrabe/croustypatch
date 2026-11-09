//! A quarantined host has no DNS: the agent must reach its server through the addresses it saved earlier.
use croustypatch_agent::config::Config;
use croustypatch_agent::net::{connect_pinned, http_client_pinned, load_server_ips, save_server_ips, server_host_port};
use std::net::IpAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn host_and_port_are_read_from_the_server_url() {
    assert_eq!(server_host_port("https://patch.example.org"), ("patch.example.org".into(), 443));
    assert_eq!(server_host_port("https://patch.example.org:8443/x"), ("patch.example.org".into(), 8443));
    assert_eq!(server_host_port("http://10.0.0.5"), ("10.0.0.5".into(), 80));
    assert_eq!(server_host_port("https://[fd00::5]:9443"), ("fd00::5".into(), 9443));
}

#[test]
fn addresses_survive_a_restart_of_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config { state_dir: dir.path().to_path_buf(), ..Default::default() };
    assert!(load_server_ips(&cfg).is_empty());
    let ips: Vec<IpAddr> = vec!["10.0.15.10".parse().unwrap(), "fd00::1".parse().unwrap()];
    save_server_ips(&cfg, &ips);
    assert_eq!(load_server_ips(&cfg), ips);
}

#[tokio::test]
async fn an_unresolvable_server_name_is_reached_through_the_saved_address() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = s.read(&mut buf).await.unwrap();
        s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await.unwrap();
        String::from_utf8_lossy(&buf[..n]).to_string()
    });
    // ".invalid" never resolves (RFC 2606), like any name once the firewall drops DNS.
    let cfg = Config { server_url: format!("http://cp-server.invalid:{port}"), ..Default::default() };
    let ips: Vec<IpAddr> = vec!["127.0.0.1".parse().unwrap()];
    let resp = http_client_pinned(&cfg, &ips).unwrap().get(format!("{}/api/v1/ping", cfg.server_url)).send().await.unwrap();
    assert_eq!(resp.text().await.unwrap(), "ok");
    let req = server.await.unwrap();
    assert!(req.to_ascii_lowercase().contains(&format!("host: cp-server.invalid:{port}")), "the server name is kept for TLS and Host: {req}");

    // the WebSocket tunnel opens its TCP connection the same way
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = Config { server_url: format!("http://cp-server.invalid:{port}"), ..Default::default() };
    let dead: IpAddr = "127.0.0.2".parse().unwrap(); // nothing listens there: the next address is tried
    let s = connect_pinned(&cfg, &[dead, "127.0.0.1".parse().unwrap()]).await;
    assert!(s.is_ok(), "{:?}", s.err());
    assert!(connect_pinned(&cfg, &[]).await.is_err());
}
