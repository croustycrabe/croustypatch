-- CroustyPatch schema, milestone 1.

CREATE TABLE roles (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  -- ["hosts:read", "tasks:write", "*"]; module:action, with * wildcards
  permissions JSONB NOT NULL DEFAULT '[]',
  builtin BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE users (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  username TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL DEFAULT '',
  email TEXT,
  password_hash TEXT,                       -- Argon2id PHC string; NULL for SSO users
  role_id UUID NOT NULL REFERENCES roles(id),
  auth_source TEXT NOT NULL DEFAULT 'local',
  is_fallback_admin BOOLEAN NOT NULL DEFAULT false,
  disabled BOOLEAN NOT NULL DEFAULT false,
  must_change_password BOOLEAN NOT NULL DEFAULT false,
  failed_logins INT NOT NULL DEFAULT 0,
  locked_until TIMESTAMPTZ,
  language TEXT NOT NULL DEFAULT 'fr',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_login_at TIMESTAMPTZ
);

CREATE TABLE dashboard_layouts (
  user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  layout JSONB NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value JSONB NOT NULL,
  encrypted BOOLEAN NOT NULL DEFAULT false,   -- value = {"ct": "<base64 AES-256-GCM>"}
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE host_groups (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE hosts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  hostname TEXT NOT NULL,
  display_name TEXT,
  status TEXT NOT NULL DEFAULT 'pending',
  public_key_pem TEXT NOT NULL,
  facts JSONB NOT NULL DEFAULT '{}',
  os_family TEXT NOT NULL DEFAULT 'linux',
  distro TEXT NOT NULL DEFAULT '',
  package_manager TEXT NOT NULL DEFAULT 'unknown',
  asset_type TEXT NOT NULL DEFAULT 'server',        -- server | workstation | laptop
  environment TEXT NOT NULL DEFAULT 'production',   -- production | staging | dev
  criticality TEXT NOT NULL DEFAULT 'medium',       -- low | medium | high
  internet_exposed BOOLEAN NOT NULL DEFAULT false,
  has_staging BOOLEAN NOT NULL DEFAULT false,
  ring INT NOT NULL DEFAULT 0,
  tags JSONB NOT NULL DEFAULT '[]',
  comm_mode TEXT NOT NULL DEFAULT 'push',           -- push | pull
  connected BOOLEAN NOT NULL DEFAULT false,
  eco BOOLEAN NOT NULL DEFAULT false,
  reboot_required BOOLEAN NOT NULL DEFAULT false,
  downtime_until TIMESTAMPTZ,
  heartbeat_secs INT NOT NULL DEFAULT 60,
  last_seen_at TIMESTAMPTZ,
  last_inventory_at TIMESTAMPTZ,
  enrolled_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX hosts_status_idx ON hosts(status);

CREATE TABLE host_group_members (
  host_id UUID NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  group_id UUID NOT NULL REFERENCES host_groups(id) ON DELETE CASCADE,
  PRIMARY KEY (host_id, group_id)
);

CREATE TABLE enrollment_tokens (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  token_hash TEXT NOT NULL UNIQUE,                  -- SHA-256 of the token; the token is shown once
  label TEXT NOT NULL DEFAULT '',
  group_id UUID REFERENCES host_groups(id) ON DELETE SET NULL,
  environment TEXT NOT NULL DEFAULT 'production',
  comm_mode TEXT NOT NULL DEFAULT 'push',
  expires_at TIMESTAMPTZ NOT NULL,
  max_uses INT NOT NULL DEFAULT 1,
  used_count INT NOT NULL DEFAULT 0,
  revoked BOOLEAN NOT NULL DEFAULT false,
  created_by TEXT NOT NULL DEFAULT '',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE host_packages (
  host_id UUID NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT 'system',
  version TEXT NOT NULL,
  candidate TEXT,
  is_security BOOLEAN NOT NULL DEFAULT false,
  cves JSONB NOT NULL DEFAULT '[]',
  PRIMARY KEY (host_id, name, source)
);
CREATE INDEX host_packages_candidate_idx ON host_packages(host_id) WHERE candidate IS NOT NULL;

CREATE TABLE vulnerabilities (
  cve_id TEXT PRIMARY KEY,
  cvss DOUBLE PRECISION,
  severity TEXT NOT NULL DEFAULT 'unknown',
  summary TEXT NOT NULL DEFAULT '',
  exploited BOOLEAN NOT NULL DEFAULT false,
  published_at TIMESTAMPTZ,
  source TEXT NOT NULL DEFAULT 'manual',
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE policies (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  enabled BOOLEAN NOT NULL DEFAULT true,
  priority INT NOT NULL DEFAULT 100,                -- lower = evaluated first
  definition JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE policy_groups (
  policy_id UUID NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
  group_id UUID NOT NULL REFERENCES host_groups(id) ON DELETE CASCADE,
  PRIMARY KEY (policy_id, group_id)
);

CREATE TABLE tasks (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  host_id UUID NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  policy_id UUID REFERENCES policies(id) ON DELETE SET NULL,
  parent_task_id UUID,
  packages JSONB NOT NULL DEFAULT '[]',
  decision JSONB,
  exec JSONB,
  result JSONB,
  message TEXT NOT NULL DEFAULT '',
  requested_by TEXT NOT NULL DEFAULT '',
  approved_by TEXT,
  scheduled_for TIMESTAMPTZ NOT NULL DEFAULT now(),
  sent_at TIMESTAMPTZ,
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ,
  soak_until TIMESTAMPTZ,
  batch_key TEXT,                                   -- dedupe key for policy-created tasks
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX tasks_host_idx ON tasks(host_id, created_at DESC);
CREATE INDEX tasks_status_idx ON tasks(status);
CREATE INDEX tasks_batch_idx ON tasks(host_id, batch_key);

CREATE TABLE task_events (
  id BIGSERIAL PRIMARY KEY,
  task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  ts TIMESTAMPTZ NOT NULL DEFAULT now(),
  status TEXT NOT NULL,
  message TEXT NOT NULL DEFAULT ''
);
CREATE INDEX task_events_task_idx ON task_events(task_id, id);

CREATE TABLE waivers (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  host_id UUID REFERENCES hosts(id) ON DELETE CASCADE,
  group_id UUID REFERENCES host_groups(id) ON DELETE CASCADE,
  cve_id TEXT,
  package TEXT,
  reason TEXT NOT NULL,
  compensating_controls TEXT NOT NULL,
  requested_by TEXT NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',           -- pending | approved | rejected | revoked
  ciso_approved_by TEXT,
  ciso_approved_at TIMESTAMPTZ,
  business_approved_by TEXT,
  business_approved_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Append-only, hash-chained audit log (WORM semantics enforced by trigger).
CREATE TABLE audit_log (
  seq BIGSERIAL PRIMARY KEY,
  ts TIMESTAMPTZ NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  target_type TEXT NOT NULL DEFAULT '',
  target_id TEXT NOT NULL DEFAULT '',
  host_id UUID,
  details JSONB NOT NULL DEFAULT '{}',
  ip TEXT NOT NULL DEFAULT '',
  prev_hash TEXT NOT NULL,
  hash TEXT NOT NULL
);
CREATE INDEX audit_host_idx ON audit_log(host_id, seq DESC);

CREATE FUNCTION audit_log_immutable() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'audit_log is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_log_no_update BEFORE UPDATE OR DELETE ON audit_log
  FOR EACH ROW EXECUTE FUNCTION audit_log_immutable();
CREATE TRIGGER audit_log_no_truncate BEFORE TRUNCATE ON audit_log
  FOR EACH STATEMENT EXECUTE FUNCTION audit_log_immutable();
