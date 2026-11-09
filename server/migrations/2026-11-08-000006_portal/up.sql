-- App center catalog (spec portail_app §3). Native packages only in this release.
CREATE TABLE portal_apps (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  version TEXT NOT NULL DEFAULT '',
  category TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  icon TEXT NOT NULL DEFAULT '',
  packages JSONB NOT NULL DEFAULT '[]',
  group_ids JSONB NOT NULL DEFAULT '[]',     -- host groups that see it; empty = every host
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
