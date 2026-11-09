use crate::config::{write_private, Config};
use anyhow::{Context, Result};
use croustypatch_common::crypto;
use croustypatch_common::jwt::{self, Claims};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub agent_id: Uuid,
    pub server_public_key_pem: String,
    pub private_key_pem: String,
}

fn path(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("identity.json")
}

impl Identity {
    pub fn load(cfg: &Config) -> Result<Self> {
        let p = path(cfg);
        let text = std::fs::read_to_string(&p).with_context(|| format!("{} not found: has this host been enrolled? (croustypatch-agent enroll …)", p.display()))?;
        serde_json::from_str(&text).context("identity file is corrupt")
    }

    pub fn save(&self, cfg: &Config) -> Result<()> {
        std::fs::create_dir_all(&cfg.state_dir)?;
        write_private(&path(cfg), serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    pub fn exists(cfg: &Config) -> bool {
        path(cfg).exists()
    }

    /// Short-lived RS256 JWT proving possession of this agent's private key.
    pub fn jwt(&self) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims { sub: self.agent_id.to_string(), iss: "agent".into(), iat: now - 5, exp: now + 120, jti: crypto::random_token(16), scope: None };
        jwt::sign_rs256(&self.private_key_pem, &claims).map_err(Into::into)
    }
}
