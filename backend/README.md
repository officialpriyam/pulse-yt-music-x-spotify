# Pulse API

The Rust Axum service handles Google OAuth, YouTube search and playlists, and Pulse liked tracks. It runs as a Vercel Function or as a local HTTP server.

## Prepare Google and Postgres

1. Provision a Postgres database, for example Neon through the Vercel Marketplace. Run [schema.sql](schema.sql) in its SQL editor. Use a pooled connection string with TLS for Vercel.
2. Enable **YouTube Data API v3** in Google Cloud. Configure the OAuth consent screen and add your Google account as a test user while the app is in testing.
3. Create an OAuth client with type **Web application**. Register `http://localhost:3000/auth/google/callback` for local development and `https://YOUR-WEB-DOMAIN/auth/google/callback` for production. Use the **Next.js site domain**, since the browser talks to the Rust API through the site.
4. Generate a separate encryption key for each environment. Keep the key stable between deployments so active sessions can still be decrypted.

```sh
node -e "console.log(require('crypto').randomBytes(32).toString('base64'))"
```

## Environment variables

Copy [.env.example](.env.example) to `.env` for local development, or edit the local `.env` if it is already present. Fill in the credentials privately.

| Variable | Value |
| --- | --- |
| `GOOGLE_CLIENT_ID` | Google web OAuth client ID |
| `GOOGLE_CLIENT_SECRET` | Google web OAuth secret |
| `DATABASE_URL_POOLED` | Neon pooled Postgres connection string. The app prefers this value and accepts `DATABASE_URL` as a fallback. |
| `PULSE_TOKEN_KEY` | Base64 key that decodes to 32 bytes |
| `PULSE_PUBLIC_URL` | Web frontend origin, without a path. Locally use `http://localhost:3000`. |
| `PULSE_BIND` | Local server only. Defaults to `127.0.0.1:4000`. |

Use separate database branches or databases for production and testing. Keep the database URL, Google secret and token key in the Rust project's environment. Postgres is the only storage dependency. The Next.js frontend only needs the backend URL.

## Deploy the backend on Vercel

1. Import the same GitHub repository for both Vercel projects. Create the Rust project rooted at **`backend`**. Choose **Other** as the framework preset and keep build and output overrides empty. The Rust runtime uses [vercel.json](vercel.json), the Cargo manifest and the [function entry point](api/vercel.rs). The backend uses Singapore (`sin1`) to stay near the Neon database.
2. In the backend project's Environment Variables, add the first five variables above to **Production**. Set `PULSE_PUBLIC_URL` to the Next.js site's production origin. If you use a Preview domain, configure its origin and OAuth callback separately.
3. Deploy using Vercel's Git integration, or run `vercel --prod` from the `backend` folder. Open `/healthz` on the deployed backend URL. The JSON should show `"configured":true` and `"status":"ok"`.
4. Deploy [the web project](../web/README.md) with `PULSE_BACKEND_URL` set to the Rust deployment's origin. Test **Connect Google** from the web site. The Google callback must return to that same site.

Use a backend domain the web project can reach. If Deployment Protection intercepts requests to the backend with a Vercel sign-in page, adjust the project's protection configuration for the production API origin.

[Vercel Rust runtime](https://vercel.com/docs/functions/runtimes/rust) · [Vercel Postgres integrations](https://vercel.com/docs/postgres) · [Google OAuth web server flow](https://developers.google.com/identity/protocols/oauth2/web-server)

## Run locally

Install Rust and Cargo. On Linux, install the OpenSSL development headers and pkg-config. Start the API from this folder so it loads `.env`.

```sh
cargo run --locked
curl http://127.0.0.1:4000/healthz
```

When the four credential variables are blank, the API starts in guest mode for UI development. Then start Next.js from `web` and open `http://localhost:3000`.

## API

| Method and path | Behavior |
| --- | --- |
| `GET /healthz` | Service and configured storage check |
| `GET /auth/google/login` | Starts Google authorization |
| `GET /auth/google/callback` | Consumes OAuth state, creates a session and redirects to the web app |
| `POST /auth/logout` | Ends the Pulse session |
| `GET /api/v1/session` | Sign-in state and display name |
| `GET /api/v1/youtube/search?q=QUERY` | Searches the YouTube music catalog |
| `GET /api/v1/youtube/playlists` | Lists playlists on the signed-in YouTube account |
| `GET /api/v1/youtube/playlist-items?playlist_id=ID` | Lists entries for a playlist |
| `GET /api/v1/favorites` | Lists the user's saved video IDs and available video metadata |
| `PUT /api/v1/favorites/VIDEO_ID` | Saves a video |
| `DELETE /api/v1/favorites/VIDEO_ID` | Removes a saved video |

Search, playlists and playlist items accept `page_token` from the previous response's `nextPageToken`. Search and state-changing browser requests send `X-Pulse-Request: 1`. Search is limited to 20 requests per account per UTC day and also uses the Google Cloud project's shared YouTube API quota.

## Storage and Android

Postgres stores OAuth state, accounts, sessions and saved video IDs. OAuth state is single use and expires after ten minutes. Sessions expire after eight hours and survive function restarts. Session and OAuth state cookie values are hashed in the database, and provider tokens are encrypted in the Rust service. Expired rows are pruned when a sign-in starts. The browser holds HTTP-only cookies.

The Android app currently talks directly to its music providers and needs no backend URL. A future Android integration would need a configurable API origin and a mobile login flow if you want to share Pulse liked tracks with the web app. The web integration uses public YouTube APIs for browsing and library management.
