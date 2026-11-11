-- Extra facts about an inventory entry; for containers: image, compose stack and service, state, health, start time.
ALTER TABLE host_packages ADD COLUMN meta JSONB NOT NULL DEFAULT '{}';
