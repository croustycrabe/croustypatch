-- The compliance (OpenSCAP) feature is gone: its tables, settings and leftovers go with it.
-- (The audit log keeps its history: it is append-only.)
DROP TRIGGER IF EXISTS compliance_reports_immutable ON compliance_reports;
DROP TABLE IF EXISTS compliance_reports;
DROP FUNCTION IF EXISTS compliance_no_update();
DROP TABLE IF EXISTS compliance_profiles;
ALTER TABLE hosts DROP COLUMN IF EXISTS compliance_profile;
DELETE FROM tasks WHERE kind = 'compliance';
DELETE FROM settings WHERE key = 'compliance';
UPDATE settings SET value = value - 'openscap' WHERE key = 'modules';
UPDATE hosts SET modules = modules - 'openscap' WHERE modules ? 'openscap';
UPDATE enrollment_tokens SET modules = modules - 'openscap' WHERE modules ? 'openscap';
UPDATE notification_rules SET events = COALESCE((SELECT jsonb_agg(e) FROM jsonb_array_elements(events) e WHERE e #>> '{}' NOT LIKE 'compliance.%'), '[]'::jsonb) WHERE events::text LIKE '%compliance.%';
