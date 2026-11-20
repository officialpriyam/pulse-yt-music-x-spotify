ALTER TABLE pulse_accounts RENAME COLUMN google_sub TO account_id;
ALTER TABLE pulse_sessions RENAME COLUMN google_sub TO account_id;
ALTER TABLE pulse_favorites RENAME COLUMN google_sub TO account_id;
ALTER TABLE pulse_search_usage RENAME COLUMN google_sub TO account_id;

ALTER TABLE pulse_accounts
    ADD COLUMN google_subject text UNIQUE,
    ADD COLUMN discord_subject text UNIQUE,
    ADD COLUMN username text UNIQUE,
    ADD COLUMN password_hash text;
UPDATE pulse_accounts SET google_subject = account_id;

ALTER TABLE pulse_oauth_states
    ADD COLUMN flow text NOT NULL DEFAULT 'legacy',
    ADD COLUMN session_hash bytea;

CREATE TABLE pulse_youtube_connections (
    account_id text PRIMARY KEY REFERENCES pulse_accounts (account_id) ON DELETE CASCADE,
    encrypted_tokens bytea NOT NULL
);

ALTER TABLE pulse_favorites
    ADD COLUMN title text,
    ADD COLUMN artist text;

CREATE TABLE pulse_spotify_connections (
    account_id text PRIMARY KEY REFERENCES pulse_accounts (account_id) ON DELETE CASCADE,
    encrypted_tokens bytea NOT NULL
);

CREATE TABLE pulse_playlists (
    id text PRIMARY KEY,
    account_id text NOT NULL REFERENCES pulse_accounts (account_id) ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX pulse_playlists_owner ON pulse_playlists (account_id, created_at DESC);

CREATE TABLE pulse_playlist_tracks (
    playlist_id text NOT NULL REFERENCES pulse_playlists (id) ON DELETE CASCADE,
    track_id text NOT NULL CHECK (track_id ~ '^[A-Za-z0-9_-]{11}$'),
    title text NOT NULL,
    artist text NOT NULL,
    added_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (playlist_id, track_id)
);
CREATE INDEX pulse_playlist_track_order ON pulse_playlist_tracks (playlist_id, added_at, track_id);
