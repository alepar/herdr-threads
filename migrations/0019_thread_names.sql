-- Optional execution names; duplicates remain valid.
ALTER TABLE threads ADD COLUMN name TEXT;
CREATE INDEX threads_instance_name ON threads(instance_id, name);
