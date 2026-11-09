-- Notification module: channels (where to send), rules (what, to whom, how often), outbox (each delivery).
CREATE TABLE notification_channels (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL,                         -- smtp | webhook | ntfy | gotify
  config JSONB NOT NULL DEFAULT '{}',         -- non-secret settings (host, url, topic, recipients…)
  secret TEXT NOT NULL DEFAULT '',            -- AES-256-GCM sealed (password, HMAC key, token); never returned
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE notification_rules (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  enabled BOOLEAN NOT NULL DEFAULT true,
  events JSONB NOT NULL DEFAULT '[]',         -- event keys from the catalog
  min_severity TEXT NOT NULL DEFAULT 'high',  -- info | medium | high | critical
  frequency TEXT NOT NULL DEFAULT 'instant',  -- instant | digest
  digest_hour INT NOT NULL DEFAULT 8,         -- UTC hour of the daily digest
  group_ids JSONB NOT NULL DEFAULT '[]',      -- host groups in scope; empty = every host
  channel_ids JSONB NOT NULL DEFAULT '[]',
  roles JSONB NOT NULL DEFAULT '[]',          -- e-mail: users of these roles receive it
  emails JSONB NOT NULL DEFAULT '[]',         -- e-mail: extra addresses
  last_digest_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE notifications (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  rule_id UUID REFERENCES notification_rules(id) ON DELETE CASCADE,
  channel_id UUID NOT NULL REFERENCES notification_channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL,
  severity TEXT NOT NULL,
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  payload JSONB NOT NULL DEFAULT '{}',
  recipients JSONB NOT NULL DEFAULT '[]',
  status TEXT NOT NULL DEFAULT 'pending',     -- pending | digest | sent | failed
  attempts INT NOT NULL DEFAULT 0,
  last_error TEXT NOT NULL DEFAULT '',
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  sent_at TIMESTAMPTZ
);
CREATE INDEX notifications_due_idx ON notifications(status, next_attempt_at);
CREATE INDEX notifications_rule_idx ON notifications(rule_id, status);
