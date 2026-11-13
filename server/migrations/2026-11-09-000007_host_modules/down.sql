ALTER TABLE portal_apps ADD COLUMN packages JSONB NOT NULL DEFAULT '[]';
UPDATE portal_apps SET packages = COALESCE(recipes->'*'->'packages', recipes->'apt'->'packages', recipes->'pacman'->'packages', '[]');
ALTER TABLE portal_apps DROP COLUMN recipes;
ALTER TABLE enrollment_tokens DROP COLUMN modules;
ALTER TABLE hosts DROP COLUMN modules;
