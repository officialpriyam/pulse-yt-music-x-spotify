use axum::response::Html;

use super::*;

const ANDROID_COOKIE: &str = "pulse_app_login";
const START: &str = "auth/android/google/start";
const CALLBACK: &str = "auth/android/google/callback";

fn bytes(value: &str) -> ApiResult<[u8; 32]> {
    URL_SAFE_NO_PAD
        .decode(value)
        .ok()
        .and_then(|data| data.try_into().ok())
        .ok_or(AUTH_EXPIRED)
}

#[derive(Deserialize)]
pub(crate) struct Begin {
    challenge: String,
}

#[derive(Deserialize)]
pub(crate) struct BrowserState {
    state: String,
}

#[derive(Deserialize)]
pub(crate) struct Proof {
    state: String,
    verifier: String,
}

fn callback_url(backend: &Backend) -> ApiResult<Url> {
    backend.base.join(CALLBACK).map_err(|_| INTERNAL)
}

pub(crate) async fn begin(
    State(app): State<Arc<App>>,
    Json(input): Json<Begin>,
) -> ApiResult<Json<Value>> {
    let backend = app.backend()?;
    Flow::Youtube.provider(backend)?;
    let challenge = bytes(&input.challenge)?;
    let state = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let verifier = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let client = backend.pool.get().await.map_err(|_| STORAGE)?;
    client
        .execute(
            "INSERT INTO pulse_android_oauth (state_hash, challenge, google_verifier, expires_at)
         VALUES ($1, $2, $3, now() + interval '10 minutes')",
            &[&&digest(&state)[..], &&challenge[..], &verifier],
        )
        .await
        .map_err(|_| STORAGE)?;
    let mut url = backend.base.join(START).map_err(|_| INTERNAL)?;
    url.query_pairs_mut().append_pair("state", &state);
    Ok(Json(json!({ "state": state, "browser_url": url.as_str() })))
}

pub(crate) async fn authorize(
    State(app): State<Arc<App>>,
    Query(input): Query<BrowserState>,
) -> ApiResult<Response> {
    let backend = app.backend()?;
    let google = Flow::Youtube.provider(backend)?;
    bytes(&input.state)?;
    let client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let verifier: String = client
        .query_opt(
            "SELECT google_verifier FROM pulse_android_oauth
         WHERE state_hash = $1 AND expires_at > now() AND google_verifier <> ''",
            &[&&digest(&input.state)[..]],
        )
        .await
        .map_err(|_| STORAGE)?
        .ok_or(AUTH_EXPIRED)?
        .get(0);
    let mut url =
        Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("Google OAuth URL");
    url.query_pairs_mut()
        .append_pair("client_id", &google.id)
        .append_pair("redirect_uri", callback_url(backend)?.as_str())
        .append_pair("response_type", "code")
        .append_pair("scope", &format!("openid profile {YOUTUBE_READ}"))
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent select_account")
        .append_pair("state", &input.state)
        .append_pair("code_challenge", &URL_SAFE_NO_PAD.encode(digest(&verifier)))
        .append_pair("code_challenge_method", "S256");
    Ok((
        [(
            header::SET_COOKIE,
            backend.cookie(ANDROID_COOKIE, &input.state, 600),
        )],
        Redirect::to(url.as_str()),
    )
        .into_response())
}

fn return_to_app(backend: &Backend, success: bool) -> Response {
    let message = if success {
        "Google is connected. Return to Pulse to open your library."
    } else {
        "Google sign-in was not completed. Return to Pulse to try again."
    };
    (
        [(header::SET_COOKIE, backend.cookie(ANDROID_COOKIE, "", 0))],
        Html(format!(r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta name="referrer" content="no-referrer"><title>Pulse sign-in</title></head><body style="background:#101216;color:#f7f6f3;font:18px system-ui;padding:24px;max-width:32rem;margin:10vh auto"><h1>Pulse</h1><p>{message}</p><p><a style="color:#d8e2b0" href="pulse-app://oauth/complete">Return to Pulse</a></p></body></html>"#)),
    ).into_response()
}

pub(crate) async fn finish(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(input): Query<Callback>,
) -> ApiResult<Response> {
    let backend = app.backend()?;
    let google = Flow::Youtube.provider(backend)?;
    let hash = cookie_hash(&headers, &backend.cookie_name(ANDROID_COOKIE))
        .filter(|hash| *hash == digest(&input.state))
        .ok_or(AUTH_EXPIRED)?;
    let mut client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    let row = tx
        .query_opt(
            "SELECT google_verifier FROM pulse_android_oauth
         WHERE state_hash = $1 AND expires_at > now() AND google_verifier <> '' FOR UPDATE",
            &[&&hash[..]],
        )
        .await
        .map_err(|_| STORAGE)?
        .ok_or(AUTH_EXPIRED)?;
    let verifier: String = row.get(0);
    tx.execute(
        "UPDATE pulse_android_oauth SET google_verifier = '' WHERE state_hash = $1",
        &[&&hash[..]],
    )
    .await
    .map_err(|_| STORAGE)?;
    tx.commit().await.map_err(|_| STORAGE)?;
    drop(client);

    let result = async {
        let code = input.code.as_deref().ok_or(AUTH_EXPIRED)?;
        let redirect = callback_url(backend)?;
        let response = app
            .token_request(&[
                ("grant_type", "authorization_code"),
                ("client_id", &google.id),
                ("client_secret", &google.secret),
                ("redirect_uri", redirect.as_str()),
                ("code", code),
                ("code_verifier", &verifier),
            ])
            .await?;
        if !response
            .scope
            .as_deref()
            .is_some_and(|scope| scope.split_whitespace().any(|item| item == YOUTUBE_READ))
        {
            return Err(CONNECT_YOUTUBE);
        }
        let profile: Identity = app
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
        if profile.sub.is_empty() || profile.sub.len() > 256 {
            return Err(UPSTREAM);
        }
        Ok((profile, response.into_tokens(None)))
    }
    .await;
    let mut client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let Ok((profile, tokens)) = result else {
        client
            .execute(
                "UPDATE pulse_android_oauth SET denied = true WHERE state_hash = $1",
                &[&&hash[..]],
            )
            .await
            .map_err(|_| STORAGE)?;
        return Ok(return_to_app(backend, false));
    };
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    let candidate = format!("pulse:{}", URL_SAFE_NO_PAD.encode(random::<24>()?));
    let name: String = profile.name.chars().take(80).collect();
    let account: String = tx.query_one(
        "INSERT INTO pulse_accounts (account_id, display_name, google_subject) VALUES ($1, $2, $3)
         ON CONFLICT (google_subject) DO UPDATE SET google_subject = EXCLUDED.google_subject RETURNING account_id",
        &[&candidate, &name, &profile.sub],
    ).await.map_err(|_| STORAGE)?.get(0);
    let data = backend.seal(&digest(&account), &tokens)?;
    tx.execute(
        "INSERT INTO pulse_youtube_connections (account_id, encrypted_tokens) VALUES ($1, $2)
         ON CONFLICT (account_id) DO UPDATE SET encrypted_tokens = EXCLUDED.encrypted_tokens",
        &[&account, &data],
    )
    .await
    .map_err(|_| STORAGE)?;
    let updated = tx.execute("UPDATE pulse_android_oauth SET account_id = $1 WHERE state_hash = $2 AND expires_at > now()",
        &[&account, &&hash[..]]).await.map_err(|_| STORAGE)?;
    if updated == 0 {
        return Err(AUTH_EXPIRED);
    }
    tx.commit().await.map_err(|_| STORAGE)?;
    Ok(return_to_app(backend, true))
}

pub(crate) async fn complete(
    State(app): State<Arc<App>>,
    Json(input): Json<Proof>,
) -> ApiResult<Response> {
    let backend = app.backend()?;
    bytes(&input.state)?;
    bytes(&input.verifier)?;
    let challenge = digest(&input.verifier);
    let state = digest(&input.state);
    let bucket = format!("android-oauth:{}", URL_SAFE_NO_PAD.encode(state));
    if backend.limits.check(&bucket, 22).await? > 0 {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Wait before checking sign-in again",
        ));
    }
    let mut client = backend.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    let row = tx
        .query_opt(
            "SELECT p.account_id, p.denied, a.display_name, p.issued FROM pulse_android_oauth p
         LEFT JOIN pulse_accounts a USING (account_id)
         WHERE p.state_hash = $1 AND p.challenge = $2 AND p.expires_at > now() FOR UPDATE OF p",
            &[&&state[..], &&challenge[..]],
        )
        .await
        .map_err(|_| STORAGE)?
        .ok_or(AUTH_EXPIRED)?;
    if row.get::<_, bool>(1) {
        return Err(AUTH_EXPIRED);
    }
    let Some(account) = row.get::<_, Option<String>>(0) else {
        return Ok(StatusCode::ACCEPTED.into_response());
    };
    let name: String = row.get(2);
    // A repeated completion after a lost response returns the same transport token.
    let token = URL_SAFE_NO_PAD.encode(digest(&format!(
        "pulse.android.session.{}:{}",
        input.state, input.verifier
    )));
    let hash = digest(&token);
    if row.get::<_, bool>(3) {
        tx.query_opt("SELECT 1 FROM pulse_sessions WHERE session_hash = $1 AND expires_at > now() AND mobile", &[&&hash[..]])
            .await.map_err(|_| STORAGE)?.ok_or(AUTH_EXPIRED)?;
    } else {
        tx.execute(
            "INSERT INTO pulse_sessions (session_hash, account_id, encrypted_tokens, expires_at, mobile)
            VALUES ($1, $2, ''::bytea, now() + interval '30 days', true)",
            &[&&hash[..], &account],
        ).await.map_err(|_| STORAGE)?;
        tx.execute(
            "UPDATE pulse_android_oauth SET issued = true WHERE state_hash = $1",
            &[&&state[..]],
        )
        .await
        .map_err(|_| STORAGE)?;
    }
    tx.commit().await.map_err(|_| STORAGE)?;
    Ok(Json(json!({ "token": token, "name": name })).into_response())
}
