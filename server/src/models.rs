//! Row structs returned by the API. Secrets (password hashes, host public keys) are deliberately absent.
use crate::schema::*;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

type Ts = DateTime<Utc>;

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = roles)]
pub struct Role {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub permissions: Value,
    pub builtin: bool,
    pub created_at: Ts,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = users)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role_id: Uuid,
    pub auth_source: String,
    pub is_fallback_admin: bool,
    pub disabled: bool,
    pub must_change_password: bool,
    pub language: String,
    pub created_at: Ts,
    pub last_login_at: Option<Ts>,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = host_groups)]
pub struct HostGroup {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: Ts,
    pub color: String,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = hosts)]
pub struct Host {
    pub id: Uuid,
    pub hostname: String,
    pub display_name: Option<String>,
    pub status: String,
    pub facts: Value,
    pub os_family: String,
    pub distro: String,
    pub package_manager: String,
    pub asset_type: String,
    pub environment: String,
    pub criticality: String,
    pub internet_exposed: bool,
    pub has_staging: bool,
    pub ring: i32,
    pub tags: Value,
    pub comm_mode: String,
    pub connected: bool,
    pub eco: bool,
    pub reboot_required: bool,
    pub downtime_until: Option<Ts>,
    pub heartbeat_secs: i32,
    pub last_seen_at: Option<Ts>,
    pub last_inventory_at: Option<Ts>,
    pub enrolled_at: Ts,
    pub modules: Value,
    /// Latest resource reading (cpu_pct, mem_*, swap_*, load_*, uptime_secs, disks) and when it was taken.
    pub metrics: Value,
    pub metrics_at: Option<Ts>,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = host_packages)]
pub struct HostPackage {
    pub name: String,
    pub source: String,
    pub version: String,
    pub candidate: Option<String>,
    pub is_security: bool,
    pub cves: Value,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = enrollment_tokens)]
pub struct EnrollmentToken {
    pub id: Uuid,
    pub label: String,
    pub group_id: Option<Uuid>,
    pub environment: String,
    pub comm_mode: String,
    pub expires_at: Ts,
    pub max_uses: i32,
    pub used_count: i32,
    pub revoked: bool,
    pub created_by: String,
    pub created_at: Ts,
    pub modules: Option<Value>,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = vulnerabilities)]
pub struct Vulnerability {
    pub cve_id: String,
    pub cvss: Option<f64>,
    pub severity: String,
    pub summary: String,
    pub exploited: bool,
    pub published_at: Option<Ts>,
    pub source: String,
    pub updated_at: Ts,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = policies)]
pub struct Policy {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub priority: i32,
    pub definition: Value,
    pub created_at: Ts,
    pub updated_at: Ts,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = tasks)]
pub struct Task {
    pub id: Uuid,
    pub host_id: Uuid,
    pub kind: String,
    pub status: String,
    pub policy_id: Option<Uuid>,
    pub parent_task_id: Option<Uuid>,
    pub packages: Value,
    pub decision: Option<Value>,
    pub exec: Option<Value>,
    pub result: Option<Value>,
    pub message: String,
    pub requested_by: String,
    pub approved_by: Option<String>,
    pub scheduled_for: Ts,
    pub sent_at: Option<Ts>,
    pub started_at: Option<Ts>,
    pub finished_at: Option<Ts>,
    pub soak_until: Option<Ts>,
    pub created_at: Ts,
    pub updated_at: Ts,
    /// An automatic patch must not start after this moment (the end of its patch day).
    pub deadline_at: Option<Ts>,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = task_events)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: Uuid,
    pub ts: Ts,
    pub status: String,
    pub message: String,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = waivers)]
pub struct Waiver {
    pub id: Uuid,
    pub host_id: Option<Uuid>,
    pub group_id: Option<Uuid>,
    pub cve_id: Option<String>,
    pub package: Option<String>,
    pub reason: String,
    pub compensating_controls: String,
    pub requested_by: String,
    pub expires_at: Ts,
    pub status: String,
    pub ciso_approved_by: Option<String>,
    pub ciso_approved_at: Option<Ts>,
    pub business_approved_by: Option<String>,
    pub business_approved_at: Option<Ts>,
    pub created_at: Ts,
}

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = audit_log)]
pub struct AuditEntry {
    pub seq: i64,
    pub ts: Ts,
    pub actor: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub host_id: Option<Uuid>,
    pub details: Value,
    pub ip: String,
    pub prev_hash: String,
    pub hash: String,
}
