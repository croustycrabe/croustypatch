//! Tests for `health`.
use croustypatch_agent::exec::Runner;
use croustypatch_common::model::HealthProbe;
use croustypatch_agent::health::*;

#[tokio::test]
async fn tcp_probe() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let r = Runner { dry_run: false };
    assert!(check(&HealthProbe::Tcp { host: "127.0.0.1".into(), port }, &r).await.is_ok());
    drop(l);
    // Other tests fork child processes; until such a child calls exec() it still holds a copy of
    // the listener's descriptor, so the port can stay open for a few milliseconds after drop().
    let mut closed = false;
    for _ in 0..100 {
        if check(&HealthProbe::Tcp { host: "127.0.0.1".into(), port }, &r).await.is_err() {
            closed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(closed, "probe kept succeeding after the listener was dropped");
}

#[tokio::test]
async fn rejects_dangerous_inputs() {
    let r = Runner { dry_run: false };
    assert!(check(&HealthProbe::Systemd { unit: "--help".into() }, &r).await.unwrap_err().contains("invalid"));
    assert!(check(&HealthProbe::Http { url: "file:///etc/passwd".into(), expect_status: 200 }, &r).await.is_err());
}
