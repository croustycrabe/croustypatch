-- FreeBSD is no longer supported: app-center recipes keyed by its package manager ("pkg") go away.
UPDATE portal_apps SET recipes = recipes - 'pkg', updated_at = now() WHERE recipes ? 'pkg';
