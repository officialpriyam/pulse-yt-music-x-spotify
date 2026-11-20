pub(crate) mod android;

use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString},
};

use super::*;

const INVALID_INPUT: ApiError = (
    StatusCode::BAD_REQUEST,
    "Use 3–32 letters, numbers or underscores for your username and 12–128 bytes for your password",
);
const INVALID_LOGIN: ApiError = (StatusCode::UNAUTHORIZED, "Incorrect username or password");
const LINKED_ELSEWHERE: ApiError = (
    StatusCode::CONFLICT,
    "This provider identity is already linked to a different Pulse account",
);
const AUTH_EXPIRED: ApiError = (StatusCode::BAD_REQUEST, "Sign-in was cancelled or expired");
const OAUTH_COOKIE: &str = "pulse_login";
const SPOTIFY_TOKEN: &str = "https://accounts.spotify.com/api/token";
const DISCORD_TOKEN: &str = "https://discord.com/api/oauth2/token";

#[derive(Deserialize)]
pub(super) struct Credentials {
    username: String,
    password: String,
}

async fn check_credentials(app: &App, input: Credentials) -> ApiResult<(String, String)> {
    let username = input.username.trim().to_ascii_lowercase();
    if !(3..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        || !(12..=128).contains(&input.password.len())
    {
        return Err(INVALID_INPUT);
    }
    let bucket = format!("password:{}", URL_SAFE_NO_PAD.encode(digest(&username)));
    if app.backend()?.limits.check(&bucket, 6).await? > 0 {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Try again in a minute"));
    }
    Ok((username, input.password))
}

async fn password_hash(
    app: &App,
    password: String,
    previous: Option<String>,
) -> ApiResult<Option<String>> {
    let slot = Arc::clone(&app.password_jobs)
        .try_acquire_owned()
        .map_err(|_| STORAGE)?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        if let Some(previous) = previous {
            let hash = PasswordHash::new(&previous).map_err(|_| INTERNAL)?;
            return Ok(Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .ok()
                .map(|_| previous));
        }
        let salt = SaltString::encode_b64(&random::<16>()?).map_err(|_| INTERNAL)?;
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| INTERNAL)?;
        Ok(Some(hash.to_string()))
    })
    .await
    .map_err(|_| INTERNAL)?
}

async fn issue_session(
    tx: &deadpool_postgres::Transaction<'_>,
    backend: &Backend,
    headers: &HeaderMap,
    account: &str,
) -> ApiResult<HeaderValue> {
    let id = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let hash = digest(&id);
    tx.execute(
        "INSERT INTO pulse_sessions (session_hash, account_id, encrypted_tokens, expires_at)
         VALUES ($1, $2, ''::bytea, now() + interval '8 hours')",
        &[&&hash[..], &account],
    )
    .await
    .map_err(|_| STORAGE)?;
    if let Some(old) = cookie_hash(headers, &backend.cookie_name(SESSION)) {
        tx.execute(
            "DELETE FROM pulse_sessions WHERE session_hash = $1",
            &[&&old[..]],
        )
        .await
        .map_err(|_| STORAGE)?;
    }
    Ok(backend.cookie(SESSION, &id, 28_800))
}

pub(super) async fn signup(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<Credentials>,
) -> ApiResult<Response> {
    require_web_request(&headers)?;
    let backend = app.backend()?;
    let (username, password) = check_credentials(&app, input).await?;
    let hash = password_hash(&app, password, None).await?.ok_or(INTERNAL)?;
    let account = format!("pulse:{}", URL_SAFE_NO_PAD.encode(random::<24>()?));
    let mut client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    let inserted = tx
        .execute(
            "INSERT INTO pulse_accounts (account_id, display_name, username, password_hash)
         VALUES ($1, $2, $2, $3) ON CONFLICT (username) DO NOTHING",
            &[&account, &username, &hash],
        )
        .await
        .map_err(|_| STORAGE)?;
    if inserted == 0 {
        return Err((
            StatusCode::CONFLICT,
            "Choose a different username or sign in",
        ));
    }
    let cookie = issue_session(&tx, backend, &headers, &account).await?;
    tx.commit().await.map_err(|_| STORAGE)?;
    Ok((StatusCode::CREATED, [(header::SET_COOKIE, cookie)]).into_response())
}

pub(super) async fn password_login(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<Credentials>,
) -> ApiResult<Response> {
    require_web_request(&headers)?;
    let backend = app.backend()?;
    let (username, password) = check_credentials(&app, input).await?;
    let client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let row = client
        .query_opt(
            "SELECT account_id, password_hash FROM pulse_accounts WHERE username = $1",
            &[&username],
        )
        .await
        .map_err(|_| STORAGE)?;
    drop(client);
    let previous = row.as_ref().map(|row| row.get(1));
    // An unknown user still incurs one Argon2 computation.
    let valid = password_hash(&app, password, previous).await?.is_some();
    let account: String = row.filter(|_| valid).ok_or(INVALID_LOGIN)?.get(0);
    let mut client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    let cookie = issue_session(&tx, backend, &headers, &account).await?;
    tx.commit().await.map_err(|_| STORAGE)?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

pub(super) async fn add_credentials(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<Credentials>,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    let session = app.signed_in(&headers).await?;
    if session.username.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "This account already has a username and password",
        ));
    }
    let (username, password) = check_credentials(&app, input).await?;
    let hash = password_hash(&app, password, None).await?.ok_or(INTERNAL)?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let changed = client.execute(
        "UPDATE pulse_accounts SET username = $1, password_hash = $2 WHERE account_id = $3 AND username IS NULL",
        &[&username, &hash, &session.sub],
    ).await.map_err(|error| {
        if error.code() == Some(&deadpool_postgres::tokio_postgres::error::SqlState::UNIQUE_VIOLATION) {
            (StatusCode::CONFLICT, "Choose a different username")
        } else { STORAGE }
    })?;
    if changed == 0 {
        return Err((
            StatusCode::CONFLICT,
            "This account already has a username and password",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Google,
    Discord,
    Youtube,
    Spotify,
}

impl Flow {
    fn parse(value: &str) -> ApiResult<Self> {
        match value {
            "google" => Ok(Self::Google),
            "discord" => Ok(Self::Discord),
            "youtube" => Ok(Self::Youtube),
            "spotify" => Ok(Self::Spotify),
            _ => Err((StatusCode::NOT_FOUND, "Unknown sign-in provider")),
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Discord => "discord",
            Self::Youtube => "youtube",
            Self::Spotify => "spotify",
        }
    }

    fn provider(self, backend: &Backend) -> ApiResult<&Provider> {
        match self {
            Self::Discord => &backend.discord,
            Self::Spotify => &backend.spotify,
            Self::Google | Self::Youtube => &backend.google,
        }
        .as_ref()
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "This provider is unavailable",
        ))
    }

    fn redirect(self, backend: &Backend) -> ApiResult<Url> {
        backend
            .base
            .join(&format!("auth/{}/callback", self.slug()))
            .map_err(|_| INTERNAL)
    }

    fn cookie(self) -> String {
        format!("{OAUTH_COOKIE}_{}", self.slug())
    }
}

#[derive(Deserialize)]
pub(super) struct Start {
    connect: Option<bool>,
}

pub(super) async fn oauth_start(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(options): Query<Start>,
) -> ApiResult<Response> {
    let backend = app.backend()?;
    let flow = Flow::parse(&provider)?;
    let provider = flow.provider(backend)?;
    let linking = matches!(flow, Flow::Youtube | Flow::Spotify) || options.connect.unwrap_or(false);
    let session = if linking {
        Some(app.signed_in(&headers).await?.hash)
    } else {
        None
    };
    let state = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let verifier = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let challenge = URL_SAFE_NO_PAD.encode(digest(&verifier));
    let client = backend.pool.get().await.map_err(|_| STORAGE)?;
    client
        .execute(
            "INSERT INTO pulse_oauth_states (state_hash, verifier, flow, session_hash, expires_at)
         VALUES ($1, $2, $3, $4, now() + interval '10 minutes')",
            &[
                &&digest(&state)[..],
                &verifier,
                &flow.slug(),
                &session.as_ref().map(|hash| &hash[..]),
            ],
        )
        .await
        .map_err(|_| STORAGE)?;
    let (authorize, scope) = match flow {
        Flow::Discord => ("https://discord.com/oauth2/authorize", "identify"),
        Flow::Spotify => (
            "https://accounts.spotify.com/authorize",
            "streaming user-read-email user-read-private user-modify-playback-state",
        ),
        Flow::Google => (
            "https://accounts.google.com/o/oauth2/v2/auth",
            "openid profile",
        ),
        Flow::Youtube => (
            "https://accounts.google.com/o/oauth2/v2/auth",
            "openid profile https://www.googleapis.com/auth/youtube.readonly",
        ),
    };
    let mut url = Url::parse(authorize).expect("OAuth URL");
    let redirect = flow.redirect(backend)?;
    url.query_pairs_mut()
        .append_pair("client_id", &provider.id)
        .append_pair("redirect_uri", redirect.as_str())
        .append_pair("response_type", "code")
        .append_pair("scope", scope)
        .append_pair("state", &state);
    if matches!(flow, Flow::Google | Flow::Youtube) {
        url.query_pairs_mut()
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
    }
    if flow == Flow::Youtube {
        url.query_pairs_mut()
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
    }
    Ok((
        [(
            header::SET_COOKIE,
            backend.cookie(&flow.cookie(), &state, 600),
        )],
        Redirect::to(url.as_str()),
    )
        .into_response())
}

#[derive(Deserialize)]
pub(super) struct Callback {
    code: Option<String>,
    state: String,
}

#[derive(Deserialize)]
struct Identity {
    #[serde(alias = "id")]
    sub: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    username: String,
}

async fn exchange(
    app: &App,
    backend: &Backend,
    provider: &Provider,
    flow: Flow,
    code: &str,
    verifier: &str,
) -> ApiResult<(Identity, Option<Tokens>)> {
    let redirect = flow.redirect(backend)?;
    let fields = [
        ("grant_type", "authorization_code"),
        ("client_id", &provider.id),
        ("client_secret", &provider.secret),
        ("redirect_uri", redirect.as_str()),
        ("code", code),
        ("code_verifier", verifier),
    ];
    let token: TokenResponse = if flow == Flow::Discord {
        app.http
            .post(DISCORD_TOKEN)
            .form(&fields[..5])
            .send()
            .await
            .map_err(|_| UPSTREAM)?
            .error_for_status()
            .map_err(|_| UPSTREAM)?
            .json()
            .await
            .map_err(|_| UPSTREAM)?
    } else {
        app.token_request(&fields).await?
    };
    if flow == Flow::Youtube
        && !token
            .scope
            .as_deref()
            .is_some_and(|s| s.split_whitespace().any(|item| item == YOUTUBE_READ))
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Allow YouTube library access to connect YouTube",
        ));
    }
    let profile_url = if flow == Flow::Discord {
        "https://discord.com/api/v10/users/@me"
    } else {
        "https://openidconnect.googleapis.com/v1/userinfo"
    };
    let profile: Identity = app
        .http
        .get(profile_url)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(|_| UPSTREAM)?
        .error_for_status()
        .map_err(|_| UPSTREAM)?
        .json()
        .await
        .map_err(|_| UPSTREAM)?;
    if profile.sub.is_empty() || profile.sub.len() > 256 {
        return Err(UPSTREAM);
    }
    Ok((
        profile,
        (flow == Flow::Youtube).then(|| token.into_tokens(None)),
    ))
}

pub(super) async fn oauth_callback(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<Callback>,
) -> ApiResult<Response> {
    let backend = app.backend()?;
    let flow = Flow::parse(&provider)?;
    let provider = flow.provider(backend)?;
    let hash = cookie_hash(&headers, &backend.cookie_name(&flow.cookie()))
        .filter(|hash| *hash == digest(&query.state))
        .ok_or(AUTH_EXPIRED)?;
    let client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let row = client.query_opt(
        "DELETE FROM pulse_oauth_states WHERE state_hash = $1 AND flow = $2 AND expires_at > now()
         RETURNING verifier, session_hash",
        &[&&hash[..], &flow.slug()],
    ).await.map_err(|_| STORAGE)?.ok_or(AUTH_EXPIRED)?;
    drop(client);
    let verifier: String = row.get(0);
    let bound: Option<Vec<u8>> = row.get(1);
    let clear = backend.cookie(&flow.cookie(), "", 0);
    let Some(code) = query.code else {
        let target = if bound.is_some() {
            "/settings?auth=cancelled"
        } else {
            "/login?auth=cancelled"
        };
        return Ok(([(header::SET_COOKIE, clear)], Redirect::to(target)).into_response());
    };
    if flow == Flow::Spotify {
        let binding = bound.ok_or(AUTH_EXPIRED)?;
        let active = app.signed_in(&headers).await?;
        if active.hash.as_slice() != binding.as_slice() {
            return Err(AUTH_EXPIRED);
        }
        let redirect = flow.redirect(backend)?;
        let response = spotify_request(
            &app,
            provider,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", redirect.as_str()),
            ],
        )
        .await?;
        if !response
            .scope
            .as_deref()
            .is_some_and(|scope| scope.split_whitespace().any(|item| item == "streaming"))
        {
            return Err((
                StatusCode::FORBIDDEN,
                "Allow Spotify playback access to connect Spotify",
            ));
        }
        let data = backend.seal(&digest(&active.sub), &response.into_tokens(None))?;
        let client = backend.pool.get().await.map_err(|_| STORAGE)?;
        let inserted = client.execute(
            "INSERT INTO pulse_spotify_connections (account_id, encrypted_tokens)
             SELECT account_id, $2 FROM pulse_sessions WHERE session_hash = $1 AND expires_at > now()
             ON CONFLICT (account_id) DO UPDATE SET encrypted_tokens = EXCLUDED.encrypted_tokens",
            &[&binding, &data],
        ).await.map_err(|_| STORAGE)?;
        if inserted == 0 {
            return Err(AUTH_EXPIRED);
        }
        return Ok(([(header::SET_COOKIE, clear)], Redirect::to("/settings")).into_response());
    }
    let (identity, tokens) = exchange(&app, backend, provider, flow, &code, &verifier).await?;
    let column = if flow == Flow::Discord {
        "discord_subject"
    } else {
        "google_subject"
    };
    let mut client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    let linking = bound.is_some();
    let account: String = if let Some(bound) = bound {
        let active = cookie_hash(&headers, &backend.cookie_name(SESSION)).ok_or(AUTH_EXPIRED)?;
        if active.as_slice() != bound.as_slice() {
            return Err(AUTH_EXPIRED);
        }
        let row = tx.query_opt(
            "SELECT account_id FROM pulse_sessions WHERE session_hash = $1 AND expires_at > now()",
            &[&bound],
        ).await.map_err(|_| STORAGE)?.ok_or(AUTH_EXPIRED)?;
        let account: String = row.get(0);
        let changed = tx
            .execute(
                &format!(
                    "UPDATE pulse_accounts SET {column} = $1 WHERE account_id = $2
                      AND ({column} IS NULL OR {column} = $1)"
                ),
                &[&identity.sub, &account],
            )
            .await
            .map_err(|error| {
                if error.code()
                    == Some(&deadpool_postgres::tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
                {
                    LINKED_ELSEWHERE
                } else {
                    STORAGE
                }
            })?;
        if changed == 0 {
            return Err(LINKED_ELSEWHERE);
        }
        account
    } else {
        if flow == Flow::Youtube {
            return Err(AUTH_EXPIRED);
        }
        let account = format!("pulse:{}", URL_SAFE_NO_PAD.encode(random::<24>()?));
        let name = if identity.name.is_empty() {
            &identity.username
        } else {
            &identity.name
        };
        let name: String = name.chars().take(80).collect();
        tx.query_one(
            &format!("INSERT INTO pulse_accounts (account_id, display_name, {column}) VALUES ($1, $2, $3)
                      ON CONFLICT ({column}) DO UPDATE SET {column} = EXCLUDED.{column} RETURNING account_id"),
            &[&account, &name, &identity.sub],
        ).await.map_err(|_| STORAGE)?.get(0)
    };
    if let Some(tokens) = tokens {
        let hash = digest(&account);
        let data = backend.seal(&hash, &tokens)?;
        tx.execute(
            "INSERT INTO pulse_youtube_connections (account_id, encrypted_tokens) VALUES ($1, $2)
             ON CONFLICT (account_id) DO UPDATE SET encrypted_tokens = EXCLUDED.encrypted_tokens",
            &[&account, &data],
        )
        .await
        .map_err(|_| STORAGE)?;
    }
    let cookie = if linking {
        None
    } else {
        Some(issue_session(&tx, backend, &headers, &account).await?)
    };
    tx.commit().await.map_err(|_| STORAGE)?;
    let mut response = Redirect::to(if linking { "/settings" } else { "/" }).into_response();
    response.headers_mut().append(header::SET_COOKIE, clear);
    if let Some(cookie) = cookie {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    Ok(response)
}

pub(super) async fn disconnect_youtube(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    let session = app.signed_in(&headers).await?;
    let mut client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    tx.query_one(
        "SELECT account_id FROM pulse_accounts WHERE account_id = $1 FOR UPDATE",
        &[&session.sub],
    )
    .await
    .map_err(|_| STORAGE)?;
    tx.execute(
        "DELETE FROM pulse_youtube_connections WHERE account_id = $1",
        &[&session.sub],
    )
    .await
    .map_err(|_| STORAGE)?;
    tx.execute(
        "UPDATE pulse_sessions SET encrypted_tokens = ''::bytea WHERE account_id = $1",
        &[&session.sub],
    )
    .await
    .map_err(|_| STORAGE)?;
    tx.commit().await.map_err(|_| STORAGE)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn session(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let mut result = json!({
        "configured": app.backend.is_some(),
        "signed_in": false,
        "google_available": app.backend.as_ref().is_some_and(|b| b.google.is_some()),
        "discord_available": app.backend.as_ref().is_some_and(|b| b.discord.is_some()),
        "spotify_available": app.backend.as_ref().is_some_and(|b| b.spotify.is_some()),
        "spotify_connected": false,
        "youtube_catalog_available": app.youtube_api_key.is_some() && app.backend.is_some(),
        "google_connected": false,
        "discord_linked": false,
        "youtube_connected": false,
    });
    match app.signed_in(&headers).await {
        Ok(session) => {
            result["signed_in"] = json!(true);
            result["name"] = json!(session.name);
            result["username"] = json!(session.username);
            result["google_connected"] = json!(session.google_linked);
            result["discord_linked"] = json!(session.discord_linked);
            result["youtube_connected"] = json!(session.youtube_connected);
            result["spotify_connected"] = json!(session.spotify_connected);
        }
        Err(UNAUTHORIZED) => (),
        Err(error) => return Err(error),
    }
    Ok(Json(result))
}

async fn spotify_request(
    app: &App,
    provider: &Provider,
    fields: &[(&str, &str)],
) -> ApiResult<TokenResponse> {
    let response = app
        .http
        .post(SPOTIFY_TOKEN)
        .basic_auth(&provider.id, Some(&provider.secret))
        .form(fields)
        .send()
        .await
        .map_err(|_| UPSTREAM)?;
    if response.status() == StatusCode::BAD_REQUEST {
        return Err((StatusCode::FORBIDDEN, "Reconnect Spotify to continue"));
    }
    response
        .error_for_status()
        .map_err(|_| UPSTREAM)?
        .json()
        .await
        .map_err(|_| UPSTREAM)
}

pub(super) async fn spotify_token(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    require_web_request(&headers)?;
    let backend = app.backend()?;
    let provider = Flow::Spotify.provider(backend)?;
    let session = app.signed_in(&headers).await?;
    let hash = digest(&session.sub);
    let load = || async {
        let client = backend.pool.get().await.map_err(|_| STORAGE)?;
        let row = client
            .query_opt(
                "SELECT encrypted_tokens FROM pulse_spotify_connections WHERE account_id = $1",
                &[&session.sub],
            )
            .await
            .map_err(|_| STORAGE)?
            .ok_or((StatusCode::FORBIDDEN, "Connect Spotify to continue"))?;
        let data: Vec<u8> = row.get(0);
        Ok::<_, ApiError>((backend.open(&hash, &data)?, data))
    };
    let (tokens, _) = load().await?;
    if tokens.expires_at > now() {
        return Ok(Json(json!({ "access_token": tokens.access_token })));
    }
    let lock = format!("spotify:{}", URL_SAFE_NO_PAD.encode(hash));
    let lease = backend.limits.lock_refresh(&lock).await?.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Spotify connection is refreshing. Try again.",
    ))?;
    let refreshed = async {
        let (tokens, previous) = load().await?;
        if tokens.expires_at > now() {
            return Ok(tokens.access_token);
        }
        let refresh = tokens
            .refresh_token
            .as_deref()
            .ok_or((StatusCode::FORBIDDEN, "Reconnect Spotify to continue"))?;
        let next = spotify_request(
            &app,
            provider,
            &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
        )
        .await?
        .into_tokens(tokens.refresh_token);
        let data = backend.seal(&hash, &next)?;
        let client = backend.pool.get().await.map_err(|_| STORAGE)?;
        let changed = client
            .execute(
                "UPDATE pulse_spotify_connections SET encrypted_tokens = $1
            WHERE account_id = $2 AND encrypted_tokens = $3",
                &[&data, &session.sub, &previous],
            )
            .await
            .map_err(|_| STORAGE)?;
        if changed == 0 {
            return Err((
                StatusCode::CONFLICT,
                "Spotify connection changed. Try again.",
            ));
        }
        Ok(next.access_token)
    }
    .await;
    backend.limits.release_refresh(&lock, &lease).await;
    Ok(Json(json!({ "access_token": refreshed? })))
}

pub(super) async fn disconnect_spotify(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    client
        .execute(
            "DELETE FROM pulse_spotify_connections WHERE account_id = $1",
            &[&session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
    Ok(StatusCode::NO_CONTENT)
}
