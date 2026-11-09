//! Harness: a real agent task runner wired to channels, the mock package backend and a simulated service.
#![allow(dead_code)]
use chrono::{Duration, Utc};
use croustypatch_agent::config::Config;
use croustypatch_agent::exec::Runner;
use croustypatch_agent::identity::Identity;
use croustypatch_agent::pkg::mock::Mock;
use croustypatch_agent::pkg::Backend;
use croustypatch_agent::runner::{worker, Agent};
use croustypatch_common::crypto;
use croustypatch_common::protocol::{AgentMessage, CommandAction, CommandPayload, TaskUpdate};
use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct Rig {
    pub dir: tempfile::TempDir,
    pub mock_path: PathBuf,
    pub cmd_tx: mpsc::UnboundedSender<CommandPayload>,
    pub out_rx: mpsc::UnboundedReceiver<AgentMessage>,
    pub agent: Arc<Agent>,
    pub agent_id: Uuid,
}

pub fn pkg(name: &str, version: &str, candidate: &str, security: bool) -> Value {
    json!({"name": name, "version": version, "candidate": candidate, "is_security": security, "cves": []})
}

/// `db` is the JSON content of the mock package database.
pub fn rig(db: Value) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let mock_path = dir.path().join("mock.json");
    std::fs::write(&mock_path, serde_json::to_vec(&db).unwrap()).unwrap();
    let cfg = Config {
        server_url: "http://127.0.0.1:1".into(),
        state_dir: dir.path().join("state"),
        dry_run: true, // the package backend is a file; only firewall changes are skipped
        probe_interval_secs: 1,
        ..Default::default()
    };
    let kp = crypto::generate_rsa(2048).unwrap();
    let agent_id = Uuid::new_v4();
    let identity = Identity { agent_id, server_public_key_pem: String::new(), private_key_pem: kp.private_pem };
    let (out, out_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let agent = Arc::new(Agent {
        runner: Runner { dry_run: true },
        cfg,
        identity,
        backend: Backend::Mock(Mock::new(mock_path.clone())),
        out,
        server_ips: Mutex::new(vec!["127.0.0.1".parse().unwrap()]),
        running: Mutex::new(Default::default()),
        settings: Mutex::new(None),
    });
    tokio::spawn(worker(agent.clone(), cmd_rx));
    Rig { dir, mock_path, cmd_tx, out_rx, agent, agent_id }
}

impl Rig {
    pub fn send(&self, task_id: Uuid, action: CommandAction) {
        let now = Utc::now();
        let p = CommandPayload { task_id, agent_id: self.agent_id, issued_at: now, expires_at: now + Duration::minutes(10), nonce: Uuid::new_v4().to_string(), action };
        self.cmd_tx.send(p).unwrap();
    }

    /// Collect this task's updates until a terminal one (or panic after `secs`).
    pub async fn until_terminal(&mut self, task_id: Uuid, secs: u64) -> Vec<TaskUpdate> {
        let mut got = vec![];
        let deadline = tokio::time::Instant::now() + StdDuration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, self.out_rx.recv()).await {
                Ok(Some(AgentMessage::TaskUpdate(u))) if u.task_id == task_id => {
                    let done = u.status.is_terminal();
                    got.push(u);
                    if done {
                        return got;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => panic!("task {task_id} did not finish within {secs}s; updates so far: {:?}", got.iter().map(|u| (u.status, u.message.clone())).collect::<Vec<_>>()),
            }
        }
    }

    pub fn db(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.mock_path).unwrap()).unwrap()
    }

    pub fn version(&self, name: &str) -> String {
        self.db()["installed"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap()["version"].as_str().unwrap().to_string()
    }
}

/// A TCP "service" that is healthy unless `libssl3` is at a version starting with `bad_prefix`.
pub struct Service {
    pub port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Service {
    pub fn start(mock_path: PathBuf, bad_prefix: &'static str) -> Self {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut listener: Option<TcpListener> = None;
            while !s2.load(Ordering::Relaxed) {
                let healthy = std::fs::read(&mock_path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| v["installed"].as_array().and_then(|a| a.iter().find(|p| p["name"] == "libssl3").map(|p| p["version"].as_str().unwrap_or("").to_string())))
                    .map(|v| !v.starts_with(bad_prefix))
                    .unwrap_or(true);
                if healthy && listener.is_none() {
                    // may briefly fail while another process still holds the old socket: just retry
                    if let Ok(l) = TcpListener::bind(("127.0.0.1", port)) {
                        l.set_nonblocking(true).unwrap();
                        listener = Some(l);
                    }
                }
                if !healthy {
                    listener = None;
                }
                if let Some(l) = &listener {
                    let _ = l.accept(); // drain connections from the probe
                }
                std::thread::sleep(StdDuration::from_millis(40));
            }
        });
        // wait until it is really listening
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(StdDuration::from_millis(20));
        }
        Self { port, stop, handle: Some(handle) }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
