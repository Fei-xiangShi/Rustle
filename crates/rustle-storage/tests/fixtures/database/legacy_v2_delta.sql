ALTER TABLE songs ADD COLUMN normalization_gain REAL;
UPDATE songs SET normalization_gain = 0.75 WHERE id = 1;
