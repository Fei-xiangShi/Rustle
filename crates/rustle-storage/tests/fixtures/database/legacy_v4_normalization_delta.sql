ALTER TABLE songs ADD COLUMN normalization_updated_at INTEGER;
UPDATE songs SET normalization_updated_at = 1700000000 WHERE id = 1;
