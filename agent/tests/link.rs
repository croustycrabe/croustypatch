//! Integration: what the agent accepts from the wire. Every command is verified before it reaches the runner.
mod support;

use chrono::{Duration, Utc};
use croustypatch_agent::config::Config;
use croustypatch_agent::exec::Runner;
use croustypatch_agent::identity::Identity;
use croustypatch_agent::link::Link;
use croustypatch_agent::pkg::mock::Mock;
use croustypatch_agent::pkg::Backend;
use croustypatch_agent::runner::Agent;
use croustypatch_common::crypto;
use croustypatch_common::protocol::{sign_command, CommandAction, CommandPayload};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use uuid::Uuid;

struct Wire {
    link: Link,
    cmd_rx: mpsc::UnboundedReceiver<CommandPayload>,
    server_key: crypto::KeyPair,
    agent_id: Uuid,
    _dir: tempfile::TempDir,
}

fn wire() -> Wire {
    let dir = tempfile::tempdir().unwrap();
    let server_key = crypto::generate_rsa(2048).unwrap();
    let agent_key = crypto::generate_rsa(2048).unwrap();
    let agent_id = Uuid::new_v4();
    let (out, _out_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let agent = Arc::new(Agent {
        cfg: Config { state_dir: dir.path().into(), ..Default::default() },
        identity: Identity { agent_id, server_public_key_pem: server_key.public_pem.clone(), private_key_pem: agent_key.private_pem },
        backend: Backend::Mock(Mock::new(dir.path().join("m.json"))),
        runner: Runner { dry_run: true },
        out,
        server_ips: Mutex::new(vec![]),
        running: Mutex::new(Default::default()),
    });
    Wire { link: Link::new(agent, cmd_tx), cmd_rx, server_key, agent_id, _dir: dir }
}

fn payload(agent: Uuid, nonce: &str, issued: chrono::DateTime<Utc>, ttl: Duration) -> CommandPayload {
    CommandPayload { task_id: Uuid::new_v4(), agent_id: agent, issued_at: issued, expires_at: issued + ttl, nonce: nonce.into(), action: CommandAction::Scan }
}

#[tokio::test]
async fn only_valid_fresh_unique_commands_for_this_agent_are_accepted() {
    let mut w = wire();
    let sign = |p: &CommandPayload| sign_command(&w.server_key.private_pem, p).unwrap();
    let now = Utc::now();

    let good = sign(&payload(w.agent_id, "n1", now, Duration::minutes(5)));
    assert!(w.link.accept(&good));
    assert!(w.cmd_rx.try_recv().is_ok(), "accepted commands reach the runner");

    assert!(!w.link.accept(&good), "replay of the same nonce");
    assert!(!w.link.accept(&sign(&payload(Uuid::new_v4(), "n2", now, Duration::minutes(5)))), "addressed to another agent");
    assert!(!w.link.accept(&sign(&payload(w.agent_id, "n3", now - Duration::minutes(30), Duration::minutes(5)))), "expired");
    assert!(!w.link.accept(&sign(&payload(w.agent_id, "n4", now + Duration::minutes(30), Duration::minutes(5)))), "issued in the future");

    let mut tampered = sign(&payload(w.agent_id, "n5", now, Duration::minutes(5)));
    tampered.payload = tampered.payload.replace("scan", "quarantine");
    assert!(!w.link.accept(&tampered), "payload changed after signing");

    let impostor = crypto::generate_rsa(2048).unwrap();
    assert!(!w.link.accept(&sign_command(&impostor.private_pem, &payload(w.agent_id, "n6", now, Duration::minutes(5))).unwrap()), "signed by someone else");
    assert!(w.cmd_rx.try_recv().is_err(), "nothing rejected ever reached the runner");
}
