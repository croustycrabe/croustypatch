//! CroustyPatch server library. The binary (`main.rs`) is only the CLI and HTTP wiring on top of this.
pub mod ai;
pub mod api;
pub mod audit;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod docker_migration;
pub mod docker_registry;
pub mod engine;
pub mod error;
pub mod gateway;
pub mod icons;
pub mod logs;
pub mod models;
pub mod modules;
pub mod notify;
pub mod planner;
pub mod reboot;
pub mod reports;
pub mod scheduler;
pub mod schema;
pub mod state;
pub mod tls;

// `pq-sys` is only linked for its bundled libpq; keep the crate referenced.
use pq_sys as _;
