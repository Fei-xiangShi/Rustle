CREATE TABLE songs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    artist TEXT NOT NULL DEFAULT 'Unknown Artist',
    album TEXT NOT NULL DEFAULT 'Unknown Album',
    duration_secs INTEGER NOT NULL DEFAULT 0,
    track_number INTEGER,
    year INTEGER,
    genre TEXT,
    cover_path TEXT,
    file_hash TEXT,
    file_size INTEGER NOT NULL DEFAULT 0,
    format TEXT,
    normalization_gain REAL,
    play_count INTEGER NOT NULL DEFAULT 0,
    last_played INTEGER,
    last_modified INTEGER NOT NULL,
    is_missing INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_songs_file_path ON songs(file_path);
CREATE INDEX idx_songs_artist ON songs(artist);
CREATE INDEX idx_songs_album ON songs(album);
CREATE INDEX idx_songs_file_hash ON songs(file_hash);

CREATE TABLE playlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT,
    cover_path TEXT,
    is_smart INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE playlist_songs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL,
    song_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    added_at INTEGER NOT NULL,
    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
    FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE,
    UNIQUE(playlist_id, song_id)
);

CREATE INDEX idx_playlist_songs_playlist ON playlist_songs(playlist_id);

CREATE TABLE queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    source_playlist_id INTEGER,
    FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE,
    FOREIGN KEY (source_playlist_id) REFERENCES playlists(id) ON DELETE SET NULL
);

CREATE INDEX idx_queue_position ON queue(position);

CREATE TABLE playback_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    current_song_id INTEGER,
    queue_position INTEGER NOT NULL DEFAULT 0,
    position_secs REAL NOT NULL DEFAULT 0.0,
    volume REAL NOT NULL DEFAULT 1.0,
    shuffle INTEGER NOT NULL DEFAULT 0,
    repeat_mode INTEGER NOT NULL DEFAULT 0,
    personal_fm_mode INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (current_song_id) REFERENCES songs(id) ON DELETE SET NULL
);

INSERT INTO playback_state (
    id,
    queue_position,
    position_secs,
    volume,
    shuffle,
    repeat_mode,
    updated_at
) VALUES (1, 0, 0.0, 1.0, 0, 0, 0);

CREATE TABLE play_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id INTEGER NOT NULL,
    played_at INTEGER NOT NULL,
    listened_secs INTEGER NOT NULL DEFAULT 0,
    completed INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE
);

CREATE INDEX idx_play_history_song ON play_history(song_id);
CREATE INDEX idx_play_history_played_at ON play_history(played_at);

CREATE TABLE watched_folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    playlist_id INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_scanned INTEGER,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_watched_folders_playlist
    ON watched_folders(playlist_id)
    WHERE playlist_id IS NOT NULL;

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
