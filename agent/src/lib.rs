//! CroustyPatch agent library. The binary (`main.rs`) is only the CLI on top of this.
pub mod app_command;
pub mod config;
pub mod exec;
pub mod facts;
pub mod health;
pub mod identity;
pub mod isolate;
pub mod link;
pub mod modules;
pub mod net;
pub mod pkg;
pub mod portal;
pub mod tui;
pub mod preflight;
pub mod reboot;
pub mod runner;
pub mod selfupdate;
