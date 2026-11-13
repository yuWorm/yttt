-- The placement each Host signed: the node it chose and the address its
-- devices dial. Only a newer signed placement moves a Host.
ALTER TABLE devices ADD COLUMN placement TEXT;
ALTER TABLE devices ADD COLUMN placement_seq BIGINT NOT NULL DEFAULT 0;
