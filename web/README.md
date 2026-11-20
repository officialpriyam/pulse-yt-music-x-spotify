# Pulse Web

Pulse Web is the responsive Next.js client. It includes Home, charts, radio stations, Search, liked tracks, Pulse playlists, YouTube playlists, Settings, sign-up, sign-in and a Spotify audio player.

## Run locally

1. Start the [Rust backend](../backend/README.md) and apply its migrations.
2. Copy `.env.example` to `.env.local` and set `PULSE_BACKEND_URL` if the API runs on a different origin.
3. Run `npm ci` and `npm run dev` from this directory. Open http://localhost:3000.

Guests can play the bundled, licensed music from Home, Search, Radio or Player. Playback continues while navigating. [Music credits](public/audio/CREDITS.txt). Username and password accounts use Neon and Redis. Google, Discord, YouTube, and Spotify appear as available options once their backend OAuth client is configured. Google and Discord sign-in can create accounts. To add a provider to an existing account, sign in first and use **Settings → Connections**.

## Vercel

1. Create a second Vercel project for this repository with root directory **web** and framework preset **Next.js**.
2. Set the project's Production variable `PULSE_BACKEND_URL` to `https://pulse-yt-music-x-spotify.vercel.app` or your Rust API's production origin.
3. Set `PULSE_PUBLIC_URL` on the **backend project** to the web app's HTTPS origin. Redeploy the backend after saving it. The browser uses the web domain for credentials, OAuth redirects, and cookies.
4. Deploy the web project. Open `/api/v1/session` on the web domain to verify it returns JSON. Create a Pulse account and make a playlist as a smoke test.

Keep database credentials, Redis credentials, OAuth secrets, and the token encryption key on the backend project. The frontend proxies `/auth/*` and `/api/v1/*`.

The separate player at `/player/spotify` uses the official Spotify Web Playback SDK and needs Spotify Premium, protected media support, and access to your Spotify developer app. YouTube catalog and playlist pages display track information without a video player.
