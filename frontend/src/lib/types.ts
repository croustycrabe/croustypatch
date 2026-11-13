// Shapes returned by the CroustyPatch server (snake_case, as sent).
export type Lang = 'fr' | 'en';

export interface Me {
  id: string;
  username: string;
  role: string;
  permissions: string[];
  language: Lang;
  must_change_password: boolean;
  email?: string | null;
}

export interface Host {
  id: string;
  hostname: string;
  display_name: string | null;
  status: HostStatus;
  facts: Record<string, unknown>;
  os_family: string;
  distro: string;
  package_manager: string;
  asset_type: 'server' | 'workstation' | 'laptop';
  environment: 'production' | 'staging' | 'dev';
  criticality: 'low' | 'medium' | 'high';
  internet_exposed: boolean;
  has_staging: boolean;
  ring: number;
  tags: string[];
  comm_mode: 'push' | 'pull';
  connected: boolean;
  eco: boolean;
  reboot_required: boolean;
  downtime_until: string | null;
  heartbeat_secs: number;
  last_seen_at: string | null;
  last_inventory_at: string | null;
  enrolled_at: string;
  modules: HostModules;
  metrics?: HostMetrics['current'];
  metrics_at?: string | null;
  /** Audit profile of this host: '' (fleet default), an XCCDF profile id or `custom:<uuid>`. */
  compliance_profile?: string;
}

/** Optional features switched on per host. */
export interface HostModules {
  docker: boolean;
  helper_script: boolean;
  openscap: boolean;
  portal: boolean;
  portal_updates: boolean;
  eco: boolean;
}

export type HostStatus = 'online' | 'offline' | 'eco' | 'maintenance' | 'alert' | 'quarantined' | 'pending';

export interface HostSummary extends Host {
  group_ids: string[];
  /** Pending updates of every kind; of those, `docker_updates` are container images (the system ones are the rest). */
  updates: number;
  security_updates: number;
  docker_updates: number;
}

export interface HostPackage {
  name: string;
  source: string;
  version: string;
  candidate: string | null;
  is_security: boolean;
  cves: string[];
}

export interface HostGroup {
  id: string;
  name: string;
  description: string;
  color?: string;
  host_count?: number;
}

export type TaskStatus =
  | 'pending_approval' | 'queued' | 'sent' | 'running' | 'soaking' | 'succeeded' | 'failed'
  | 'rolling_back' | 'rolled_back' | 'quarantined' | 'blocked' | 'rejected' | 'cancelled' | 'expired';

export interface TaskPackage {
  name: string;
  from: string | null;
  to: string | null;
  cves?: string[];
  is_security?: boolean;
}

export interface Assessment {
  engine: string;
  contextual_risk: Risk;
  stability_risk: Risk;
  matched_rule: string | null;
  rationale: string[];
  action: PolicyAction;
  matched_decision: number | null;
}

export type Risk = 'Low' | 'Medium' | 'High' | 'Critical';
export type PolicyAction = 'AUTO_DEPLOY' | 'REQUIRE_APPROVAL' | 'BLOCK';

export interface Task {
  id: string;
  host_id: string;
  hostname: string;
  kind: 'scan' | 'patch' | 'rollback' | 'quarantine' | 'release' | 'reboot' | 'compliance' | 'app' | 'agent_update' | 'docker';
  status: TaskStatus;
  policy_id: string | null;
  parent_task_id: string | null;
  packages: TaskPackage[];
  decision: { policy_name?: string; assessment?: Assessment } | null;
  exec: { soak_minutes?: number; snapshot_required?: boolean } | null;
  result: { changes?: { name: string; from: string | null; to: string | null }[]; reboot_required?: boolean; log_tail?: string[] } | null;
  message: string;
  requested_by: string;
  approved_by: string | null;
  scheduled_for: string;
  sent_at: string | null;
  started_at: string | null;
  finished_at: string | null;
  soak_until: string | null;
  created_at: string;
  updated_at: string;
}

export interface TaskEvent {
  id: number;
  ts: string;
  status: string;
  message: string;
}

export interface Probe {
  type: 'tcp' | 'http' | 'systemd';
  host?: string;
  port?: number;
  url?: string;
  expect_status?: number;
  unit?: string;
}

/** A patch day: when updates may run. `timezone` (IANA) replaces the fixed offset when set. */
export interface MaintenanceWindow {
  days: number[];
  start: string;
  end: string;
  utc_offset_minutes: number;
  timezone?: string | null;
  repeat?: 'weekly' | 'monthly_weekday' | 'monthly_day';
  nth?: number | null;
  day_of_month?: number | null;
}

export interface Occurrence { start: string; end: string }

/** GET /docker: every container of every host whose Docker module is on. */
export interface DockerData {
  summary: { containers: number; with_updates: number; with_newer_version: number; hosts: number };
  containers: {
    host_id: string; hostname: string; name: string; package: string; image: string; current: string; candidate: string | null; update: boolean;
    project: string | null; service: string | null; state: string | null; health: string | null; started_at: string | null; open_task: { id: string; status: string } | null;
    /** A newer version tag in the registry (informative: moving to it means changing the compose file). */
    latest: { tag: string; major: boolean; same_major: string | null; apply?: { mode: 'file' | 'manual'; file?: string; reason?: string; already?: boolean } } | null;
    /** What the agent's registry checks found: `digest` ok / pinned / offline / error: …; `tags` listed / probed / not_version / pinned / offline / error: … */
    check?: { digest: string; tags: string } | null;
  }[];
}

/** Latest resource reading of a host and its history (GET /hosts/{id}/metrics). All fields are absent for agents before 0.6. */
export interface HostMetrics {
  at: string | null;
  current: {
    cpu_pct?: number | null; mem_total_mb?: number; mem_used_mb?: number; mem_pct?: number | null; swap_total_mb?: number; swap_used_mb?: number;
    load_1m?: number; load_5m?: number; load_15m?: number; uptime_secs?: number; disks?: { mount: string; fs: string; total_mb: number; used_mb: number }[];
  };
  points: { at: string; cpu: number | null; mem: number | null; load: number | null }[];
}

/** Why a host is (not) being patched automatically (GET /hosts/{id}/patching). */
export interface PatchingExplain {
  state: 'host_inactive' | 'task_open' | 'no_group' | 'no_policy' | 'out_of_scope' | 'nothing_pending' | 'nothing_matches' | 'already_handled' | 'waiting_for_window' | 'will_plan' | 'needs_approval' | 'blocked';
  pending: { total: number; security: number };
  groups: string[];
  policies: { id: string; name: string; in_scope: boolean; security_only: boolean; matching: number; not_security: number; not_docker?: number; docker?: DockerMode; waived: number; ai: boolean; window: MaintenanceWindow | null }[];
  chosen: string | null;
  action: PolicyAction | null;
  window: { open: boolean; current: Occurrence | null; next: Occurrence[]; config: MaintenanceWindow } | null;
  open_task: { id: string; kind: string; status: string; scheduled_for: string } | null;
}

export type DockerMode = 'include' | 'exclude' | 'only';

export interface PolicyDef {
  /** Container image updates: with the system updates, left out, or the only thing this policy updates. */
  docker?: DockerMode;
  ai_mode_enabled: boolean;
  target_scope: string;
  security_only: boolean;
  rules: { rule_id: string; condition: string; contextual_risk: Risk }[];
  decisions: {
    conditions: { contextual_risk?: Risk | null; stability_risk?: Risk | null; contextual_risk_min?: Risk | null; stability_risk_max?: Risk | null };
    action: PolicyAction;
  }[];
  default_action: PolicyAction;
  maintenance_window: MaintenanceWindow | null;
  rollback: { soak_minutes: number; probes: Probe[]; auto_rollback: boolean; snapshot_required: boolean };
  reboot: { mode: string };
  rings: { delay_minutes: number };
}

export interface Policy {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  priority: number;
  definition: PolicyDef;
  group_ids: string[];
  /** Hosts in the policy's groups. */
  hosts?: number;
  in_window?: boolean;
  next_runs?: Occurrence[];
}

export interface Vulnerability {
  vulnerability: { cve_id: string; cvss: number | null; severity: string; summary: string; exploited: boolean; published_at: string | null; source: string };
  exposed_hosts: number;
}

export interface Waiver {
  id: string;
  host_id: string | null;
  group_id: string | null;
  cve_id: string | null;
  package: string | null;
  reason: string;
  compensating_controls: string;
  requested_by: string;
  expires_at: string;
  status: 'pending' | 'approved' | 'rejected' | 'revoked' | 'expired';
  ciso_approved_by: string | null;
  business_approved_by: string | null;
  created_at: string;
}

export interface AuditEntry {
  seq: number;
  ts: string;
  actor: string;
  action: string;
  target_type: string;
  target_id: string;
  host_id: string | null;
  details: Record<string, unknown>;
  ip: string;
  hash: string;
}

export interface AuditVerification {
  ok: boolean;
  entries: number;
  first_bad_seq: number | null;
  head_hash: string;
}

export interface EnrollmentToken {
  id: string;
  label: string;
  group_id: string | null;
  environment: string;
  comm_mode: string;
  expires_at: string;
  max_uses: number;
  used_count: number;
  revoked: boolean;
  created_by: string;
  created_at: string;
}

export interface UserRow {
  id: string;
  username: string;
  display_name: string;
  email: string | null;
  role_id: string;
  auth_source: string;
  is_fallback_admin: boolean;
  disabled: boolean;
  must_change_password: boolean;
  last_login_at: string | null;
}

export interface Role {
  id: string;
  name: string;
  description: string;
  permissions: string[];
  builtin: boolean;
}

export interface Modules {
  docker: boolean;
  helper_script: boolean;
  openscap: boolean;
  portal: boolean;
  notifications: boolean;
  eco: boolean;
}

export interface Dashboard {
  fleet: { total: number; by_status: Record<string, number>; reboot_required: number; hosts_needing_updates: number; os: { name: string; count: number }[] };
  vulnerabilities: { hosts_with_security_updates: number; security_updates: number; critical_cves_unpatched: number; outdated_packages: number };
  top_outdated: { name: string; hosts: number; security: boolean }[];
  tasks: { awaiting_approval: number; in_progress: number; waivers_pending: number };
  activity: { id: string; host_id: string; hostname: string; kind: string; status: TaskStatus; message: string; ts: string }[];
  compliance: { enabled: boolean; score: number | null };
  portal: { enabled: boolean; installs: number | null };
  resources: { reporting: number; avg_cpu: number | null; avg_mem: number | null; busiest: { id: string; hostname: string; cpu: number | null; mem: number | null; disk: number | null }[] };
  patching: {
    next: { policy: string; start: string; end: string; hosts: number; auto: boolean; docker?: string }[];
    week: { succeeded: number; failed: number; rolled_back: number; missed: number };
    uncovered_hosts: number;
  };
}

export interface Layout {
  widgets: { id: string; visible: boolean; size: 'xs' | 'sm' | 'md' | 'lg' }[];
}

export interface Settings {
  modules: Modules;
  ai: { enabled: boolean; provider: string; base_url: string; model: string; api_key: string; api_key_set?: boolean };
  connectivity: { mode: 'connected' | 'air_gapped' };
  agent: { heartbeat_secs: number };
  compliance: { threshold: number; profile: string; auto_install: boolean; schedule?: AuditSchedule };
  /** What stays reachable while a host is isolated, besides the server. */
  isolation: { keep_ssh: boolean; ssh_port: number; ssh_sources: string[] };
}

export interface PackageRow {
  name: string;
  source: string;
  hosts: number;
  outdated: number;
  security: boolean;
  latest: string | null;
}

export interface PackageHost {
  host_id: string;
  hostname: string;
  version: string;
  candidate: string | null;
  is_security: boolean;
  cves: string[];
}

export interface ScapProfile { id: string; title: string }
/** What the agent found: oscap version, the SCAP content it uses and that content's profiles. */
/** `recommended`: the profile used when none is chosen (agents 0.8+); `adapted`: "noble → resolute" when the content of another release is pointed at this one. */
export interface OpenScapInfo { version: string; datastream: string; profiles: ScapProfile[]; recommended?: string; adapted?: string }
export interface CompReport { id: string; profile: string; passed: number; failed: number; not_applicable: number; score: number; results_sha256: string; collected_at: string; failed_rules?: { rule_id: string; severity: string }[] }
export interface CompHost { reports: CompReport[]; openscap: OpenScapInfo | null; module: boolean; profile: string; effective_profile: string; auto_install: boolean; package_manager?: string | null }
export interface CompProfile { id: string; name: string; description: string; base_profile: string; disabled_rules: string[]; used_by: number; created_at: string; updated_at: string }
export interface CompProfiles { custom: CompProfile[]; available: { id: string; title: string; hosts: number }[]; hosts_with_openscap: number; default: string }
export interface FailingRule { rule_id: string; severity: string; hosts: number }
/** Planned audits of the fleet (`frequency` off: none); weekday 0 = Monday. */
export interface AuditSchedule { frequency: 'off' | 'daily' | 'weekly' | 'monthly'; weekday: number; day_of_month: number; at: string; timezone: string }
export interface CompFleet { score: number | null; threshold: number; planned_next?: string | null; hosts: (CompReport & { host_id: string; hostname: string | null })[] }

/** A page of rows with the total behind it (`page` is 1-based, as in the API). */
export interface Paged<T> { items: T[]; total: number; page: number; per_page: number }

/** A line of Administration > Logs. */
export interface LogEntry { ts: string; source: 'tasks' | 'notifications' | 'reports' | 'server'; level: 'info' | 'warn' | 'error'; title: string; message: string; host_id?: string; hostname?: string; ref_id?: string }
