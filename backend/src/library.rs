use super::*;

const NOT_FOUND: ApiError = (StatusCode::NOT_FOUND, "Playlist not found");

#[derive(Deserialize)]
pub(super) struct Name {
    name: String,
}

#[derive(Deserialize)]
pub(super) struct Track {
    title: String,
    artist: String,
}

fn playlist_name(value: &Name) -> ApiResult<&str> {
    let name = value.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Use a playlist name between 1 and 100 characters",
        ));
    }
    Ok(name)
}

pub(super) async fn playlists(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let rows = client
        .query(
            "SELECT p.id, p.name, count(t.track_id) FROM pulse_playlists p
         LEFT JOIN pulse_playlist_tracks t ON p.id = t.playlist_id
         WHERE p.account_id = $1 GROUP BY p.id ORDER BY p.created_at DESC LIMIT 100",
            &[&session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
    Ok(Json(json!({ "items": rows.iter().map(|row| json!({
        "id": row.get::<_, String>(0), "title": row.get::<_, String>(1), "count": row.get::<_, i64>(2),
    })).collect::<Vec<_>>() })))
}

pub(super) async fn playlist(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let row = client
        .query_opt(
            "SELECT name FROM pulse_playlists WHERE id = $1 AND account_id = $2",
            &[&id, &session.sub],
        )
        .await
        .map_err(|_| STORAGE)?
        .ok_or(NOT_FOUND)?;
    let rows = client
        .query(
            "SELECT t.track_id, t.title, t.artist FROM pulse_playlist_tracks t
         JOIN pulse_playlists p ON p.id = t.playlist_id WHERE p.id = $1 AND p.account_id = $2
         ORDER BY t.added_at, t.track_id LIMIT 250",
            &[&id, &session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
    Ok(Json(
        json!({ "id": id, "title": row.get::<_, String>(0), "items": rows.iter().map(|row| json!({
        "id": row.get::<_, String>(0), "snippet": { "title": row.get::<_, String>(1), "channelTitle": row.get::<_, String>(2) }
    })).collect::<Vec<_>>() }),
    ))
}

pub(super) async fn create(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<Name>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_web_request(&headers)?;
    let name = playlist_name(&input)?;
    let session = app.signed_in(&headers).await?;
    let id = URL_SAFE_NO_PAD.encode(random::<18>()?);
    let mut client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    tx.query_one(
        "SELECT account_id FROM pulse_accounts WHERE account_id = $1 FOR UPDATE",
        &[&session.sub],
    )
    .await
    .map_err(|_| STORAGE)?;
    let inserted = tx.execute(
        "INSERT INTO pulse_playlists (id, account_id, name)
         SELECT $1, $2, $3 WHERE (SELECT count(*) FROM pulse_playlists WHERE account_id = $2) < 100",
        &[&id, &session.sub, &name],
    ).await.map_err(|_| STORAGE)?;
    if inserted == 0 {
        return Err((
            StatusCode::CONFLICT,
            "Your library holds up to 100 playlists",
        ));
    }
    tx.commit().await.map_err(|_| STORAGE)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "title": name, "count": 0 })),
    ))
}

pub(super) async fn rename(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Name>,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    let name = playlist_name(&input)?;
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let changed = client
        .execute(
            "UPDATE pulse_playlists SET name = $1 WHERE id = $2 AND account_id = $3",
            &[&name, &id, &session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
    if changed == 0 {
        return Err(NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn remove(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    client
        .execute(
            "DELETE FROM pulse_playlists WHERE id = $1 AND account_id = $2",
            &[&id, &session.sub],
        )
        .await
        .map_err(|_| STORAGE)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn add(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((id, track)): Path<(String, String)>,
    Json(input): Json<Track>,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    if !valid_track_id(&track)
        || input.title.trim().is_empty()
        || input.title.len() > 500
        || input.artist.len() > 400
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid track details"));
    }
    let session = app.signed_in(&headers).await?;
    let mut client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    let tx = client.transaction().await.map_err(|_| STORAGE)?;
    tx.query_opt(
        "SELECT id FROM pulse_playlists WHERE id = $1 AND account_id = $2 FOR UPDATE",
        &[&id, &session.sub],
    )
    .await
    .map_err(|_| STORAGE)?
    .ok_or(NOT_FOUND)?;
    let changed = tx.execute(
        "INSERT INTO pulse_playlist_tracks (playlist_id, track_id, title, artist)
         SELECT $1, $2, $3, $4 WHERE (SELECT count(*) FROM pulse_playlist_tracks WHERE playlist_id = $1) < 250
         ON CONFLICT DO NOTHING", &[&id, &track, &input.title.trim(), &input.artist.trim()],
    ).await.map_err(|_| STORAGE)?;
    if changed == 0
        && tx
            .query_opt(
                "SELECT 1 FROM pulse_playlist_tracks WHERE playlist_id = $1 AND track_id = $2",
                &[&id, &track],
            )
            .await
            .map_err(|_| STORAGE)?
            .is_none()
    {
        return Err((StatusCode::CONFLICT, "A playlist holds up to 250 tracks"));
    }
    tx.commit().await.map_err(|_| STORAGE)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn remove_track(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((id, track)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    require_web_request(&headers)?;
    let session = app.signed_in(&headers).await?;
    let client = app.backend()?.pool.get().await.map_err(|_| STORAGE)?;
    client
        .execute(
            "DELETE FROM pulse_playlist_tracks t USING pulse_playlists p
         WHERE t.playlist_id = p.id AND p.account_id = $1 AND p.id = $2 AND t.track_id = $3",
            &[&session.sub, &id, &track],
        )
        .await
        .map_err(|_| STORAGE)?;
    Ok(StatusCode::NO_CONTENT)
}
