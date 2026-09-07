ALTER TABLE songs ADD COLUMN is_missing INTEGER NOT NULL DEFAULT 0;

ALTER TABLE watched_folders ADD COLUMN playlist_id INTEGER;

CREATE UNIQUE INDEX idx_watched_folders_playlist
    ON watched_folders(playlist_id)
    WHERE playlist_id IS NOT NULL;
