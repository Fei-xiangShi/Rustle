-- Unreleased development builds added this column on top of released V4.
-- No released code reads it; drop it so the schema is exactly V4 again.
ALTER TABLE songs DROP COLUMN normalization_updated_at;
