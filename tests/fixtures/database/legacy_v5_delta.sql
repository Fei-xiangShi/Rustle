ALTER TABLE playback_state
    ADD COLUMN personal_fm_mode INTEGER NOT NULL DEFAULT 0;
UPDATE playback_state SET personal_fm_mode = 1 WHERE id = 1;
