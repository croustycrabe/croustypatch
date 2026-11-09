-- Colour key of a host group (one of a fixed palette, see api::fleet::GROUP_COLORS); empty = automatic from the name.
ALTER TABLE host_groups ADD COLUMN color TEXT NOT NULL DEFAULT '';
