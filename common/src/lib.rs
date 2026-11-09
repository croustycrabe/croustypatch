//! Types and primitives shared by the CroustyPatch server and agent.
pub mod crypto;
pub mod jwt;
pub mod model;
pub mod protocol;

pub const API_PREFIX: &str = "/api/v1";
pub const PROTOCOL_VERSION: u32 = 1;
