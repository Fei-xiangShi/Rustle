CREATE TABLE downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id INTEGER NOT NULL,
    ncm_id INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL,
    artist TEXT NOT NULL DEFAULT '',
    file_path TEXT NOT NULL,
    file_size INTEGER NOT NULL DEFAULT 0,
    quality TEXT NOT NULL DEFAULT '',
    downloaded_at INTEGER NOT NULL
);
CREATE INDEX idx_downloads_song_id ON downloads(song_id);
CREATE INDEX idx_downloads_downloaded_at ON downloads(downloaded_at);
INSERT INTO downloads (
    id,
    song_id,
    ncm_id,
    title,
    artist,
    file_path,
    file_size,
    quality,
    downloaded_at
) VALUES (
    1,
    1,
    0,
    'Fixture Song',
    'Fixture Artist',
    'fixture/download.flac',
    1024,
    'lossless',
    100
);
