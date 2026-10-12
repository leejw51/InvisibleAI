//! Resumable chunked video uploads, ported from PocketSkynet's upload protocol.
//!
//! ```text
//! POST   /api/uploads              begin  → { id, offset, size, chunkSize }
//! PATCH  /api/uploads/{id}?offset= append → { offset, size }
//! GET    /api/uploads/{id}         status → { id, offset, size, chunkSize }
//! POST   /api/uploads/{id}/finish  commit → FileInfo
//! DELETE /api/uploads/{id}         abort
//! ```
//!
//! # Why an upload is a conversation and not a request
//!
//! The single-request upload holds the whole body in memory on both sides
//! (base64 in the browser, decoded bytes on the backend), which is why it is
//! capped at 25 MB. A video is split instead: the browser sends `Blob` slices
//! of the picked file, which it never reads into page memory, and each slice
//! is an ordinary small request. The backend's exposure is one chunk
//! ([`MAX_CHUNK_BYTES`]) per upload in flight, not the size of the video, a
//! dropped connection costs one chunk, and progress is what the backend has
//! actually acknowledged.
//!
//! # The offset is the backend's, not the browser's
//!
//! Every append names the offset it believes it is writing at and is refused
//! with a 409 (carrying the real offset) unless the session row agrees. The
//! check and the update are one conditional statement, so two racing chunks
//! cannot both win. That is what makes a retried chunk safe: when the write
//! landed but the response did not, the retry is refused, the browser re-reads
//! the offset, and nothing is written twice.
//!
//! # What `finish` verifies
//!
//! The assembled file is re-hashed and compared with the SHA-256 the browser
//! declared at `begin`, which covers the wire, chunk order, and the disk in one
//! pass. The bytes must also start with a video container signature. Only then
//! is the file moved under `files/`, and SQLite records its path, never its
//! bytes.
use crate::{Api, ApiError, AppState, PrivateState, chat, media};
use anyhow::Context;
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use causewaybay_shared::FileInfo;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The largest video accepted.
pub const MAX_VIDEO: u64 = 1024 * 1024 * 1024;
/// The hard ceiling on one chunk, and therefore the backend's memory per
/// upload in flight. Enforced as the body limit on these routes.
pub const MAX_CHUNK_BYTES: usize = 16 * 1024 * 1024;
/// What `begin` tells the browser to send. Small on purpose: on a lossy link
/// (a phone on cellular or a flaky VPN hop) what matters is how little one
/// failure throws away, not how few requests there are. Advisory: the browser
/// may send less, and chunks need not be uniform.
pub const SUGGESTED_CHUNK_BYTES: usize = 500 * 1024;
/// How long a session may sit untouched before the sweep reclaims its disk.
/// Polling the status counts as being alive.
const SESSION_TTL_SECONDS: i64 = 24 * 60 * 60;
/// Open sessions one account may hold at once.
const MAX_OPEN_SESSIONS: i64 = 8;

pub fn init(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    crate::schema::ensure(
        conn,
        "CREATE TABLE IF NOT EXISTS upload_sessions(id TEXT PRIMARY KEY, owner_wallet TEXT NOT NULL, name TEXT NOT NULL, declared_size INTEGER NOT NULL, received INTEGER NOT NULL DEFAULT 0, sha256 TEXT, room_id TEXT, clear_epoch INTEGER, data_epoch INTEGER, wiki INTEGER NOT NULL DEFAULT 0, temp_name TEXT NOT NULL, created_at INTEGER NOT NULL, touched_at INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS upload_sessions_owner ON upload_sessions(owner_wallet);",
    )
}

/// Video names the backend can play and preview.
pub fn is_video_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".mp4", ".mov", ".m4v", ".webm"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// The container signature, not the name: MP4/MOV/M4V start with an `ftyp`
/// box, WebM with the EBML header.
fn looks_like_video(head: &[u8]) -> bool {
    head.get(4..8) == Some(b"ftyp") || head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

struct Session {
    id: String,
    owner: String,
    name: String,
    declared_size: i64,
    received: i64,
    sha256: Option<String>,
    room: Option<String>,
    clear_epoch: Option<i64>,
    data_epoch: Option<i64>,
    wiki: bool,
    temp_name: String,
}
impl Session {
    fn view(&self) -> serde_json::Value {
        json!({"id":self.id,"offset":self.received,"size":self.declared_size,"chunkSize":SUGGESTED_CHUNK_BYTES})
    }
}

fn read(s: &AppState, id: &str) -> anyhow::Result<Option<Session>> {
    Ok(s.db
        .0
        .lock()
        .unwrap()
        .query_row(
            "SELECT id,owner_wallet,name,declared_size,received,sha256,room_id,clear_epoch,data_epoch,wiki,temp_name FROM upload_sessions WHERE id=?1",
            [id],
            |r| {
                Ok(Session {
                    id: r.get(0)?,
                    owner: r.get(1)?,
                    name: r.get(2)?,
                    declared_size: r.get(3)?,
                    received: r.get(4)?,
                    sha256: r.get(5)?,
                    room: r.get(6)?,
                    clear_epoch: r.get(7)?,
                    data_epoch: r.get(8)?,
                    wiki: r.get::<_, i64>(9)? != 0,
                    temp_name: r.get(10)?,
                })
            },
        )
        .optional()?)
}

/// Load a session and prove the caller owns it. Someone else's session is a
/// plain 404, so a stranger cannot learn that it exists.
fn owned(s: &AppState, id: &str) -> Api<Session> {
    read(s, id)?
        .filter(|session| session.owner == s.db.owner())
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Upload not found".into()))
}

fn uploads_dir(s: &AppState) -> PathBuf {
    s.db.1.join("uploads")
}

/// The part file for a session, with its generated name re-checked before it
/// becomes a path.
fn temp_path(s: &AppState, session: &Session) -> Api<PathBuf> {
    let name = &session.temp_name;
    let ok = name.len() > 5
        && name.ends_with(".part")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        && !name.contains("..");
    if !ok {
        return Err(anyhow::anyhow!("Refusing to touch upload part {name:?}").into());
    }
    Ok(uploads_dir(s).join(name))
}

/// Remove a session's bytes, then its row: a row without a file fails
/// politely, a file without a row is disk nothing reclaims.
async fn discard(s: &AppState, session: &Session) {
    if let Ok(path) = temp_path(s, session) {
        let _ = tokio::fs::remove_file(&path).await;
    }
    let _ =
        s.db.0
            .lock()
            .unwrap()
            .execute("DELETE FROM upload_sessions WHERE id=?1", [&session.id]);
}

fn offset_conflict(session: &Session) -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        format!(
            "Upload is at offset {}; resume from there",
            session.received
        ),
    )
}

/// Lowercase and shape-check a declared digest, so a browser that will fail
/// the comparison finds out before it sends a gigabyte.
fn normalise_digest(raw: Option<&str>) -> Api<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let lower = raw.to_ascii_lowercase();
    ensure!(
        lower.len() == 64 && lower.bytes().all(|b| b.is_ascii_hexdigit()),
        "sha256 must be 64 hexadecimal characters"
    );
    Ok(Some(lower))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Begin {
    name: String,
    size: u64,
    sha256: Option<String>,
    #[serde(alias = "room_id")]
    room_id: Option<String>,
    #[serde(alias = "clear_epoch")]
    clear_epoch: Option<i64>,
    #[serde(alias = "data_epoch")]
    data_epoch: Option<i64>,
    #[serde(default)]
    wiki: bool,
}

/// `POST /api/uploads`
pub async fn begin(PrivateState(s): PrivateState, Json(req): Json<Begin>) -> Api<Response> {
    let name = media::upload_name(&req.name)?;
    ensure!(
        is_video_name(&name),
        "Chunked upload is for MP4, MOV, M4V, or WebM videos"
    );
    ensure!(req.size > 0, "Empty file");
    // Refused before a byte is sent: the reason the size is declared up front.
    ensure!(req.size <= MAX_VIDEO, "Videos must be 1 GB or smaller");
    let sha256 = normalise_digest(req.sha256.as_deref())?;
    {
        let _lock = s.chat_submit.lock().unwrap();
        chat::validate_epoch(
            &s.db,
            req.room_id.as_deref(),
            req.clear_epoch,
            req.data_epoch,
        )?;
        if let Some(room) = &req.room_id {
            chat::get_room(&s.db, room)?;
        }
    }
    sweep(&s).await;
    let owner = s.db.owner().to_string();
    let open: i64 = s.db.0.lock().unwrap().query_row(
        "SELECT COUNT(*) FROM upload_sessions WHERE owner_wallet=?1",
        [&owner],
        |r| r.get(0),
    )?;
    ensure!(
        open < MAX_OPEN_SESSIONS,
        "You already have {MAX_OPEN_SESSIONS} uploads in progress. Finish or cancel one first."
    );
    let id = format!("up_{}", uuid::Uuid::new_v4().simple());
    let temp_name = format!("{id}.part");
    // The file before the row: a row naming a missing file reads as corruption
    // at every append, so the file is removed again if the insert fails.
    let dir = uploads_dir(&s);
    crate::artifacts::private_dir(&dir)?;
    let path = dir.join(&temp_name);
    tokio::fs::File::create(&path)
        .await
        .context("Could not reserve the upload")?;
    let stamp = now();
    let inserted = s.db.0.lock().unwrap().execute(
        "INSERT INTO upload_sessions(id,owner_wallet,name,declared_size,received,sha256,room_id,clear_epoch,data_epoch,wiki,temp_name,created_at,touched_at) VALUES(?1,?2,?3,?4,0,?5,?6,?7,?8,?9,?10,?11,?11)",
        params![id, owner, name, req.size as i64, sha256, req.room_id, req.clear_epoch, req.data_epoch, req.wiki as i64, temp_name, stamp],
    );
    if let Err(error) = inserted {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(anyhow::Error::from(error).into());
    }
    let session = owned(&s, &id)?;
    Ok((StatusCode::CREATED, Json(session.view())).into_response())
}

#[derive(Deserialize)]
pub struct AppendParams {
    offset: Option<u64>,
}

/// `PATCH /api/uploads/{id}?offset=N` with the raw chunk as the body.
pub async fn append(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Query(params): Query<AppendParams>,
    body: Bytes,
) -> Api<Json<serde_json::Value>> {
    let session = owned(&s, &id)?;
    ensure!(!body.is_empty(), "Empty chunk");
    let offset = params
        .offset
        .ok_or_else(|| anyhow::anyhow!("offset is required"))?;
    // The common case, a retried chunk, gets a 409 with the real offset before
    // anything is written. The conditional update below is what makes it safe.
    if offset != session.received as u64 {
        return Err(offset_conflict(&session));
    }
    let end = offset
        .checked_add(body.len() as u64)
        .ok_or_else(|| anyhow::anyhow!("Chunk overflows the file"))?;
    ensure!(
        end <= session.declared_size as u64,
        "Chunk would write past the declared file size"
    );
    let path = temp_path(&s, &session)?;
    // Claim the offset before writing, so two writers that both passed the
    // check above cannot both append.
    let claimed = s.db.0.lock().unwrap().execute(
        "UPDATE upload_sessions SET received=received+?3, touched_at=?4 WHERE id=?1 AND received=?2",
        params![id, session.received, body.len() as i64, now()],
    )? == 1;
    if !claimed {
        return Err(offset_conflict(&owned(&s, &id)?));
    }
    // `append(true)` rather than a seek, so the write cannot land anywhere but
    // the end.
    let written = async {
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await?;
        file.write_all(&body).await?;
        file.flush().await
    }
    .await;
    if let Err(error) = written {
        // Put the offset back: the row promised bytes that are not there.
        let _ = s.db.0.lock().unwrap().execute(
            "UPDATE upload_sessions SET received=?3 WHERE id=?1 AND received=?2",
            params![id, end as i64, session.received],
        );
        return Err(anyhow::Error::from(error)
            .context("Could not save part of this video")
            .into());
    }
    Ok(Json(json!({"offset":end,"size":session.declared_size})))
}

/// `GET /api/uploads/{id}`: where the backend is. Polling keeps it alive.
pub async fn status(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
) -> Api<Json<serde_json::Value>> {
    let session = owned(&s, &id)?;
    s.db.0.lock().unwrap().execute(
        "UPDATE upload_sessions SET touched_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    Ok(Json(session.view()))
}

/// `DELETE /api/uploads/{id}`: give up and give the disk back.
pub async fn abort(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<StatusCode> {
    let session = owned(&s, &id)?;
    discard(&s, &session).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/uploads/{id}/finish`
pub async fn finish(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<FileInfo>> {
    let session = owned(&s, &id)?;
    ensure!(
        session.received == session.declared_size,
        "Upload is incomplete: {} of {} bytes received",
        session.received,
        session.declared_size
    );
    let path = temp_path(&s, &session)?;
    // One pass over the assembled file: the check that covers the wire, the
    // order of the chunks, and the disk together.
    let (digest, size, head) = hash_file(&path).await?;
    if size != session.declared_size as u64 {
        discard(&s, &session).await;
        return Err(anyhow::anyhow!(
            "Assembled upload is {size} bytes, expected {}",
            session.declared_size
        )
        .into());
    }
    if session
        .sha256
        .as_deref()
        .is_some_and(|expected| expected != digest)
    {
        // Keeping bytes known to be wrong would quietly store corruption.
        discard(&s, &session).await;
        return Err(
            anyhow::anyhow!("Uploaded data does not match the declared sha256 checksum").into(),
        );
    }
    let mime = media::mime(&session.name, &head);
    if !looks_like_video(&head) || !mime.starts_with("video/") {
        discard(&s, &session).await;
        return Err(ApiError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "This file is not a video the backend can play".into(),
        ));
    }
    // A failed commit (a room cleared meanwhile) keeps the bytes: the transfer
    // succeeded, and the sweep reclaims them if nobody retries.
    let file = commit(&s, &session, mime, &path)?;
    discard(&s, &session).await;
    crate::after_upload(&s, &file, session.wiki && session.room.is_none());
    Ok(Json(file))
}

fn commit(
    s: &Arc<AppState>,
    session: &Session,
    mime: &str,
    path: &std::path::Path,
) -> Api<FileInfo> {
    let _lock = s.chat_submit.lock().unwrap();
    chat::validate_epoch(
        &s.db,
        session.room.as_deref(),
        session.clear_epoch,
        session.data_epoch,
    )?;
    if let Some(room) = &session.room {
        chat::get_room(&s.db, room)?;
    }
    let file =
        s.db.put_file_from(&session.name, mime, path, session.declared_size as u64)?;
    if let Some(room) = &session.room {
        s.db.assign_room("files", &file.id, room)?;
    }
    Ok(file)
}

/// SHA-256, length, and the first bytes of a file, read in bounded pieces so
/// no code path holds a whole upload.
async fn hash_file(path: &std::path::Path) -> anyhow::Result<(String, u64, Vec<u8>)> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut head = Vec::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if head.len() < 64 {
            let take = (64 - head.len()).min(n);
            head.extend_from_slice(&buf[..take]);
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), total, head))
}

/// Reclaim the disk held by sessions nobody came back for.
pub async fn sweep(s: &AppState) {
    let cutoff = now() - SESSION_TTL_SECONDS;
    let stale: Vec<String> = {
        let conn = s.db.0.lock().unwrap();
        let Ok(mut stmt) =
            conn.prepare("SELECT id FROM upload_sessions WHERE touched_at < ?1 LIMIT 100")
        else {
            return;
        };
        stmt.query_map([cutoff], |r| r.get(0))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    for id in stale {
        if let Ok(Some(session)) = read(s, &id) {
            discard(s, &session).await;
        }
    }
}

/// Sweep at startup and then hourly.
pub fn start_sweeper(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            sweep(&state).await;
            tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_digest_is_normalised_or_refused() {
        assert_eq!(normalise_digest(None).unwrap(), None);
        assert_eq!(normalise_digest(Some("   ")).unwrap(), None);
        assert_eq!(
            normalise_digest(Some(&"A".repeat(64))).unwrap().unwrap(),
            "a".repeat(64)
        );
        assert!(normalise_digest(Some(&"a".repeat(63))).is_err());
        assert!(normalise_digest(Some(&"g".repeat(64))).is_err());
    }

    #[tokio::test]
    async fn hashing_a_file_matches_hashing_it_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.bin");
        let data: Vec<u8> = (0..(1024 * 1024 * 2 + 12345))
            .map(|i| (i % 251) as u8)
            .collect();
        tokio::fs::write(&path, &data).await.unwrap();
        let (digest, size, head) = hash_file(&path).await.unwrap();
        assert_eq!(size, data.len() as u64);
        assert_eq!(digest, hex::encode(Sha256::digest(&data)));
        assert_eq!(head, data[..64]);
    }
}
