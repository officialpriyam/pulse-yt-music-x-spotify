package dev.priyx.pulse

import dev.priyx.pulse.backend.lyrics.primaryArtist
import dev.priyx.pulse.backend.lyrics.songTitle
import dev.priyx.pulse.backend.youtube.atSize
import dev.priyx.pulse.data.util.SpotifyLinkParser
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class SharedMusicTest {
    @Test
    fun parsesLocalizedSpotifyLinks() {
        assertEquals(
            "spotify:track:4PTG3Z6ehGkBFwjybzWkR8",
            SpotifyLinkParser.parse("https://open.spotify.com/intl-en/track/4PTG3Z6ehGkBFwjybzWkR8?si=123")?.uri,
        )
        assertNull(SpotifyLinkParser.parse("https://example.com/track/4PTG3Z6ehGkBFwjybzWkR8"))
    }

    @Test
    fun normalizesTrackInfoForLyrics() {
        assertEquals("Midnight City", "Midnight City (Official Video)".songTitle())
        assertEquals("M83", "M83 feat. Susanne Sundfør".primaryArtist())
    }

    @Test
    fun keepsCoverSizingConsistent() {
        assertEquals(
            "https://lh3.googleusercontent.com/example=w544-h544-p-l90-rj",
            "https://lh3.googleusercontent.com/example=w60-h60".atSize(544),
        )
    }
}
