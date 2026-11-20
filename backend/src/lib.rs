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
    extract::{Path, Query, State},
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

const CALLBACK: &str = "/auth/google/callback";
const LOGIN: &str = "pulse_login";
const SESSION: &str = "pulse_session";
const GOOGLE_TOKEN: &str = "https://oauth2.googleapis.com/token";
const YOUTUBE_READ: &str = "https://www.googleapis.com/auth/youtube.readonly";

pub type AnyError = Box<dyn std::error::Error + Send + Sync>;
type ApiError = (StatusCode, &'static str);
type ApiResult<T> = Result<T, ApiError>;
const UNAUTHORIZED: ApiError = (StatusCode::UNAUTHORIZED, "Connect Google to continue");
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

struct Google {
    id: String,
    secret: String,
    redirect: Url,
    pool: Pool,
    cipher: Aes256Gcm,
    limits: Limits,
}

impl Google {
    fn from_env() -> Result<Option<Self>, AnyError> {
        let production = env::var("VERCEL_ENV").as_deref() == Ok("production")
            || env_value("PULSE_PUBLIC_URL").is_some_and(|origin| origin.starts_with("https://"));
        let config = (
            env_value("GOOGLE_CLIENT_ID"),
            env_value("GOOGLE_CLIENT_SECRET"),
            env_value("DATABASE_URL_POOLED").or_else(|| env_value("DATABASE_URL")),
            env_value("PULSE_TOKEN_KEY"),
        );
        let (id, secret, database, key) = match config {
            (None, None, None, None) if !production && env_value("REDIS_URL").is_none() => return Ok(None),
            (Some(id), Some(secret), Some(database), Some(key)) => (id, secret, database, key),
            _ => return Err("Set GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET, a Postgres URL and PULSE_TOKEN_KEY together".into()),
        };
        let base = Url::parse(&env::var("PULSE_PUBLIC_URL")?)
            .map_err(|_| "PULSE_PUBLIC_URL is invalid")?;
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
            id,
            secret,
            pool,
            cipher,
            limits: Limits::from_env(&base)?,
            redirect: base.join(CALLBACK)?,
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
        if self.redirect.scheme() == "https" {
            format!("__Host-{name}")
        } else {
            name.into()
        }
    }

    fn cookie(&self, name: &str, value: &str, ttl: u64) -> HeaderValue {
        let secure = if self.redirect.scheme() == "https" {
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
    google: Option<Google>,
    inflight: Semaphore,
}

impl App {
    fn google(&self) -> ApiResult<&Google> {
        self.google.as_ref().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Google and Postgres are not configured",
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
        let Some(google) = &self.google else {
            return Err(UNAUTHORIZED);
        };
        let hash = cookie_hash(headers, &google.cookie_name(SESSION)).ok_or(UNAUTHORIZED)?;
        let client = google.pool.get().await.map_err(|_| STORAGE)?;
        let row = client
            .query_opt(
                "SELECT s.google_sub, a.display_name, s.encrypted_tokens
             FROM pulse_sessions s JOIN pulse_accounts a USING (google_sub)
             WHERE session_hash = $1 AND expires_at > now()",
                &[&&hash[..]],
            )
            .await
            .map_err(|_| STORAGE)?
            .ok_or(UNAUTHORIZED)?;
        Ok(SignedIn {
            hash,
            sub: row.get(0),
            name: row.get(1),
            encrypted: row.get(2),
        })
    }

    async fn access_token(&self, session: &SignedIn) -> ApiResult<String> {
        let google = self.google()?;
        let tokens = google.open(&session.hash, &session.encrypted)?;
        if tokens.expires_at > now() {
            return Ok(tokens.access_token);
        }
        let id = URL_SAFE_NO_PAD.encode(session.hash);
        let lease = google.limits.lock_refresh(&id).await?.ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Google session is refreshing. Try again.",
        ))?;
        let result = self.refresh_access_token(google, &session.hash).await;
        google.limits.release_refresh(&id, &lease).await;
        result
    }

    async fn refresh_access_token(&self, google: &Google, hash: &[u8]) -> ApiResult<String> {
        let client = google.pool.get().await.map_err(|_| STORAGE)?;
        let row = client
            .query_opt(
                "SELECT encrypted_tokens FROM pulse_sessions
             WHERE session_hash = $1 AND expires_at > now()",
                &[&hash],
            )
            .await
            .map_err(|_| STORAGE)?
            .ok_or(UNAUTHORIZED)?;
        let previous: Vec<u8> = row.get(0);
        drop(client);
        let tokens = google.open(hash, &previous)?;
        if tokens.expires_at > now() {
            return Ok(tokens.access_token);
        }
        let refresh = tokens.refresh_token.as_deref().ok_or(UNAUTHORIZED)?;
        let next = self
            .token_request(&[
                ("grant_type", "refresh_token"),
                ("client_id", &google.id),
                ("client_secret", &google.secret),
                ("refresh_token", refresh),
            ])
            .await?
            .into_tokens(tokens.refresh_token);
        let encrypted = google.seal(hash, &next)?;
        let client = google.pool.get().await.map_err(|_| STORAGE)?;
        client
            .execute(
                "UPDATE pulse_sessions SET encrypted_tokens = $1
             WHERE session_hash = $2 AND encrypted_tokens = $3 AND expires_at > now()",
                &[&encrypted, &hash, &previous],
            )
            .await
            .map_err(|_| STORAGE)?;
        Ok(next.access_token)
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
    encrypted: Vec<u8>,
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

async fn login(State(app): State<Arc<App>>) -> ApiResult<Response> {
    let google = app.google()?;
    let state = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let verifier = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let hash = digest(&state);
    let challenge = URL_SAFE_NO_PAD.encode(digest(&verifier));
    let client = google.pool.get().await.map_err(|_| STORAGE)?;
    client
        .execute(
            "INSERT INTO pulse_oauth_states (state_hash, verifier, expires_at)
         VALUES ($1, $2, now() + interval '10 minutes')",
            &[&&hash[..], &verifier],
        )
        .await
        .map_err(|_| STORAGE)?;
    let mut url = Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("Google URL");
    url.query_pairs_mut()
        .append_pair("client_id", &google.id)
        .append_pair("redirect_uri", google.redirect.as_str())
        .append_pair("response_type", "code")
        .append_pair("scope", &format!("openid profile {YOUTUBE_READ}"))
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    let mut response = Redirect::to(url.as_str()).into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, google.cookie(LOGIN, &state, 600));
    Ok(response)
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: String,
}

#[derive(Deserialize)]
struct Profile {
    sub: String,
    #[serde(default)]
    name: String,
}

async fn callback(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> ApiResult<Response> {
    let invalid = (StatusCode::BAD_REQUEST, "Invalid or expired Google sign-in");
    let google = app.google()?;
    let hash = cookie_hash(&headers, &google.cookie_name(LOGIN))
        .filter(|hash| *hash == digest(&query.state))
        .ok_or(invalid)?;
    let client = google.pool.get().await.map_err(|_| STORAGE)?;
    let row = client.query_opt(
        "DELETE FROM pulse_oauth_states WHERE state_hash = $1 AND expires_at > now() RETURNING verifier",
        &[&&hash[..]]
    ).await.map_err(|_| STORAGE)?.ok_or(invalid)?;
    drop(client);

    let code = query
        .code
        .ok_or((StatusCode::BAD_REQUEST, "Google sign-in was cancelled"))?;
    let verifier: &str = row.get(0);
    let response = app
        .token_request(&[
            ("grant_type", "authorization_code"),
            ("client_id", &google.id),
            ("client_secret", &google.secret),
            ("redirect_uri", google.redirect.as_str()),
            ("code", &code),
            ("code_verifier", verifier),
        ])
        .await?;
    if !response
        .scope
        .as_deref()
        .is_some_and(|scope| scope.split_whitespace().any(|value| value == YOUTUBE_READ))
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Allow YouTube access when connecting Google",
        ));
    }
    let profile: Profile = app
        .http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(&response.access_token)
        .send()
        .await
        .map_err(|_| UPSTREAM)?
        .error_for_status()
        .map_err(|_| UPSTREAM)?
        .json()
        .await
        .map_err(|_| UPSTREAM)?;

    let id = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let hash = digest(&id);
    let token_data = google.seal(&hash, &response.into_tokens(None))?;
    let mut client = google.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    tx.execute(
        "INSERT INTO pulse_accounts (google_sub, display_name) VALUES ($1, $2)
         ON CONFLICT (google_sub) DO UPDATE SET display_name = EXCLUDED.display_name",
        &[&profile.sub, &profile.name],
    )
    .await
    .map_err(|_| STORAGE)?;
    tx.execute(
        "INSERT INTO pulse_sessions (session_hash, google_sub, encrypted_tokens, expires_at)
         VALUES ($1, $2, $3, now() + interval '8 hours')",
        &[&&hash[..], &profile.sub, &token_data],
    )
    .await
    .map_err(|_| STORAGE)?;
    if let Some(old) = cookie_hash(&headers, &google.cookie_name(SESSION)) {
        tx.execute(
            "DELETE FROM pulse_sessions WHERE session_hash = $1",
            &[&&old[..]],
        )
        .await
        .map_err(|_| STORAGE)?;
    }
    tx.commit().await.map_err(|_| STORAGE)?;
    let mut response = Redirect::to("/").into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, google.cookie(LOGIN, "", 0));
    response
        .headers_mut()
        .append(header::SET_COOKIE, google.cookie(SESSION, &id, 28_800));
    Ok(response)
}

async fn session(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    match app.signed_in(&headers).await {
        Ok(session) => Ok(Json(
            json!({ "google_connected": true, "name": session.name }),
        )),
        Err(UNAUTHORIZED) => Ok(Json(
            json!({ "google_connected": false, "configured": app.google.is_some() }),
        )),
        Err(error) => Err(error),
    }
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
    let google = app.google()?;
    if let Some(hash) = cookie_hash(&headers, &google.cookie_name(SESSION)) {
        google
            .pool
            .get()
            .await
            .map_err(|_| STORAGE)?
            .execute(
                "DELETE FROM pulse_sessions WHERE session_hash = $1",
                &[&&hash[..]],
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
    let client = app.google()?.pool.get().await.map_err(|_| STORAGE)?;
    let charged = client
        .execute(
            "INSERT INTO pulse_search_usage (google_sub, day, requests)
         VALUES ($1, (now() AT TIME ZONE 'UTC')::date, 1)
         ON CONFLICT (google_sub) DO UPDATE SET
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
    let client = app.google()?.pool.get().await.map_err(|_| STORAGE)?;
    let rows = client.query(
        "SELECT video_id FROM pulse_favorites WHERE google_sub = $1 ORDER BY saved_at DESC LIMIT 50", &[&session.sub]
    ).await.map_err(|_| STORAGE)?;
    let ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    if ids.is_empty() {
        return Ok(Json(json!({ "items": [], "ids": [] })));
    }
    drop(client);
    let token = app.access_token(&session).await?;
    let Json(mut data) = youtube(
        &app.http,
        &token,
        "videos",
        &[("part", "snippet"), ("id", &ids.join(","))],
        None,
    )
    .await?;
    data["ids"] = json!(ids);
    Ok(Json(data))
}

async fn set_favorite(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    method: axum::http::Method,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    if id.len() != 11
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid YouTube video ID"));
    }
    let session = app.signed_in(&headers).await?;
    let client = app.google()?.pool.get().await.map_err(|_| STORAGE)?;
    if method == axum::http::Method::DELETE {
        client
            .execute(
                "DELETE FROM pulse_favorites WHERE google_sub = $1 AND video_id = $2",
                &[&session.sub, &id],
            )
            .await
            .map_err(|_| STORAGE)?;
    } else {
        let mut client = client;
        let tx = client.transaction().await.map_err(|_| STORAGE)?;
        tx.query_one(
            "SELECT google_sub FROM pulse_accounts WHERE google_sub = $1 FOR UPDATE",
            &[&session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
        let inserted = tx
            .execute(
                "INSERT INTO pulse_favorites (google_sub, video_id)
             SELECT $1, $2 WHERE (SELECT count(*) FROM pulse_favorites WHERE google_sub = $1) < 50
             ON CONFLICT DO NOTHING",
                &[&session.sub, &id],
            )
            .await
            .map_err(|_| STORAGE)?;
        if inserted == 0
            && tx
                .query_opt(
                    "SELECT 1 FROM pulse_favorites WHERE google_sub = $1 AND video_id = $2",
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
    if let Some(google) = &app.google {
        let method = request.method();
        let origin = google.redirect.origin().ascii_serialization();
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
            let (bucket, count) = if matches!(path, "/auth/google/login" | CALLBACK) {
                ("oauth", limits.oauth)
            } else {
                ("api", limits.global)
            };
            let session = cookie_hash(request.headers(), &google.cookie_name(SESSION));
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
    if let Some(google) = &app.google {
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
        json!({ "service": "pulse-api", "status": "ok", "configured": app.google.is_some() }),
    ))
}

pub fn router() -> Result<Router, AnyError> {
    let app = Arc::new(App {
        http: Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        google: Google::from_env()?,
        inflight: Semaphore::new(env_limit("PULSE_MAX_INFLIGHT", 32, 1, 256)? as usize),
    });
    Ok(Router::new()
        .route("/healthz", get(health))
        .route("/auth/google/login", get(login))
        .route(CALLBACK, get(callback))
        .route("/auth/logout", post(logout))
        .route("/api/v1/session", get(session))
        .route("/api/v1/youtube/playlists", get(playlists))
        .route("/api/v1/youtube/playlist-items", get(playlist_items))
        .route("/api/v1/youtube/search", get(search))
        .route("/api/v1/favorites", get(favorites))
        .route(
            "/api/v1/favorites/{id}",
            put(set_favorite).delete(set_favorite),
        )
        .layer(middleware::from_fn_with_state(Arc::clone(&app), admission))
        .layer(middleware::from_fn(private_response))
        .with_state(app))
}
