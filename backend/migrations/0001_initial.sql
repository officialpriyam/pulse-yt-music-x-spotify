CREATE TABLE IF NOT EXISTS pulse_accounts (
    google_sub text PRIMARY KEY,
    display_name text NOT NULL
);

CREATE TABLE IF NOT EXISTS pulse_oauth_states (
    state_hash bytea PRIMARY KEY,
    verifier text NOT NULL,
    expires_at timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS pulse_oauth_expiry ON pulse_oauth_states (expires_at);

CREATE TABLE IF NOT EXISTS pulse_sessions (
    session_hash bytea PRIMARY KEY,
    google_sub text NOT NULL REFERENCES pulse_accounts (google_sub) ON DELETE CASCADE,
    encrypted_tokens bytea NOT NULL,
    expires_at timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS pulse_session_expiry ON pulse_sessions (expires_at);

CREATE TABLE IF NOT EXISTS pulse_favorites (
    google_sub text NOT NULL REFERENCES pulse_accounts (google_sub) ON DELETE CASCADE,
    video_id text NOT NULL CHECK (video_id ~ '^[A-Za-z0-9_-]{11}$'),
    saved_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (google_sub, video_id)
);
CREATE INDEX IF NOT EXISTS pulse_favorites_order ON pulse_favorites (google_sub, saved_at DESC);

CREATE TABLE IF NOT EXISTS pulse_search_usage (
    google_sub text PRIMARY KEY REFERENCES pulse_accounts (google_sub) ON DELETE CASCADE,
    day date NOT NULL,
    requests integer NOT NULL
);
