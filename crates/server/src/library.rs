use crate::PrivateState;
use crate::{Api, ApiError, artifacts, chat};
use anyhow::Result;
use axum::{Json, extract::Path, http::StatusCode};
use causewaybay_shared::{ImageInfo, InputMode, ProcessedFile};
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn images(PrivateState(s): PrivateState) -> Api<Json<Vec<ImageInfo>>> {
    let mut images = vec![];
    let wiki = wiki_on(&s)?;
    for m in s.db.all()? {
        if let Some(path) = s.db.image_path(&m.id)? {
            let memo_id = if wiki { m.id.clone() } else { String::new() };
            images.push(ImageInfo {
                id: m.id.clone(),
                name: m.title,
                path,
                url: format!("/api/memos/{}/image", m.id),
                source: "capture".into(),
                created_at: m.created_at,
                tags: memo_tags(&s, &memo_id)?,
                memo_id,
            });
        }
    }
    for f in s.db.files()? {
        if f.mime.starts_with("image/") {
            let memo_id = if wiki {
                s.db.file_memo(&f.id)?.unwrap_or_default()
            } else {
                String::new()
            };
            images.push(ImageInfo {
                id: f.id.clone(),
                name: f.name,
                path: f.path,
                url: format!("/api/files/{}/download", f.id),
                source: "file".into(),
                created_at: f.created_at,
                tags: memo_tags(&s, &memo_id)?,
                memo_id,
            });
        }
    }
    images.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(images))
}
/// A message's Wiki tags (none for a loose file), so a picture or a clip shows
/// and edits its tags where it is listed, like a message in the chat.
fn memo_tags(s: &crate::AppState, memo: &str) -> anyhow::Result<Vec<String>> {
    if memo.is_empty() {
        return Ok(vec![]);
    }
    let conn = s.db.0.lock().unwrap();
    crate::wiki::tags_of(&conn, s.db.owner(), memo)
}
/// Whether the owner's Wiki is on: without it a file has no tags to show or
/// edit, so the lists carry no message either and the pages show no editor.
fn wiki_on(s: &crate::AppState) -> anyhow::Result<bool> {
    let conn = s.db.0.lock().unwrap();
    Ok(crate::assistance::read_locked(&conn, s.db.owner())?.wiki_enabled)
}
/// Every uploaded video, newest first, with a frame thumbnail and a streaming
/// URL that honours Range requests so players can seek.
pub async fn videos(PrivateState(s): PrivateState) -> Api<Json<Vec<serde_json::Value>>> {
    let mut videos: Vec<serde_json::Value> = vec![];
    let wiki = wiki_on(&s)?;
    for f in s.db.files()? {
        if !f.mime.starts_with("video/") {
            continue;
        }
        let memo_id = if wiki {
            s.db.file_memo(&f.id)?.unwrap_or_default()
        } else {
            String::new()
        };
        videos.push(serde_json::json!({
            "id": f.id,
            "name": f.name,
            "mime": f.mime,
            "size": f.size,
            "created_at": f.created_at,
            "url": format!("/api/files/{}/stream", f.id),
            "download": format!("/api/files/{}/download", f.id),
            "thumbnail": format!("/api/files/{}/thumbnail", f.id),
            "tags": memo_tags(&s, &memo_id)?,
            "memo_id": memo_id,
        }));
    }
    videos.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
    Ok(Json(videos))
}
/// A file served inline for playback. A `Range: bytes=a-b` request gets that
/// slice with 206 and Content-Range, which Safari needs before it will seek.
pub async fn stream(
    PrivateState(s): PrivateState,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: axum::extract::Request,
) -> Api<axum::response::Response> {
    let f = s.db.file_info(&id)?;
    if f.path.is_empty() {
        let (_, bytes) = s.db.file(&id)?;
        return serve_bytes(bytes, &f.mime, "inline", Some("private, max-age=3600"));
    }
    let (_, path) = s.db.file_location(&id)?;
    serve_from_disk(
        &path,
        request,
        &f.mime,
        "inline",
        Some("private, max-age=3600"),
    )
    .await
}
/// A file whose bytes live in SQLite (text), served whole with the same
/// headers a disk-backed file gets.
pub fn serve_bytes(
    bytes: Vec<u8>,
    mime: &str,
    disposition: &str,
    cache: Option<&str>,
) -> Api<axum::response::Response> {
    use axum::http::{HeaderValue, header};
    use axum::response::IntoResponse;
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    if let Some(value) = cache.and_then(|c| HeaderValue::from_str(c).ok()) {
        headers.insert(header::CACHE_CONTROL, value);
    }
    Ok(response)
}

/// Serve a stored file straight from the persistence folder with Range
/// support (Safari needs it to seek a video), never loading it into memory.
pub async fn serve_from_disk(
    path: &std::path::Path,
    request: axum::extract::Request,
    mime: &str,
    disposition: &str,
    cache: Option<&str>,
) -> Api<axum::response::Response> {
    use axum::http::{HeaderValue, header};
    use tower::ServiceExt;
    if !path.is_file() {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Stored file is missing from the backend persistence folder".into(),
        ));
    }
    let response = tower_http::services::ServeFile::new(path)
        .oneshot(request)
        .await
        .unwrap_or_else(|never| match never {});
    let mut response = response.map(axum::body::Body::new);
    let served = matches!(
        response.status(),
        StatusCode::OK | StatusCode::PARTIAL_CONTENT
    );
    let headers = response.headers_mut();
    if served {
        if let Ok(value) = HeaderValue::from_str(mime) {
            headers.insert(header::CONTENT_TYPE, value);
        }
        if let Ok(value) = HeaderValue::from_str(disposition) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
    }
    if let Some(value) = cache.and_then(|c| HeaderValue::from_str(c).ok()) {
        headers.insert(header::CACHE_CONTROL, value);
    }
    Ok(response)
}
fn remove(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub async fn delete_file(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
) -> Api<Json<Value>> {
    let f =
        s.db.file_info(&id)
            .map_err(|_| ApiError(StatusCode::NOT_FOUND, "File not found".into()))?;
    // A memo that exists only to file this upload in the Wiki goes with it.
    let kept: Vec<String> = s
        .db
        .0
        .lock()
        .unwrap()
        .prepare("SELECT id FROM memos WHERE owner_wallet=?1 AND roomid IS NULL AND kind IN ('memo','note') AND instr(input,?2)>0")?
        .query_map(rusqlite::params![s.db.owner(), format!("(ID {id})")], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    for memo in kept {
        s.db.delete(&memo)?;
    }
    let mut conn = s.db.0.lock().unwrap();
    let tx = conn.transaction()?;
    if !f.path.is_empty() {
        remove(&artifacts::resolve(&s.db.1, &f.path)?)?;
    }
    tx.execute("DELETE FROM compilations WHERE file_id=?1", [&id])?;
    tx.execute("DELETE FROM files WHERE id=?1", [&id])?;
    chat::remove_attachment(&tx, Some(&id), None)?;
    tx.commit()?;
    crate::forget_thumbnail(&s.db.1, &id);
    Ok(Json(json!({"deleted":true})))
}
pub async fn delete_image(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
) -> Api<Json<Value>> {
    let path =
        s.db.image_path(&id)?
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Image not found".into()))?;
    {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction()?;
        remove(&artifacts::resolve(&s.db.1, &path)?)?;
        crate::forget_thumbnail(&s.db.1, &id);
        tx.execute(
            "UPDATE memos SET image_path=NULL,image=NULL WHERE id=?1",
            [&id],
        )?;
        chat::remove_attachment(&tx, None, Some(&id))?;
        tx.commit()?;
    }
    s.db.sync_wiki(&id)?;
    Ok(Json(json!({"deleted":true})))
}
#[derive(Deserialize, Default)]
pub struct Processing {
    #[serde(default)]
    pub input_mode: InputMode,
}
pub async fn process(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Json(v): Json<Processing>,
) -> Api<Json<ProcessedFile>> {
    let _permit = s.slots.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "The workspace is busy. Try again shortly.".into(),
        )
    })?;
    let result = crate::media::process(&s, &id, v.input_mode).await?;
    s.db.0.lock().unwrap().execute(
        "INSERT INTO file_processing(file_id,document,created_at) VALUES(?1,?2,?3)",
        rusqlite::params![
            id,
            serde_json::to_string(&result).map_err(anyhow::Error::from)?,
            crate::db::now()
        ],
    )?;
    Ok(Json(result))
}
