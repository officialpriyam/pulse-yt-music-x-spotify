# Pulse external links

Project defaults use the GitHub owner `officialpriyam` and repository `officialpriyam/pulse-yt-music-x-spotify`. The Pulse checkout uses the Git remote `origin` for this project. The original clone source is `upstream`. Original project and contributor links in credits identify the work Pulse builds on.

| Area | Destinations and purpose |
| --- | --- |
| Android updates | `api.github.com` checks releases for the repository in [Updater.kt](../app/src/main/java/dev/priyx/pulse/update/Updater.kt). Download URLs come from release assets. |
| Spotify metadata configuration | `raw.githubusercontent.com` reads `pathfinder.json` from the same GitHub repository. The app has bundled fallback IDs. |
| About and help | GitHub profile and avatar for `officialpriyam`, plus discussions and issues for `officialpriyam/pulse-yt-music-x-spotify`. |
| Release announcements | Telegram posts use explicitly configured repository destinations. The inherited Telegram and Ko-fi links were removed. |
| Lyrics | LRCLIB, Lyrics.ovh, Lossless, Better Lyrics, NetEase, KuGou and optional Spicy Lyrics. Configure an approved Spicy Lyrics origin with the Gradle property `pulseSpicyLyricsOrigin` to enable this provider. |
| Catalog and playback helpers | Apple Music for catalog metadata, SponsorBlock, and Return YouTube Dislike in the vendored InnerTube module. |
| Support and diagnostics | F-Droid links to the optional YTDLnis downloader. The Widevine diagnostics include a test license proxy. |
| Build and project metadata | Gradle and Maven repositories, JitPack, GitHub dependencies and actions, Shields.io badges, and upstream licensing and attribution links. |

The Rust API uses Google OAuth and the YouTube Data API. Its public origin comes from `PULSE_PUBLIC_URL` at runtime.
