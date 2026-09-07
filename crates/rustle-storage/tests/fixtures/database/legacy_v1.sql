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
    play_count INTEGER NOT NULL DEFAULT 0,
    last_played INTEGER,
    last_modified INTEGER NOT NULL,
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
    enabled INTEGER NOT NULL DEFAULT 1,
    last_scanned INTEGER,
    created_at INTEGER NOT NULL
);

INSERT INTO songs (
    id,
    file_path,
    title,
    artist,
    album,
    duration_secs,
    file_size,
    format,
    play_count,
    last_modified,
    created_at
) VALUES (
    1,
    'fixture/song.flac',
    'Fixture Song',
    'Fixture Artist',
    'Fixture Album',
    180,
    1024,
    'flac',
    2,
    100,
    100
);
INSERT INTO playlists (
    id, name, description, is_smart, created_at, updated_at
) VALUES (
    1, 'Fixture Playlist', 'Synthetic fixture only', 0, 100, 100
);
INSERT INTO playlist_songs (
    id, playlist_id, song_id, position, added_at
) VALUES (1, 1, 1, 0, 100);
INSERT INTO queue (
    id, song_id, position, source_playlist_id
) VALUES (1, 1, 0, 1);
UPDATE playback_state SET
    current_song_id = 1,
    queue_position = 0,
    position_secs = 12.5,
    volume = 0.75,
    updated_at = 100
WHERE id = 1;
INSERT INTO play_history (
    id, song_id, played_at, listened_secs, completed
) VALUES (1, 1, 100, 90, 0);
INSERT INTO watched_folders (
    id, path, enabled, last_scanned, created_at
) VALUES (1, 'fixture/library', 1, 100, 100);
