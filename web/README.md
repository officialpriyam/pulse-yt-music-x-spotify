# Pulse Web

The Next.js web version lives in this folder. It includes an Android-inspired glass layout, a guest mix, search, playlists and saved tracks. Google unlocks the signed-in features.

## Run locally

1. Start the [Rust backend](../backend/README.md) on `127.0.0.1:4000`.
2. Copy `.env.example` to `.env.local` if the local file is absent.
3. Run these commands from the `web` folder.

```sh
npm ci
npm run dev
```

Open `http://localhost:3000`. With the backend in guest mode, you can browse the featured mix and search within it. A configured Google and Postgres backend enables full YouTube search, playlists and liked tracks.

## Deploy on Vercel

1. Create a second Vercel project rooted at **`web`** with the **Next.js** framework preset.
2. Add the environment variable `PULSE_BACKEND_URL` with the Rust service's origin, for example `https://your-pulse-api.vercel.app`.
3. Make sure the backend's `PULSE_PUBLIC_URL` matches the web site's origin and that the web callback is registered in Google Cloud.
4. Deploy the web project. Check that `/api/v1/session` on the web domain returns JSON, then test **Connect Google** and open **Library**.

The Next.js project rewrites `/auth/*` and `/api/v1/*` to the Rust backend. The browser uses the site's origin for OAuth and cookies. Store Google and database secrets on the backend project only.

The web app is for browsing and managing the library. Playback is available in the Android app.
