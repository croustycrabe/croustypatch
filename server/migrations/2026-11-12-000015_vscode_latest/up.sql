-- Catalog entries are copies of a suggestion at the time they were added. Visual Studio Code added before 0.7.5 builds
-- Microsoft's package from the AUR but does not say where its latest version is published, so the app center never
-- offers its updates. Add the source to those entries (and only those).
UPDATE portal_apps
SET recipes = jsonb_set(recipes, '{pacman,command,latest}', '{"aur": "visual-studio-code-bin"}'::jsonb), updated_at = now()
WHERE recipes->'pacman'->'command'->>'install' LIKE '%visual-studio-code-bin%'
  AND NOT (recipes->'pacman'->'command' ? 'latest');
