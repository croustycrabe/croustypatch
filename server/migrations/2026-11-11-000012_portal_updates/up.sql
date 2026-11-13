-- Users of laptops and workstations may start the pending system updates of their own machine from the app center
-- (no approval step: the person at the keyboard decides). Servers keep it off; administrators change it per host.
UPDATE hosts SET modules = jsonb_set(modules, '{portal_updates}', to_jsonb(asset_type IN ('laptop', 'workstation')));
