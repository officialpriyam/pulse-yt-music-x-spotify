ALTER TABLE pulse_sessions ADD COLUMN mobile boolean NOT NULL DEFAULT false;

CREATE TABLE pulse_android_oauth (
    state_hash bytea PRIMARY KEY,
    challenge bytea NOT NULL,
    google_verifier text NOT NULL,
    account_id text REFERENCES pulse_accounts (account_id) ON DELETE CASCADE,
    denied boolean NOT NULL DEFAULT false,
    issued boolean NOT NULL DEFAULT false,
    expires_at timestamptz NOT NULL
);
CREATE INDEX pulse_android_oauth_expiry ON pulse_android_oauth (expires_at);

CREATE OR REPLACE FUNCTION pulse_prune_expired() RETURNS bigint
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    WITH expired_states AS (
        SELECT state_hash FROM public.pulse_oauth_states
        WHERE expires_at <= now() ORDER BY expires_at LIMIT 1000 FOR UPDATE SKIP LOCKED
    ), removed_states AS (
        DELETE FROM public.pulse_oauth_states p USING expired_states e
        WHERE p.state_hash = e.state_hash RETURNING 1
    ), expired_android AS (
        SELECT state_hash FROM public.pulse_android_oauth
        WHERE expires_at <= now() ORDER BY expires_at LIMIT 1000 FOR UPDATE SKIP LOCKED
    ), removed_android AS (
        DELETE FROM public.pulse_android_oauth p USING expired_android e
        WHERE p.state_hash = e.state_hash RETURNING 1
    ), expired_sessions AS (
        SELECT session_hash FROM public.pulse_sessions
        WHERE expires_at <= now() ORDER BY expires_at LIMIT 1000 FOR UPDATE SKIP LOCKED
    ), removed_sessions AS (
        DELETE FROM public.pulse_sessions p USING expired_sessions e
        WHERE p.session_hash = e.session_hash RETURNING 1
    )
    SELECT (SELECT count(*) FROM removed_states) + (SELECT count(*) FROM removed_android) + (SELECT count(*) FROM removed_sessions)
$$;
