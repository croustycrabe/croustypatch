//! Shared harness for the integration tests: a throwaway PostgreSQL database per test.
//!
//! Needs a reachable PostgreSQL server. By default the development one from `docker/docker-compose.yml`
//! (`docker compose -f docker/docker-compose.yml up -d`); override with `TEST_DATABASE_URL`, a URL for a
//! role allowed to create databases. Each test creates `cp_test_<random>` and drops it afterwards.
#![allow(dead_code)]
pub mod http;
use croustypatch_server::config::{Config, TlsMode};
use croustypatch_server::crypto::MasterKey;
use croustypatch_server::state::{AppState, PgPool};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};
use std::sync::Mutex;
use zeroize::Zeroizing;

const DEFAULT_ADMIN_URL: &str = "postgres://croustypatch:croustypatch-dev@127.0.0.1:5433/postgres";

pub struct TestDb {
    pub pool: PgPool,
    name: String,
    admin_url: String,
}

fn admin_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| DEFAULT_ADMIN_URL.to_string())
}

/// `postgres://u:p@h:port/db` -> same URL with another database name.
fn with_db(url: &str, db: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("database URL must end with /dbname");
    format!("{base}/{db}")
}

impl TestDb {
    pub fn new() -> Self {
        let admin_url = admin_url();
        let name = format!("cp_test_{}", uuid::Uuid::new_v4().simple());
        let mut admin = PgConnection::establish(&admin_url).unwrap_or_else(|e| {
            panic!("cannot reach PostgreSQL at {admin_url} ({e}).\nStart it with `docker compose -f docker/docker-compose.yml up -d` or set TEST_DATABASE_URL.")
        });
        diesel::sql_query(format!("CREATE DATABASE {name}")).execute(&mut admin).expect("create test database");
        // Small, lazy pools: tests run in parallel and PostgreSQL's default limit is 100 connections.
        let mgr = ConnectionManager::<PgConnection>::new(with_db(&admin_url, &name));
        let pool = Pool::builder().max_size(10).min_idle(Some(0)).connection_timeout(std::time::Duration::from_secs(20)).build(mgr).expect("connect to test database");
        croustypatch_server::db::migrate(&pool).expect("migrations");
        Self { pool, name, admin_url }
    }

    pub fn conn(&self) -> PooledConnection<ConnectionManager<PgConnection>> {
        self.pool.get().expect("connection")
    }

    /// A server state wired to this database, with throwaway keys.
    pub fn state(&self) -> AppState {
        self.state_with(|_| {})
    }

    /// Same, with the configuration adjusted first (for example the agent directory).
    pub fn state_with(&self, adjust: impl FnOnce(&mut Config)) -> AppState {
        let mut cfg = test_config();
        adjust(&mut cfg);
        let master = MasterKey::from_bytes([42; 32]);
        let kp = croustypatch_common::crypto::generate_rsa(2048).expect("server key");
        let session_secret = master.derive("ui-session-v1");
        AppState {
            pool: self.pool.clone(),
            cfg,
            master,
            server_private_pem: Zeroizing::new(kp.private_pem),
            server_public_pem: kp.public_pem,
            session_secret,
            hub: Default::default(),
            tls_direct: true,
            seen_jti: Mutex::new(Default::default()),
        }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // FORCE closes any connection still held by the pool.
        if let Ok(mut admin) = PgConnection::establish(&self.admin_url) {
            let _ = diesel::sql_query(format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name)).execute(&mut admin);
        }
    }
}

pub fn test_config() -> Config {
    Config {
        database_url: String::new(),
        listen: String::new(),
        data_dir: ".".into(),
        master_key: None,
        tls_mode: TlsMode::Direct,
        tls_cert: None,
        tls_key: None,
        tls_san: vec![],
        trusted_proxies: vec!["127.0.0.1".into()],
        static_dir: ".".into(),
        agent_dir: vec![".".into()],
        scap_dir: "./no-scap-content".into(),
        cors_origin: None,
    }
}
