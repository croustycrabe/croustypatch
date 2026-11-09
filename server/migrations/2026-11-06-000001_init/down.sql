DROP TABLE IF EXISTS audit_log, waivers, task_events, tasks, policy_groups, policies, vulnerabilities,
  host_packages, enrollment_tokens, host_group_members, hosts, host_groups, settings,
  dashboard_layouts, users, roles CASCADE;
DROP FUNCTION IF EXISTS audit_log_immutable();
