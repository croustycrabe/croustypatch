//! Connection pool and embedded migrations.
use crate::config::Config;
use crate::state::PgPool;
use anyhow::{Context, Result};
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

pub fn connect_url(url: &str) -> Result<PgPool> {
    let mgr = ConnectionManager::<PgConnection>::new(url);
    Pool::builder().max_size(16).connection_timeout(std::time::Duration::from_secs(10)).build(mgr).context("connecting to PostgreSQL")
}

pub fn connect(cfg: &Config) -> Result<PgPool> {
    connect_url(&cfg.database_url)
}

/// Apply pending migrations; returns how many were applied.
pub fn migrate(pool: &PgPool) -> Result<usize> {
    let mut conn = pool.get()?;
    let applied = conn.run_pending_migrations(MIGRATIONS).map_err(|e| anyhow::anyhow!("migrations failed: {e}"))?;
    Ok(applied.len())
}
