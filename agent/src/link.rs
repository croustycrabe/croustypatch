//! Connection to the server: WebSocket tunnel (pull) or periodic HTTPS telemetry (push).
//! Every command is verified (signature, addressee, validity window, nonce) before it reaches the runner.
use crate::facts;
use crate::net;
use crate::runner::Agent;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use croustypatch_common::protocol::*;
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

pub struct Link {
    pub agent: Arc<Agent>,
    pub cmd_tx: mpsc::UnboundedSender<CommandPayload>,
    nonces: Mutex<HashMap<String, i64>>,
    /// Messages that could not be delivered yet; retried first on the next connection.
    pending: Mutex<VecDeque<AgentMessage>>,
    /// CPU samples between heartbeats.
    sampler: crate::sysinfo::Sampler,
}

impl Link {
    pub fn new(agent: Arc<Agent>, cmd_tx: mpsc::UnboundedSender<CommandPayload>) -> Self {
        Self { agent, cmd_tx, nonces: Mutex::new(HashMap::new()), pending: Mutex::new(VecDeque::new()), sampler: Default::default() }
    }

    /// Verify and queue a command. Returns false if it was rejected.
    pub fn accept(&self, cmd: &SignedCommand) -> bool {
        let now = Utc::now();
        let p = match verify_command(&self.agent.identity.server_public_key_pem, cmd, self.agent.identity.agent_id, now) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("rejected command: {e}");
                return false;
            }
        };
        {
            let mut seen = self.nonces.lock().unwrap();
            seen.retain(|_, exp| *exp > now.timestamp());
            if seen.insert(p.nonce.clone(), p.expires_at.timestamp()).is_some() {
                tracing::warn!(task = %p.task_id, "rejected command: nonce replay");
                return false;
            }
        }
        tracing::info!(task = %p.task_id, "command accepted");
        self.cmd_tx.send(p).is_ok()
    }

    async fn heartbeat(&self) -> Heartbeat {
        let (is_laptop, on_battery) = facts::power_state().await;
        let eco = self.agent.eco_allowed() && is_laptop && on_battery;
        let load_1m = std::fs::read_to_string("/proc/loadavg").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse().ok())).unwrap_or(0.0);
        let uptime_secs = std::fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok())).unwrap_or(0.0) as u64;
        let metrics = Some(crate::sysinfo::metrics(&self.sampler).await);
        Heartbeat { on_battery, eco, load_1m, uptime_secs, running_tasks: self.agent.running.lock().unwrap().iter().copied().collect(), metrics }
    }

    fn interval(&self, eco: bool) -> Duration {
        Duration::from_secs(if eco { self.agent.cfg.eco_heartbeat_secs } else { self.agent.cfg.heartbeat_secs }.max(5))
    }

    /// New settings from the server; a module switch changes what the inventory holds, so send a fresh one.
    fn settings(&self, s: AgentSettings) {
        if self.agent.apply_settings(s) {
            self.spawn_inventory();
        }
    }

    fn spawn_inventory(&self) {
        let a = self.agent.clone();
        tokio::spawn(async move {
            if let Err(e) = a.send_inventory().await {
                tracing::warn!("inventory failed: {e:#}");
            }
        });
    }

    /// Resolve the server again; keep (and persist) the result, or fall back to the last known addresses.
    async fn refresh_server_ips(&self) {
        let cfg = &self.agent.cfg;
        match crate::isolate::resolve_server(&cfg.server_url).await {
            Ok(ips) if !ips.is_empty() => {
                let changed = *self.agent.server_ips.lock().unwrap() != ips;
                if changed || net::load_server_ips(cfg) != ips {
                    net::save_server_ips(cfg, &ips);
                }
                *self.agent.server_ips.lock().unwrap() = ips;
            }
            other => {
                let mut cur = self.agent.server_ips.lock().unwrap();
                if cur.is_empty() {
                    *cur = net::load_server_ips(cfg);
                }
                if let Err(e) = other {
                    tracing::warn!("{e:#}; using the last known server addresses {:?}", *cur);
                }
            }
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    let base = 2u64.saturating_pow(attempt.min(6)).min(60);
    Duration::from_millis(base * 1000 + rand::thread_rng().gen_range(0..1000))
}

// --------------------------------------------------------------------------- pull

pub async fn run_pull(link: Arc<Link>, mut out_rx: mpsc::UnboundedReceiver<AgentMessage>) {
    let mut attempt = 0u32;
    loop {
        link.refresh_server_ips().await;
        match pull_session(&link, &mut out_rx, &mut attempt).await {
            Ok(()) => tracing::info!("tunnel closed by server"),
            Err(e) if format!("{e:#}").contains("401") => tracing::error!("the server refuses this agent's identity (401): this host was probably deleted in the web UI or enrolled again elsewhere. Re-run the installer with a new token from Hosts > Add agent"),
            Err(e) => tracing::warn!("tunnel error: {e:#}"),
        }
        attempt += 1;
        let wait = backoff(attempt);
        tracing::info!(secs = wait.as_secs(), "reconnecting");
        tokio::time::sleep(wait).await;
    }
}

async fn pull_session(link: &Link, out_rx: &mut mpsc::UnboundedReceiver<AgentMessage>, attempt: &mut u32) -> Result<()> {
    let cfg = &link.agent.cfg;
    let mut req = net::ws_url(cfg).into_client_request()?;
    req.headers_mut().insert("authorization", format!("Bearer {}", link.agent.identity.jwt()?).parse()?);
    let connector = if cfg.server_url.starts_with("https://") { Some(Connector::Rustls(Arc::new(net::tls_config(cfg)?))) } else { None };
    let ips = link.agent.server_ips();
    let handshake = async {
        if ips.is_empty() {
            tokio_tungstenite::connect_async_tls_with_config(req, None, false, connector).await
        } else {
            // Connect to a known address ourselves: works without DNS, e.g. while quarantined.
            let tcp = net::connect_pinned(cfg, &ips).await.map_err(|e| tokio_tungstenite::tungstenite::Error::Io(std::io::Error::other(e.to_string())))?;
            tokio_tungstenite::client_async_tls_with_config(req, tcp, None, connector).await
        }
    };
    let (ws, _) = tokio::time::timeout(Duration::from_secs(30), handshake).await.map_err(|_| anyhow!("connect timed out"))?.context("WebSocket handshake")?;
    let (mut sink, mut stream) = ws.split();
    *attempt = 0;
    tracing::info!("tunnel established");

    let facts = facts::collect(&cfg.server_url, link.agent.backend.kind(), vec![]).await;
    sink.send(Message::text(serde_json::to_string(&AgentMessage::Hello { facts, protocol: croustypatch_common::PROTOCOL_VERSION })?)).await?;
    link.spawn_inventory();

    // Replay anything that failed to go out last time.
    let backlog: Vec<AgentMessage> = link.pending.lock().unwrap().drain(..).collect();
    for m in backlog {
        if let Err(e) = send(&mut sink, &m).await {
            link.pending.lock().unwrap().push_back(m);
            return Err(e);
        }
    }

    // Report resources right away: a host that just connected should not look empty for a minute.
    let first = link.heartbeat().await;
    let mut eco = first.eco;
    send(&mut sink, &AgentMessage::Heartbeat(first)).await?;
    let mut last_hb = Instant::now();
    let mut last_inv = Instant::now();
    loop {
        let wake = link.interval(eco).min(Duration::from_secs(30));
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<ServerMessage>(&t) {
                    Ok(ServerMessage::Command(c)) => { link.accept(&c); }
                    Ok(ServerMessage::Ping) => link.spawn_inventory(),
                    Ok(ServerMessage::Settings(s)) => link.settings(s),
                    Ok(ServerMessage::Welcome { heartbeat_secs }) => tracing::debug!(heartbeat_secs, "welcome"),
                    Err(e) => tracing::warn!("unreadable server message: {e}"),
                },
                Some(Ok(Message::Ping(p))) => { sink.send(Message::Pong(p)).await?; }
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(e)) => return Err(e.into()),
                _ => {}
            },
            out = out_rx.recv() => {
                let Some(m) = out else { return Ok(()) };
                if let Err(e) = send(&mut sink, &m).await { link.pending.lock().unwrap().push_back(m); return Err(e); }
            }
            _ = tokio::time::sleep(wake) => {
                let hb = link.heartbeat().await;
                let eco_changed = hb.eco != eco;
                eco = hb.eco;
                // An economy-profile change is reported immediately so the server flips the host status.
                if eco_changed || last_hb.elapsed() >= link.interval(eco) {
                    send(&mut sink, &AgentMessage::Heartbeat(hb)).await?;
                    last_hb = Instant::now();
                }
                if last_inv.elapsed() >= Duration::from_secs(link.agent.cfg.inventory_interval_secs.max(60)) {
                    link.spawn_inventory();
                    last_inv = Instant::now();
                }
            }
        }
    }
}

async fn send<S>(sink: &mut S, m: &AgentMessage) -> Result<()>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    sink.send(Message::text(serde_json::to_string(m)?)).await.map_err(|e| anyhow!(e))
}

// --------------------------------------------------------------------------- push

pub async fn run_push(link: Arc<Link>, mut out_rx: mpsc::UnboundedReceiver<AgentMessage>) {
    let mut attempt = 0u32;
    let mut last_inv = Instant::now() - Duration::from_secs(1_000_000);
    let mut eco;
    loop {
        link.refresh_server_ips().await;
        let client = match net::http_client_pinned(&link.agent.cfg, &link.agent.server_ips()) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("cannot build HTTP client: {e:#}");
                tokio::time::sleep(backoff(attempt.max(3))).await;
                continue;
            }
        };
        let hb = link.heartbeat().await;
        eco = hb.eco;
        let mut msgs: Vec<AgentMessage> = link.pending.lock().unwrap().drain(..).collect();
        while let Ok(m) = out_rx.try_recv() {
            msgs.push(m);
        }
        msgs.push(AgentMessage::Heartbeat(hb));
        if last_inv.elapsed() >= Duration::from_secs(link.agent.cfg.inventory_interval_secs.max(60)) {
            link.spawn_inventory();
            last_inv = Instant::now();
        }
        let jwt = match link.agent.identity.jwt() {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("cannot sign JWT: {e:#}");
                return;
            }
        };
        let res = client.post(net::api_url(&link.agent.cfg, "/agent/telemetry")).bearer_auth(jwt).json(&TelemetryRequest { messages: msgs.clone() }).send().await;
        let wait = match res {
            Ok(r) if r.status().is_success() => match r.json::<TelemetryResponse>().await {
                Ok(t) => {
                    attempt = 0;
                    if let Some(s) = t.settings {
                        link.settings(s);
                    }
                    for c in &t.commands {
                        link.accept(c);
                    }
                    // While a task runs, poll quickly so its updates and follow-up commands flow.
                    if link.agent.running.lock().unwrap().is_empty() { link.interval(eco) } else { Duration::from_secs(5) }
                }
                Err(e) => {
                    tracing::warn!("bad telemetry response: {e}");
                    link.pending.lock().unwrap().extend(msgs.into_iter().filter(|m| !matches!(m, AgentMessage::Heartbeat(_))));
                    backoff(attempt + 1)
                }
            },
            Ok(r) => {
                if r.status() == reqwest::StatusCode::UNAUTHORIZED {
                    tracing::error!("the server refuses this agent's identity (401): this host was probably deleted in the web UI or enrolled again elsewhere. Re-run the installer with a new token from Hosts > Add agent");
                }
                tracing::warn!(status = %r.status(), "telemetry rejected");
                attempt += 1;
                link.pending.lock().unwrap().extend(msgs.into_iter().filter(|m| !matches!(m, AgentMessage::Heartbeat(_))));
                backoff(attempt)
            }
            Err(e) => {
                tracing::warn!("telemetry failed: {e}");
                attempt += 1;
                link.pending.lock().unwrap().extend(msgs.into_iter().filter(|m| !matches!(m, AgentMessage::Heartbeat(_))));
                backoff(attempt)
            }
        };
        // Wake early when the runner has something to report.
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            m = out_rx.recv() => { if let Some(m) = m { link.pending.lock().unwrap().push_back(m); } tokio::time::sleep(Duration::from_millis(500)).await; }
        }
    }
}
