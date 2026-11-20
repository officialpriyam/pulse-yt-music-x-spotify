# Pulse API

The Rust Axum service handles Pulse accounts, connected music services, liked tracks and playlists. It runs as a Vercel Function or as a local HTTP server.

## Prepare Neon and Redis

1. Provision Neon Postgres and apply the [versioned migrations](#database-migrations). Use a pooled, TLS-protected connection string in the Vercel backend.
2. Configure Redis. For Upstash, use its HTTPS REST URL and full-access REST token. For a native Redis provider, set `REDIS_URL` to the provider’s `rediss://` URL and TLS port.
3. Generate a separate encryption key for each environment. Keep the key stable between deployments so provider connections can be decrypted.

```sh
node -e "console.log(require('crypto').randomBytes(32).toString('base64'))"
```

## Optional OAuth providers

Register the callbacks on the Next.js site's origin. Use the local equivalents for development where the provider allows them.

| Provider | Setup | Callback paths |
| --- | --- | --- |
| Google and YouTube | Create a Google OAuth **Web application** client. Configure consent and test users, and enable YouTube Data API v3 for the music connection. | `/auth/google/callback`, `/auth/youtube/callback`, `/auth/android/google/callback` |
| Discord | Create a Discord application in the Developer Portal and register the OAuth2 redirect. | `/auth/discord/callback` |
| Spotify | Register a Spotify app with Web API and Web Playback SDK. Add your test users if the app is in Development Mode. | `/auth/spotify/callback` |

Set each enabled provider's client ID and secret in the backend environment. You can leave both values blank for any provider. Username and password sign-in works independently. Spotify browser playback requires Premium, an eligible developer app, and a supported browser.

## Environment variables

Copy [.env.example](.env.example) to `.env` for local development, or edit the local `.env` if it is already present. Fill in the credentials privately.

| Variable | Value |
| --- | --- |
| `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` | Optional Google web OAuth client |
| `DISCORD_CLIENT_ID` and `DISCORD_CLIENT_SECRET` | Optional Discord OAuth2 client |
| `SPOTIFY_CLIENT_ID` and `SPOTIFY_CLIENT_SECRET` | Optional Spotify OAuth client |
| `DATABASE_URL_POOLED` | Neon pooled Postgres connection string. The app prefers this value and accepts `DATABASE_URL` as a fallback. |
| `UPSTASH_REDIS_REST_URL` and `UPSTASH_REDIS_REST_TOKEN` | Upstash HTTPS REST connection. When both are set, the backend uses this connection. |
| `REDIS_URL` | Alternative Redis URL with `rediss://` and the provider’s TLS port. Loopback `redis://` works for HTTP development. |
| `PULSE_TOKEN_KEY` | Base64 key that decodes to 32 bytes |
| `PULSE_PUBLIC_URL` | Web frontend origin, without a path. Locally use `http://localhost:3000`. |
| `PULSE_BIND` | Local server only. Defaults to `127.0.0.1:4000`. |

Use separate databases or branches and Redis instances for production and testing. Keep the database URL, Redis credentials, Google secret and token encryption key in the Rust project's environment. The Next.js frontend only needs the backend URL.

## Deploy the backend on Vercel

1. Import the same GitHub repository for both Vercel projects. Create the Rust project rooted at **`backend`**. Choose **Other** as the framework preset and keep build and output overrides empty. The Rust runtime uses [vercel.json](vercel.json), the Cargo manifest and the [function entry point](api/main.rs). The backend uses Singapore (`sin1`) to stay near the Neon database.
2. In the backend project's Environment Variables, add the pooled Postgres URL, token encryption key, public web origin, Redis connection settings, and any enabled OAuth clients to **Production**. Set `PULSE_PUBLIC_URL` to the Next.js site's production origin. If you use a Preview domain, configure its origin and OAuth callback separately.
3. Deploy using Vercel's Git integration, or run `vercel --prod` from the `backend` folder. Open `/healthz` on the deployed backend URL. The JSON should show `"configured":true` and `"status":"ok"`.
4. Deploy [the web project](../web/README.md) with `PULSE_BACKEND_URL` set to the Rust deployment's origin. Test username sign-up and whichever provider connections you enabled.

Use a backend domain the web project can reach. If Deployment Protection intercepts requests to the backend with a Vercel sign-in page, adjust the project's protection configuration for the production API origin.

[Vercel Rust runtime](https://vercel.com/docs/functions/runtimes/rust) · [Vercel Postgres integrations](https://vercel.com/docs/postgres) · [Google OAuth web server flow](https://developers.google.com/identity/protocols/oauth2/web-server)

If the hosted function returns `503 configuration_required`, open Vercel **Runtime Logs** for
the deployment. The initialization error names the missing or invalid setting. Update the
backend project's **Production** environment variables and create a new deployment. A successful
build and a ready backend are separate checks.

## Run locally

Install Rust and Cargo. On Linux, install the OpenSSL development headers and pkg-config. Start the API from this folder so it loads `.env`.

```sh
cargo run --locked
curl http://127.0.0.1:4000/healthz
```

When the database, encryption key, Redis and OAuth settings are blank, the API starts in guest mode for UI development. Then start Next.js from `web` and open `http://localhost:3000`.

## Database migrations

The migration history is stored in `pulse_schema_migrations`. The runner verifies checksums and takes a Postgres transaction-level lock, so simultaneous operators cannot apply a version twice. Apply migrations once per database branch before deploying the backend. A database created with the earlier Pulse schema can apply migration 0001 in place.

From `backend/`, set `DATABASE_URL_DIRECT` to the unpooled Neon connection string using a migration role with schema permissions. This variable belongs in the migration environment. The Vercel runtime uses a separate pooled URL with a role restricted to application queries.

```sh
cargo run --locked --bin migrate -- apply
cargo run --locked --bin migrate -- check
cargo run --locked --bin migrate -- prune
```

Schedule `prune` as a maintenance job against the same database. Each run removes up to 1,000 expired OAuth states and 1,000 expired sessions. Run it more frequently if expired rows accumulate.

## Capacity and failure behavior

- Neon is the durable store. Redis coordinates request limits and session refreshes across instances. The API returns a temporary failure when Redis is unavailable.
- Limits default to 6,000 API requests and 180 OAuth requests per minute across the deployment, 120 requests per minute per session, 32 in-flight requests per instance, and two Postgres connections per instance. Tune the `PULSE_*` settings in [.env.example](.env.example) using load measurements and your Neon connection and YouTube API quotas.
- The API returns `429` with `Retry-After` for rate limits and `503` for local saturation. Apply per-IP rules to public auth routes at Vercel to limit abusive clients before they reach the function. Configure observability for error rates, dependency latency, and rate-limit responses.
- `/healthz` checks both dependencies and the database version before marking a configured service ready. Exercise your expected traffic against a staging project before committing to a production throughput target.

## API

| Method and path | Behavior |
| --- | --- |
| `GET /healthz` | Service and configured storage check |
| `POST /auth/signup` and `POST /auth/login` | Create a Pulse account or sign in with a username and password |
| `POST /auth/credentials` | Add a username and password to a signed-in OAuth account |
| `GET /auth/{provider}/login` and `GET /auth/{provider}/callback` | Start and finish an enabled OAuth flow. Pass `connect=true` to link a sign-in provider to the current Pulse account. |
| `POST /auth/{service}/disconnect` | Remove a YouTube or Spotify connection |
| `POST /api/v1/spotify/token` | Issue a short-lived Spotify access token to the connected user |
| `GET /api/v1/youtube/popular` | Browse YouTube's popular music chart |
| `GET or POST /api/v1/library/playlists` | List or create Pulse playlists |
| `GET, PUT or DELETE /api/v1/library/playlists/{id}` | Read, rename or delete a Pulse playlist |
| `PUT or DELETE /api/v1/library/playlists/{id}/tracks/{video_id}` | Add or remove a track |
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

Postgres stores accounts, Argon2id password hashes, OAuth state, sessions, provider connections, liked tracks, and Pulse playlists. OAuth state is single use and expires after ten minutes. Sessions expire after eight hours and survive function restarts. Session and OAuth state cookie values are hashed in the database, and provider tokens are encrypted in the Rust service. Expired rows are pruned by the maintenance command. The browser holds HTTP-only cookies.

Android Google sign-in opens the system browser. Its Google callback is on the public browser origin and finishes with a link back to Pulse. The phone completes the session over HTTPS by proving it initiated the request. An Android OAuth session lasts 30 days and is stored encrypted on the phone. This connection reads the account’s YouTube playlists and liked videos. Google OAuth and YouTube Data API v3 must be enabled for this feature.

Android playback still talks directly to its music services. Its public Pulse API origin is set in `gradle.properties`. To test a different backend with the readiness check in **Settings → About**, pass its origin when building.

```sh
./gradlew :app:assembleDev -PpulseBackendUrl=https://YOUR-BACKEND-DOMAIN
```

The check calls `/healthz` when tapped and reports whether the service and its dependencies are ready. The About row appears on builds configured with a backend URL. The web integration uses public YouTube APIs for browsing and library management.
