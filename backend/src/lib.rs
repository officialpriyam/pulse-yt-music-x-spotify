mod auth;
mod library;
mod limits;
pub mod migration;

use std::{
    env,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post, put},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use deadpool_postgres::{Manager, Pool, Runtime, tokio_postgres::Config};
use limits::Limits;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

const SESSION: &str = "pulse_session";
const GOOGLE_TOKEN: &str = "https://oauth2.googleapis.com/token";
const YOUTUBE_READ: &str = "https://www.googleapis.com/auth/youtube.readonly";

pub type AnyError = Box<dyn std::error::Error + Send + Sync>;
type ApiError = (StatusCode, &'static str);
type ApiResult<T> = Result<T, ApiError>;
const UNAUTHORIZED: ApiError = (StatusCode::UNAUTHORIZED, "Sign in to Pulse to continue");
const CONNECT_YOUTUBE: ApiError = (StatusCode::FORBIDDEN, "Connect YouTube to continue");
const UPSTREAM: ApiError = (
    StatusCode::BAD_GATEWAY,
    "Google or YouTube is temporarily unavailable",
);
const STORAGE: ApiError = (
    StatusCode::SERVICE_UNAVAILABLE,
    "Pulse storage is unavailable",
);
const INTERNAL: ApiError = (
    StatusCode::INTERNAL_SERVER_ERROR,
    "Pulse could not complete this request",
);

fn env_value(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn env_limit(key: &str, default: u32, min: u32, max: u32) -> Result<u32, AnyError> {
    let Some(value) = env_value(key) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<u32>()
        .ok()
        .filter(|value| (min..=max).contains(value));
    parsed.ok_or_else(|| format!("{key} must be between {min} and {max}").into())
}

fn postgres_config(connection: &str) -> Result<Config, AnyError> {
    let url = Url::parse(connection).map_err(|_| "The Postgres URL is invalid")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let mut config = connection
        .parse::<Config>()
        .map_err(|_| "The Postgres URL is invalid")?;
    if !local
        && config.get_ssl_mode() != deadpool_postgres::tokio_postgres::config::SslMode::Require
    {
        return Err("Remote Postgres connections require sslmode=require".into());
    }
    config.connect_timeout(Duration::from_secs(5));
    Ok(config)
}

struct Provider {
    id: String,
    secret: String,
}

impl Provider {
    fn from_env(id_key: &str, secret_key: &str) -> Result<Option<Self>, AnyError> {
        match (env_value(id_key), env_value(secret_key)) {
            (None, None) => Ok(None),
            (Some(id), Some(secret)) => Ok(Some(Self { id, secret })),
            _ => Err(format!("Set {id_key} and {secret_key} together").into()),
        }
    }
}

struct Backend {
    base: Url,
    google: Option<Provider>,
    discord: Option<Provider>,
    spotify: Option<Provider>,
    pool: Pool,
    cipher: Aes256Gcm,
    limits: Limits,
}

impl Backend {
    fn from_env() -> Result<Option<Self>, AnyError> {
        let production = env::var("VERCEL_ENV").as_deref() == Ok("production")
            || env_value("PULSE_PUBLIC_URL").is_some_and(|origin| origin.starts_with("https://"));
        let google = Provider::from_env("GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET")?;
        let discord = Provider::from_env("DISCORD_CLIENT_ID", "DISCORD_CLIENT_SECRET")?;
        let spotify = Provider::from_env("SPOTIFY_CLIENT_ID", "SPOTIFY_CLIENT_SECRET")?;
        let database = env_value("DATABASE_URL_POOLED").or_else(|| env_value("DATABASE_URL"));
        let key = env_value("PULSE_TOKEN_KEY");
        if !production
            && database.is_none()
            && key.is_none()
            && google.is_none()
            && discord.is_none()
            && spotify.is_none()
            && env_value("REDIS_URL").is_none()
            && env_value("UPSTASH_REDIS_REST_URL").is_none()
            && env_value("UPSTASH_REDIS_REST_TOKEN").is_none()
        {
            return Ok(None);
        }
        let database = database.ok_or("Set DATABASE_URL_POOLED or DATABASE_URL")?;
        let key = key.ok_or("Set PULSE_TOKEN_KEY")?;
        let origin = env_value("PULSE_PUBLIC_URL").ok_or("PULSE_PUBLIC_URL is required")?;
        let base = Url::parse(&origin).map_err(|_| "PULSE_PUBLIC_URL is invalid")?;
        let local = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (base.scheme() != "https" && (production || !local || base.scheme() != "http"))
            || base.host_str().is_none()
            || base.path() != "/"
            || base.query().is_some()
            || base.fragment().is_some()
            || !base.username().is_empty()
            || base.password().is_some()
        {
            return Err(
                "PULSE_PUBLIC_URL must be a public HTTPS origin or a local HTTP origin".into(),
            );
        }
        let key = STANDARD
            .decode(key)
            .map_err(|_| "PULSE_TOKEN_KEY must be base64")?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| "PULSE_TOKEN_KEY must decode to 32 bytes")?;
        let config = postgres_config(&database)?;
        let tls = postgres_native_tls::MakeTlsConnector::new(native_tls::TlsConnector::new()?);
        let pool = Pool::builder(Manager::new(config, tls))
            .runtime(Runtime::Tokio1)
            .max_size(env_limit("PULSE_DB_POOL_SIZE", 2, 1, 16)? as usize)
            .wait_timeout(Some(Duration::from_millis(700)))
            .create_timeout(Some(Duration::from_secs(8)))
            .build()?;
        Ok(Some(Self {
            google,
            discord,
            spotify,
            pool,
            cipher,
            limits: Limits::from_env(&base)?,
            base,
        }))
    }

    fn seal(&self, hash: &[u8], tokens: &Tokens) -> ApiResult<Vec<u8>> {
        let nonce = random::<12>()?;
        let message = serde_json::to_vec(tokens).map_err(|_| INTERNAL)?;
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &message,
                    aad: hash,
                },
            )
            .map_err(|_| INTERNAL)?;
        Ok([nonce.as_slice(), &ciphertext].concat())
    }

    fn open(&self, hash: &[u8], data: &[u8]) -> ApiResult<Tokens> {
        let (nonce, ciphertext) = data.split_at_checked(12).ok_or(INTERNAL)?;
        let message = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: hash,
                },
            )
            .map_err(|_| UNAUTHORIZED)?;
        serde_json::from_slice(&message).map_err(|_| INTERNAL)
    }

    fn cookie_name(&self, name: &str) -> String {
        if self.base.scheme() == "https" {
            format!("__Host-{name}")
        } else {
            name.into()
        }
    }

    fn cookie(&self, name: &str, value: &str, ttl: u64) -> HeaderValue {
        let secure = if self.base.scheme() == "https" {
            "; Secure"
        } else {
            ""
        };
        let name = self.cookie_name(name);
        HeaderValue::from_str(&format!(
            "{name}={value}; Path=/; Max-Age={ttl}; HttpOnly; SameSite=Lax{secure}"
        ))
        .expect("server-generated cookie")
    }
}

struct App {
    http: Client,
    backend: Option<Backend>,
    password_jobs: Arc<Semaphore>,
    inflight: Semaphore,
}

impl App {
    fn backend(&self) -> ApiResult<&Backend> {
        self.backend.as_ref().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Pulse accounts are unavailable",
        ))
    }

    async fn token_request(&self, fields: &[(&str, &str)]) -> ApiResult<TokenResponse> {
        let response = self
            .http
            .post(GOOGLE_TOKEN)
            .form(fields)
            .send()
            .await
            .map_err(|_| UPSTREAM)?;
        if response.status() == StatusCode::BAD_REQUEST {
            let error = response.json::<Value>().await.map_err(|_| UPSTREAM)?;
            return Err(if error["error"] == "invalid_grant" {
                UNAUTHORIZED
            } else {
                UPSTREAM
            });
        }
        response
            .error_for_status()
            .map_err(|_| UPSTREAM)?
            .json()
            .await
            .map_err(|_| UPSTREAM)
    }

    async fn signed_in(&self, headers: &HeaderMap) -> ApiResult<SignedIn> {
        let Some(backend) = &self.backend else {
            return Err(UNAUTHORIZED);
        };
        let (hash, mobile) = session_credentials(headers, backend).ok_or(UNAUTHORIZED)?;
        let client = backend.pool.get().await.map_err(|_| STORAGE)?;
        let row = client
            .query_opt(
                "SELECT a.account_id, a.display_name, a.username, a.google_subject IS NOT NULL,
                    a.discord_subject IS NOT NULL, s.encrypted_tokens,
                    c.account_id IS NOT NULL, sp.account_id IS NOT NULL
             FROM pulse_sessions s JOIN pulse_accounts a USING (account_id)
             LEFT JOIN pulse_youtube_connections c USING (account_id)
             LEFT JOIN pulse_spotify_connections sp USING (account_id)
             WHERE session_hash = $1 AND expires_at > now() AND mobile = $2",
                &[&&hash[..], &mobile],
            )
            .await
            .map_err(|_| STORAGE)?
            .ok_or(UNAUTHORIZED)?;
        let legacy_tokens: Vec<u8> = row.get(5);
        Ok(SignedIn {
            hash,
            sub: row.get(0),
            name: row.get(1),
            username: row.get(2),
            google_linked: row.get(3),
            discord_linked: row.get(4),
            youtube_connected: row.get::<_, bool>(6) || !legacy_tokens.is_empty(),
            spotify_connected: row.get(7),
        })
    }

    async fn access_token(&self, session: &SignedIn) -> ApiResult<String> {
        let backend = self.backend()?;
        let google = backend.google.as_ref().ok_or(CONNECT_YOUTUBE)?;
        let account_hash = digest(&session.sub);
        let hash = URL_SAFE_NO_PAD.encode(account_hash);
        let (tokens, _) = self.youtube_tokens(backend, session, &account_hash).await?;
        if tokens.expires_at > now() {
            return Ok(tokens.access_token);
        }
        let lease = backend.limits.lock_refresh(&hash).await?.ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "YouTube connection is refreshing. Try again.",
        ))?;
        let result = async {
            let (tokens, current) = self.youtube_tokens(backend, session, &account_hash).await?;
            if tokens.expires_at > now() {
                return Ok(tokens.access_token);
            }
            let refresh = tokens.refresh_token.as_deref().ok_or(CONNECT_YOUTUBE)?;
            let next = self
                .token_request(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", &google.id),
                    ("client_secret", &google.secret),
                    ("refresh_token", refresh),
                ])
                .await
                .map_err(|error| {
                    if error == UNAUTHORIZED {
                        CONNECT_YOUTUBE
                    } else {
                        error
                    }
                })?
                .into_tokens(tokens.refresh_token);
            let updated = backend.seal(&account_hash, &next)?;
            let client = backend.pool.get().await.map_err(|_| STORAGE)?;
            let count = client
                .execute(
                    "UPDATE pulse_youtube_connections SET encrypted_tokens = $1
                 WHERE account_id = $2 AND encrypted_tokens = $3",
                    &[&updated, &session.sub, &current],
                )
                .await
                .map_err(|_| STORAGE)?;
            if count == 0 {
                return Err(CONNECT_YOUTUBE);
            }
            Ok(next.access_token)
        }
        .await;
        backend.limits.release_refresh(&hash, &lease).await;
        result
    }

    async fn youtube_tokens(
        &self,
        backend: &Backend,
        session: &SignedIn,
        account_hash: &[u8],
    ) -> ApiResult<(Tokens, Vec<u8>)> {
        let mut client = backend.pool.get().await.map_err(|_| STORAGE)?;
        if let Some(row) = client
            .query_opt(
                "SELECT encrypted_tokens FROM pulse_youtube_connections WHERE account_id = $1",
                &[&session.sub],
            )
            .await
            .map_err(|_| STORAGE)?
        {
            let data: Vec<u8> = row.get(0);
            return Ok((backend.open(account_hash, &data)?, data));
        }
        // Promote a pre-migration Google session without asking the user to sign in again.
        let tx = client.transaction().await.map_err(|_| STORAGE)?;
        tx.query_one(
            "SELECT account_id FROM pulse_accounts WHERE account_id = $1 FOR UPDATE",
            &[&session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
        let row = tx
            .query_opt(
                "SELECT encrypted_tokens FROM pulse_sessions WHERE session_hash = $1
             AND expires_at > now() AND octet_length(encrypted_tokens) > 0 FOR UPDATE",
                &[&&session.hash[..]],
            )
            .await
            .map_err(|_| STORAGE)?
            .ok_or(CONNECT_YOUTUBE)?;
        let data: Vec<u8> = row.get(0);
        let promoted = backend.seal(account_hash, &backend.open(&session.hash, &data)?)?;
        let row = tx.query_one(
            "INSERT INTO pulse_youtube_connections (account_id, encrypted_tokens) VALUES ($1, $2)
             ON CONFLICT (account_id) DO UPDATE SET account_id = EXCLUDED.account_id RETURNING encrypted_tokens",
            &[&session.sub, &promoted],
        ).await.map_err(|_| STORAGE)?;
        tx.commit().await.map_err(|_| STORAGE)?;
        let data: Vec<u8> = row.get(0);
        Ok((backend.open(account_hash, &data)?, data))
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

impl TokenResponse {
    fn into_tokens(self, previous_refresh: Option<String>) -> Tokens {
        Tokens {
            access_token: self.access_token,
            refresh_token: self.refresh_token.or(previous_refresh),
            expires_at: now().saturating_add(self.expires_in.min(86_400).saturating_sub(30)),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
}

struct SignedIn {
    hash: [u8; 32],
    sub: String,
    name: String,
    username: Option<String>,
    google_linked: bool,
    discord_linked: bool,
    youtube_connected: bool,
    spotify_connected: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random<const N: usize>() -> ApiResult<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| INTERNAL)?;
    Ok(bytes)
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get_all(header::COOKIE).iter().find_map(|value| {
        value.to_str().ok()?.split(';').find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name).then_some(value)
        })
    })
}

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn cookie_hash(headers: &HeaderMap, name: &str) -> Option<[u8; 32]> {
    let cookie = cookie_value(headers, name)?;
    (cookie.len() == 43).then(|| digest(cookie))
}

fn session_credentials(headers: &HeaderMap, backend: &Backend) -> Option<([u8; 32], bool)> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        let token = value.to_str().ok()?.strip_prefix("Bearer ")?;
        return (token.len() == 43).then(|| (digest(token), true));
    }
    cookie_hash(headers, &backend.cookie_name(SESSION)).map(|hash| (hash, false))
}

fn require_web_request(headers: &HeaderMap) -> ApiResult<()> {
    if headers
        .get("x-pulse-request")
        .is_some_and(|value| value == "1")
    {
        return Ok(());
    }
    Err((StatusCode::FORBIDDEN, "Missing Pulse request header"))
}

async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Response> {
    require_web_request(&headers)?;
    let google = app.backend()?;
    if let Some((hash, mobile)) = session_credentials(&headers, google) {
        google
            .pool
            .get()
            .await
            .map_err(|_| STORAGE)?
            .execute(
                "DELETE FROM pulse_sessions WHERE session_hash = $1 AND mobile = $2",
                &[&&hash[..], &mobile],
            )
            .await
            .map_err(|_| STORAGE)?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, google.cookie(SESSION, "", 0));
    Ok(response)
}

#[derive(Deserialize)]
struct Page {
    page_token: Option<String>,
}

#[derive(Deserialize)]
struct PlaylistPage {
    playlist_id: String,
    page_token: Option<String>,
}

#[derive(Deserialize)]
struct Search {
    q: String,
    page_token: Option<String>,
}

async fn youtube(
    http: &Client,
    token: &str,
    resource: &str,
    fields: &[(&str, &str)],
    page: Option<String>,
) -> ApiResult<Json<Value>> {
    let mut request = http
        .get(format!("https://www.googleapis.com/youtube/v3/{resource}"))
        .bearer_auth(token)
        .query(fields);
    if let Some(page) = page.filter(|page| !page.is_empty()) {
        request = request.query(&[("pageToken", page)]);
    }
    let response = request.send().await.map_err(|_| UPSTREAM)?;
    match response.status() {
        StatusCode::BAD_REQUEST => Err((StatusCode::BAD_REQUEST, "Invalid YouTube request")),
        StatusCode::UNAUTHORIZED => Err(UNAUTHORIZED),
        StatusCode::FORBIDDEN => Err((
            StatusCode::FORBIDDEN,
            "YouTube denied access or quota is exhausted",
        )),
        StatusCode::NOT_FOUND => Err((StatusCode::NOT_FOUND, "YouTube content was not found")),
        status if status.is_success() => response.json().await.map(Json).map_err(|_| UPSTREAM),
        _ => Err(UPSTREAM),
    }
}

async fn popular(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let token = app.access_token(&app.signed_in(&headers).await?).await?;
    youtube(
        &app.http,
        &token,
        "videos",
        &[
            ("part", "snippet"),
            ("chart", "mostPopular"),
            ("videoCategoryId", "10"),
            ("maxResults", "24"),
        ],
        None,
    )
    .await
}

async fn youtube_liked(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> ApiResult<Json<Value>> {
    let token = app.access_token(&app.signed_in(&headers).await?).await?;
    youtube(
        &app.http,
        &token,
        "videos",
        &[
            ("part", "snippet"),
            ("myRating", "like"),
            ("maxResults", "50"),
        ],
        page.page_token,
    )
    .await
}

async fn playlists(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> ApiResult<Json<Value>> {
    let token = app.access_token(&app.signed_in(&headers).await?).await?;
    youtube(
        &app.http,
        &token,
        "playlists",
        &[
            ("part", "snippet,contentDetails"),
            ("mine", "true"),
            ("maxResults", "50"),
        ],
        page.page_token,
    )
    .await
}

async fn playlist_items(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(page): Query<PlaylistPage>,
) -> ApiResult<Json<Value>> {
    let token = app.access_token(&app.signed_in(&headers).await?).await?;
    youtube(
        &app.http,
        &token,
        "playlistItems",
        &[
            ("part", "snippet,contentDetails"),
            ("playlistId", &page.playlist_id),
            ("maxResults", "50"),
        ],
        page.page_token,
    )
    .await
}

async fn search(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(page): Query<Search>,
) -> ApiResult<Json<Value>> {
    let q = page.q.trim();
    if q.is_empty() || q.chars().count() > 120 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Enter between 1 and 120 characters",
        ));
    }
    require_web_request(&headers)?;
    let session = app.signed_in(&headers).await?;
    let token = app.access_token(&session).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let charged = client
        .execute(
            "INSERT INTO pulse_search_usage (account_id, day, requests)
         VALUES ($1, (now() AT TIME ZONE 'UTC')::date, 1)
         ON CONFLICT (account_id) DO UPDATE SET
           requests = CASE WHEN pulse_search_usage.day = EXCLUDED.day
                      THEN pulse_search_usage.requests + 1 ELSE 1 END,
           day = EXCLUDED.day
         WHERE pulse_search_usage.day <> EXCLUDED.day OR pulse_search_usage.requests < 20",
            &[&session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
    if charged == 0 {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Daily search limit reached"));
    }
    drop(client);
    youtube(
        &app.http,
        &token,
        "search",
        &[
            ("part", "snippet"),
            ("type", "video"),
            ("videoCategoryId", "10"),
            ("maxResults", "24"),
            ("q", q),
        ],
        page.page_token,
    )
    .await
}

async fn favorites(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let rows = client.query(
        "SELECT video_id, title, artist FROM pulse_favorites WHERE account_id = $1 ORDER BY saved_at DESC LIMIT 50",
        &[&session.sub],
    ).await.map_err(|_| STORAGE)?;
    let mut missing = Vec::new();
    let mut songs = Vec::new();
    let mut ids = Vec::new();
    for row in rows {
        let id: String = row.get(0);
        let title: Option<String> = row.get(1);
        let artist: Option<String> = row.get(2);
        if let Some(title) = title {
            songs.push(json!({ "id": id, "snippet": { "title": title, "channelTitle": artist } }));
        } else {
            missing.push(id.clone());
        }
        ids.push(id);
    }
    drop(client);
    if !missing.is_empty() && session.youtube_connected {
        let token = app.access_token(&session).await?;
        let Json(data) = youtube(
            &app.http,
            &token,
            "videos",
            &[("part", "snippet"), ("id", &missing.join(","))],
            None,
        )
        .await?;
        songs.extend(data["items"].as_array().into_iter().flatten().cloned());
    }
    Ok(Json(json!({ "items": songs, "ids": ids })))
}

#[derive(Deserialize)]
struct FavoriteMetadata {
    title: String,
    artist: String,
}

fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_track_id(id: &str) -> bool {
    valid_video_id(id)
        || id.strip_prefix("pulse-").is_some_and(|slug| {
            (6..=50).contains(&slug.len())
                && slug
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

async fn set_favorite(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    method: axum::http::Method,
    body: Option<Json<FavoriteMetadata>>,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    if !valid_track_id(&id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid track ID"));
    }
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    if method == axum::http::Method::DELETE {
        client
            .execute(
                "DELETE FROM pulse_favorites WHERE account_id = $1 AND video_id = $2",
                &[&session.sub, &id],
            )
            .await
            .map_err(|_| STORAGE)?;
    } else {
        let mut client = client;
        let tx = client.transaction().await.map_err(|_| STORAGE)?;
        tx.query_one(
            "SELECT account_id FROM pulse_accounts WHERE account_id = $1 FOR UPDATE",
            &[&session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
        let metadata = body.map(|Json(value)| value);
        if !valid_video_id(&id) && metadata.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Track title and artist are required",
            ));
        }
        if metadata.as_ref().is_some_and(|value| {
            value.title.trim().is_empty() || value.title.len() > 500 || value.artist.len() > 400
        }) {
            return Err((StatusCode::BAD_REQUEST, "Invalid track details"));
        }
        let title = metadata.as_ref().map(|value| value.title.trim());
        let artist = metadata.as_ref().map(|value| value.artist.trim());
        let inserted = tx.execute(
            "INSERT INTO pulse_favorites (account_id, video_id, title, artist)
             SELECT $1, $2, $3, $4 WHERE (SELECT count(*) FROM pulse_favorites WHERE account_id = $1) < 50
             ON CONFLICT (account_id, video_id) DO UPDATE SET title = COALESCE(EXCLUDED.title, pulse_favorites.title),
             artist = COALESCE(EXCLUDED.artist, pulse_favorites.artist)",
            &[&session.sub, &id, &title, &artist],
        ).await.map_err(|_| STORAGE)?;
        if inserted == 0
            && tx
                .query_opt(
                    "SELECT 1 FROM pulse_favorites WHERE account_id = $1 AND video_id = $2",
                    &[&session.sub, &id],
                )
                .await
                .map_err(|_| STORAGE)?
                .is_none()
        {
            return Err((
                StatusCode::CONFLICT,
                "Your library holds up to 50 saved tracks",
            ));
        }
        tx.commit().await.map_err(|_| STORAGE)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn busy() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "2")],
        "Pulse is busy. Try again shortly.",
    )
        .into_response()
}

async fn admission(
    State(app): State<Arc<App>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let Ok(_slot) = app.inflight.try_acquire() else {
        return busy();
    };
    if let Some(google) = &app.backend {
        let method = request.method();
        let origin = google.base.origin().ascii_serialization();
        if !method.is_safe()
            && request
                .headers()
                .get(header::ORIGIN)
                .is_some_and(|value| value.as_bytes() != origin.as_bytes())
        {
            return (StatusCode::FORBIDDEN, "Invalid request origin").into_response();
        }
        let path = request.uri().path();
        if path != "/healthz" {
            let limits = &google.limits;
            let (bucket, count) =
                if path.starts_with("/auth/") || path == "/api/v1/android/google/start" {
                    ("oauth", limits.oauth)
                } else {
                    ("api", limits.global)
                };
            let session = session_credentials(request.headers(), google).map(|(hash, _)| hash);
            let allowed = async move {
                let retry = limits.check(bucket, count).await?;
                if retry > 0 {
                    return Ok(retry);
                }
                if let Some(hash) = session {
                    let session = format!("session:{}", URL_SAFE_NO_PAD.encode(hash));
                    return limits.check(&session, limits.per_session).await;
                }
                Ok(0)
            }
            .await;
            match allowed {
                Ok(0) => (),
                Ok(seconds) => {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        [(header::RETRY_AFTER, seconds.to_string())],
                        "Rate limit exceeded. Try again shortly.",
                    )
                        .into_response();
                }
                Err(error) => return error.into_response(),
            }
        }
    }
    match tokio::time::timeout(Duration::from_secs(22), next.run(request)).await {
        Ok(response) => response,
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, "Pulse request timed out").into_response(),
    }
}

async fn private_response(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn health(State(app): State<Arc<App>>) -> ApiResult<Json<Value>> {
    if let Some(google) = &app.backend {
        let check_database = async {
            let client = google.pool.get().await.map_err(|_| STORAGE)?;
            let version = client
                .query_one("SELECT max(version) FROM pulse_schema_migrations", &[])
                .await
                .map_err(|_| STORAGE)?
                .get::<_, Option<i32>>(0);
            if version.is_none_or(|version| version < migration::LATEST) {
                return Err(STORAGE);
            }
            Ok(())
        };
        tokio::try_join!(check_database, google.limits.ping())?;
    }
    Ok(Json(
        json!({ "service": "pulse-api", "status": "ok", "configured": app.backend.is_some() }),
    ))
}

pub fn router() -> Result<Router, AnyError> {
    let app = Arc::new(App {
        http: Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        backend: Backend::from_env()?,
        password_jobs: Arc::new(Semaphore::new(2)),
        inflight: Semaphore::new(env_limit("PULSE_MAX_INFLIGHT", 32, 1, 256)? as usize),
    });
    Ok(Router::new()
        .route(
            "/",
            get(|| async { Json(json!({ "service": "pulse-api", "health": "/healthz" })) }),
        )
        .route("/healthz", get(health))
        .route("/api/v1/android/google/start", post(auth::android::begin))
        .route(
            "/api/v1/android/google/complete",
            post(auth::android::complete),
        )
        .route("/auth/android/google/start", get(auth::android::authorize))
        .route("/auth/android/google/callback", get(auth::android::finish))
        .route("/auth/{provider}/login", get(auth::oauth_start))
        .route("/auth/{provider}/callback", get(auth::oauth_callback))
        .route("/auth/signup", post(auth::signup))
        .route("/auth/login", post(auth::password_login))
        .route("/auth/credentials", post(auth::add_credentials))
        .route("/auth/youtube/disconnect", post(auth::disconnect_youtube))
        .route("/auth/spotify/disconnect", post(auth::disconnect_spotify))
        .route("/api/v1/spotify/token", post(auth::spotify_token))
        .route(
            "/api/v1/library/playlists",
            get(library::playlists).post(library::create),
        )
        .route(
            "/api/v1/library/playlists/{id}",
            get(library::playlist)
                .put(library::rename)
                .delete(library::remove),
        )
        .route(
            "/api/v1/library/playlists/{id}/tracks/{track}",
            put(library::add).delete(library::remove_track),
        )
        .route("/auth/logout", post(logout))
        .route("/api/v1/session", get(auth::session))
        .route("/api/v1/youtube/playlists", get(playlists))
        .route("/api/v1/youtube/popular", get(popular))
        .route("/api/v1/youtube/liked", get(youtube_liked))
        .route("/api/v1/youtube/playlist-items", get(playlist_items))
        .route("/api/v1/youtube/search", get(search))
        .route("/api/v1/favorites", get(favorites))
        .route(
            "/api/v1/favorites/{id}",
            put(set_favorite).delete(set_favorite),
        )
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(Arc::clone(&app), admission))
        .layer(middleware::from_fn(private_response))
        .with_state(app))
}
