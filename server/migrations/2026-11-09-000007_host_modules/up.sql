-- Optional modules are switched on per host (docker, helper scripts, OpenSCAP, app center, eco mode). The global
-- "modules" setting becomes the default for new hosts. Every existing host starts with today's global switches, so
-- this migration changes nothing until an administrator edits a host.
ALTER TABLE hosts ADD COLUMN modules JSONB NOT NULL DEFAULT '{}';
UPDATE hosts SET modules = jsonb_build_object(
  'docker',        COALESCE((SELECT (value->>'docker')::boolean        FROM settings WHERE key = 'modules'), false),
  'helper_script', COALESCE((SELECT (value->>'helper_script')::boolean FROM settings WHERE key = 'modules'), false),
  'openscap',      COALESCE((SELECT (value->>'openscap')::boolean      FROM settings WHERE key = 'modules'), false),
  'portal',        COALESCE((SELECT (value->>'portal')::boolean        FROM settings WHERE key = 'modules'), false),
  'eco',           COALESCE((SELECT (value->>'eco')::boolean           FROM settings WHERE key = 'modules'), true)
);

-- Modules chosen in the "Add agent" wizard; NULL = the defaults at enrolment time.
ALTER TABLE enrollment_tokens ADD COLUMN modules JSONB;

-- App center recipes per package manager: {"apt": {"packages": [...]}, "pacman": {...}, "*": {"command": {...}}}.
-- Existing apps keep installing the same package names everywhere ("*").
ALTER TABLE portal_apps ADD COLUMN recipes JSONB NOT NULL DEFAULT '{}';
UPDATE portal_apps SET recipes = jsonb_build_object('*', jsonb_build_object('packages', packages));
ALTER TABLE portal_apps DROP COLUMN packages;
