ALTER TABLE pulse_favorites
    DROP CONSTRAINT pulse_favorites_video_id_check,
    ADD CONSTRAINT pulse_favorites_track_id_check
        CHECK (video_id ~ '^[A-Za-z0-9_-]{11}$' OR video_id ~ '^pulse-[a-z0-9-]{6,50}$');

ALTER TABLE pulse_playlist_tracks
    DROP CONSTRAINT pulse_playlist_tracks_track_id_check,
    ADD CONSTRAINT pulse_playlist_tracks_track_id_check
        CHECK (track_id ~ '^[A-Za-z0-9_-]{11}$' OR track_id ~ '^pulse-[a-z0-9-]{6,50}$');
