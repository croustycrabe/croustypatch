-- Reports: a summary built from the database at send time (not from queued events), on a schedule, even when
-- nothing happened. Event notifications (rules) stay for "tell me when something occurs".
CREATE TABLE reports (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  enabled BOOLEAN NOT NULL DEFAULT true,
  frequency TEXT NOT NULL DEFAULT 'weekly',      -- daily | weekly | monthly
  weekday INT NOT NULL DEFAULT 0,                -- weekly: 0 = Monday … 6 = Sunday
  day_of_month INT NOT NULL DEFAULT 1,           -- monthly: 1..28
  send_at TEXT NOT NULL DEFAULT '08:00',         -- local time HH:MM
  timezone TEXT NOT NULL DEFAULT 'UTC',          -- IANA zone
  language TEXT NOT NULL DEFAULT 'fr',           -- fr | en
  sections JSONB NOT NULL DEFAULT '{}',          -- what the report contains
  group_ids JSONB NOT NULL DEFAULT '[]',         -- host groups in scope; empty = every host
  emails JSONB NOT NULL DEFAULT '[]',            -- recipients for e-mail channels
  channel_ids JSONB NOT NULL DEFAULT '[]',       -- where to send it (an e-mail provider, ntfy…)
  skip_if_empty BOOLEAN NOT NULL DEFAULT false,
  last_run_at TIMESTAMPTZ,
  last_slot_at TIMESTAMPTZ,                      -- the scheduled moment last handled (success, failure or skip)
  attempts INT NOT NULL DEFAULT 0,               -- tries made for that moment (a failed send is retried)
  last_status TEXT NOT NULL DEFAULT '',          -- sent | failed | skipped
  last_error TEXT NOT NULL DEFAULT '',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE report_runs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  report_id UUID NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
  at TIMESTAMPTZ NOT NULL DEFAULT now(),
  trigger TEXT NOT NULL DEFAULT 'schedule',      -- schedule | manual
  status TEXT NOT NULL,                          -- sent | failed | skipped
  error TEXT NOT NULL DEFAULT '',
  recipients JSONB NOT NULL DEFAULT '[]',
  title TEXT NOT NULL DEFAULT ''
);
CREATE INDEX report_runs_report_idx ON report_runs(report_id, at DESC);
