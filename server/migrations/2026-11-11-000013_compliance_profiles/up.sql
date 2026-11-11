-- Audit profile per host ('' = the fleet default from Settings), and custom profiles: a profile of the SCAP datastream
-- with some rules switched off (XCCDF tailoring), shared by any number of hosts.
ALTER TABLE hosts ADD COLUMN compliance_profile TEXT NOT NULL DEFAULT '';

CREATE TABLE compliance_profiles (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    base_profile  TEXT NOT NULL,
    disabled_rules JSONB NOT NULL DEFAULT '[]',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX compliance_profiles_name_key ON compliance_profiles (lower(name));
