use crate::{auth, gateway};
use actix_web::web;

/// Inventories and CVE feeds are large; everything else keeps the small default.
const BIG_JSON: usize = 16 * 1024 * 1024;

pub(crate) mod admin;
mod ai;
pub(crate) mod compliance_api;
mod mcp;
pub mod agent_dist;
pub(crate) mod notifications;
mod docker;
mod docker_shell;
mod reports;
mod packages;
pub mod portal;
mod fleet;
pub mod patching;
mod system;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .route("/health", web::get().to(system::health))
            .route("/server-info", web::get().to(system::server_info))
            // --- agents (JWT / enrolment token, not UI sessions)
            .route("/agent/enroll", web::post().to(gateway::enroll))
            .service(web::resource("/agent/telemetry").app_data(web::JsonConfig::default().limit(BIG_JSON)).route(web::post().to(gateway::telemetry)))
            .route("/agent/ws", web::get().to(gateway::ws_tunnel))
            .route("/agent/portal", web::get().to(portal::agent_view))
            .route("/agent/portal/install", web::post().to(portal::agent_request))
            .route("/agent/portal/updates", web::get().to(portal::agent_updates))
            .route("/agent/portal/update", web::post().to(portal::agent_update))
            .route("/agent/scap-content", web::get().to(agent_dist::scap_index))
            .route("/agent/scap-content/{name}", web::get().to(agent_dist::scap_file))
            // --- "Add agent": installer script and binaries (public, no secrets)
            .route("/agent/install.sh", web::get().to(agent_dist::install_script))
            .route("/agent/platforms", web::get().to(agent_dist::platforms))
            .route("/agent/download/{platform}", web::get().to(agent_dist::download))
            // --- session
            .route("/auth/login", web::post().to(auth::login))
            .route("/auth/logout", web::post().to(auth::logout))
            .route("/auth/me", web::get().to(auth::me))
            .route("/auth/password", web::post().to(auth::change_password))
            .route("/auth/language", web::put().to(auth::set_language))
            .route("/auth/profile", web::patch().to(auth::update_profile))
            .route("/dashboard/layout", web::get().to(auth::get_layout))
            .route("/dashboard/layout", web::put().to(auth::put_layout))
            .route("/dashboard", web::get().to(system::dashboard))
            // --- accounts & settings
            .route("/users", web::get().to(admin::list_users))
            .route("/users", web::post().to(admin::create_user))
            .route("/users/{id}", web::patch().to(admin::update_user))
            .route("/users/{id}", web::delete().to(admin::delete_user))
            .route("/roles", web::get().to(admin::list_roles))
            .route("/roles", web::post().to(admin::create_role))
            .route("/roles/{id}", web::put().to(admin::update_role))
            .route("/roles/{id}", web::delete().to(admin::delete_role))
            .route("/modules", web::get().to(admin::modules))
            .route("/settings", web::get().to(admin::get_settings))
            .route("/settings/{key}", web::put().to(admin::put_setting))
            .route("/ai/test", web::post().to(ai::test))
            .route("/portal/apps", web::get().to(portal::list))
            .route("/portal/apps", web::post().to(portal::create))
            .route("/portal/apps/{id}", web::put().to(portal::update))
            .route("/portal/apps/{id}", web::delete().to(portal::delete))
            .route("/portal/hosts/{id}", web::get().to(portal::host_view))
            .route("/portal/install", web::post().to(portal::request))
            .route("/portal/suggestions", web::get().to(portal::suggestions))
            .route("/portal/brand-icons", web::get().to(portal::brand_icons))
            .route("/compliance", web::get().to(compliance_api::fleet))
            .route("/compliance/profiles", web::get().to(compliance_api::profiles))
            .route("/compliance/profiles", web::post().to(compliance_api::profile_create))
            .route("/compliance/profiles/{id}", web::put().to(compliance_api::profile_update))
            .route("/compliance/profiles/{id}", web::delete().to(compliance_api::profile_delete))
            .route("/compliance/rules", web::get().to(compliance_api::failing_rules))
            .route("/hosts/{id}/compliance", web::get().to(compliance_api::host))
            .route("/hosts/{id}/compliance-scan", web::post().to(compliance_api::scan))
            .route("/mcp", web::post().to(mcp::endpoint))
            .route("/mcp", web::get().to(mcp::no_stream))
            .route("/auth/tokens", web::get().to(mcp::list_tokens))
            .route("/auth/tokens", web::post().to(mcp::create_token))
            .route("/auth/tokens/{id}", web::delete().to(mcp::revoke_token))
            .route("/tasks/{id}/ai-review", web::post().to(ai::review_task))
            .route("/notifications/catalog", web::get().to(notifications::catalog))
            .route("/notifications/channels", web::get().to(notifications::list_channels))
            .route("/notifications/channels", web::post().to(notifications::create_channel))
            .route("/notifications/channels/{id}", web::put().to(notifications::update_channel))
            .route("/notifications/channels/{id}", web::delete().to(notifications::delete_channel))
            .route("/notifications/channels/{id}/test", web::post().to(notifications::test_channel))
            .route("/notifications/rules", web::get().to(notifications::list_rules))
            .route("/notifications/rules", web::post().to(notifications::create_rule))
            .route("/notifications/rules/{id}", web::put().to(notifications::update_rule))
            .route("/notifications/rules/{id}", web::delete().to(notifications::delete_rule))
            .route("/notifications/rules/{id}/digest", web::post().to(notifications::send_digest_now))
            .route("/notifications/history", web::get().to(notifications::history))
            .route("/reports", web::get().to(reports::list))
            .route("/reports", web::post().to(reports::create))
            .route("/reports/preview", web::post().to(reports::preview))
            .route("/reports/{id}", web::put().to(reports::update))
            .route("/reports/{id}", web::delete().to(reports::delete))
            .route("/reports/{id}/send", web::post().to(reports::send_now))
            .route("/reports/{id}/runs", web::get().to(reports::runs))
            // --- fleet
            .route("/hosts", web::get().to(fleet::list_hosts))
            .route("/hosts/{id}", web::get().to(fleet::get_host))
            .route("/hosts/{id}", web::patch().to(fleet::update_host))
            .route("/hosts/{id}", web::delete().to(fleet::delete_host))
            .route("/hosts/{id}/scan", web::post().to(fleet::scan_host))
            .route("/hosts/{id}/quarantine", web::post().to(fleet::quarantine_host))
            .route("/hosts/{id}/release", web::post().to(fleet::release_host))
            .route("/hosts/{id}/reboot", web::post().to(fleet::reboot_host))
            .route("/hosts/{id}/agent-update", web::post().to(fleet::update_agent))
            .route("/docker", web::get().to(docker::list))
            .route("/docker/migration", web::get().to(docker_shell::migration))
            .route("/docker/shell", web::post().to(docker_shell::run))
            .route("/docker/migration-plan", web::post().to(docker_shell::plan))
            .route("/packages", web::get().to(packages::list_packages))
            .route("/packages/{name}/hosts", web::get().to(packages::package_hosts))
            .route("/groups", web::get().to(fleet::list_groups))
            .route("/groups", web::post().to(fleet::create_group))
            .route("/groups/{id}", web::put().to(fleet::update_group))
            .route("/groups/{id}", web::delete().to(fleet::delete_group))
            .route("/enrollment-tokens", web::get().to(fleet::list_tokens))
            .route("/enrollment-tokens", web::post().to(fleet::create_token))
            .route("/enrollment-tokens/{id}", web::delete().to(fleet::revoke_token))
            // --- patching
            .route("/policies", web::get().to(patching::list_policies))
            .route("/policies", web::post().to(patching::create_policy))
            .route("/policies/simulate", web::post().to(patching::simulate))
            .route("/policies/preview-schedule", web::post().to(patching::preview_schedule))
            .route("/hosts/{id}/patching", web::get().to(patching::host_patching))
            .route("/hosts/{id}/metrics", web::get().to(fleet::host_metrics))
            .route("/policies/{id}", web::put().to(patching::update_policy))
            .route("/policies/{id}", web::delete().to(patching::delete_policy))
            .route("/tasks", web::get().to(patching::list_tasks))
            .route("/tasks", web::post().to(patching::create_tasks))
            .route("/tasks/{id}", web::get().to(patching::get_task))
            .route("/tasks/{id}/approve", web::post().to(patching::approve_task))
            .route("/tasks/{id}/reject", web::post().to(patching::reject_task))
            .route("/tasks/{id}/cancel", web::post().to(patching::cancel_task))
            .route("/tasks/{id}/rollback", web::post().to(patching::rollback_task))
            .service(web::resource("/vulnerabilities").app_data(web::JsonConfig::default().limit(BIG_JSON)).route(web::get().to(patching::list_vulns)).route(web::put().to(patching::import_vulns)))
            .route("/waivers", web::get().to(patching::list_waivers))
            .route("/waivers", web::post().to(patching::create_waiver))
            .route("/waivers/{id}/approve", web::post().to(patching::approve_waiver))
            .route("/waivers/{id}/reject", web::post().to(patching::reject_waiver))
            .route("/waivers/{id}/revoke", web::post().to(patching::revoke_waiver))
            // --- audit
            .route("/audit", web::get().to(system::list_audit))
            .route("/audit/verify", web::get().to(system::verify_audit))
            .route("/logs", web::get().to(system::list_logs)),
    );
}

/// A plausible e-mail address: one `@`, no spaces, control characters or angle brackets (used for recipients and
/// profiles; the SMTP server is the real judge).
pub(crate) fn email_ok(v: &str) -> bool {
    let v = v.trim();
    v.len() <= 254 && v.matches('@').count() == 1 && !v.starts_with('@') && !v.ends_with('@') && !v.chars().any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>')
}

/// Shared input validation helpers.
pub(crate) fn one_of(field: &str, v: &str, allowed: &[&str]) -> crate::error::ApiResult<()> {
    if allowed.contains(&v) {
        Ok(())
    } else {
        Err(crate::error::ApiError::bad(format!("{field} must be one of: {}", allowed.join(", "))))
    }
}

pub(crate) fn name_ok(field: &str, v: &str) -> crate::error::ApiResult<()> {
    let t = v.trim();
    if t.is_empty() || t.len() > 120 || t.chars().any(|c| c.is_control()) {
        return Err(crate::error::ApiError::bad(format!("{field} must be 1-120 printable characters")));
    }
    Ok(())
}

/// Heartbeat interval (seconds) configured in settings; falls back to the default on any problem.
pub(crate) fn admin_setting_heartbeat(c: &mut diesel::PgConnection, state: &crate::state::AppState) -> u64 {
    admin::load_setting(c, state, "agent")
        .ok()
        .and_then(|v| v.get("heartbeat_secs").and_then(|h| h.as_u64()))
        .map(|h| h.clamp(10, 3600))
        .unwrap_or(crate::gateway::DEFAULT_HEARTBEAT_SECS)
}
