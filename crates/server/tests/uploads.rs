//! Chunked video uploads: offset-checked appends, resume after a lost answer,
//! checksum and signature checks at finish, and the stored file living on disk
//! with only its path in SQLite.
use axum::{body::Body, http::Request};
use causewaybay_server::{AppState, app, uploads};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

async fn call(
    router: &axum::Router,
    method: &str,
    url: &str,
    body: Body,
) -> anyhow::Result<(u16, Value)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(url)
                .header("content-type", "application/json")
                .body(body)?,
        )
        .await?;
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn append(
    router: &axum::Router,
    id: &str,
    offset: usize,
    bytes: &[u8],
) -> anyhow::Result<(u16, Value)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/uploads/{id}?offset={offset}"))
                .header("content-type", "application/octet-stream")
                .body(Body::from(bytes.to_vec()))?,
        )
        .await?;
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

/// A QuickTime-shaped payload (`ftyp` box first) of `size` bytes.
fn fake_mov(size: usize) -> Vec<u8> {
    let mut bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    bytes[..12].copy_from_slice(b"\0\0\0\x14ftypqt  ");
    bytes
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[tokio::test]
async fn a_video_is_appended_in_order_survives_a_retried_chunk_and_lands_on_disk()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let video = fake_mov(3 * uploads::SUGGESTED_CHUNK_BYTES + 777);
    let (status, started) = call(
        &router,
        "POST",
        "/api/uploads",
        Body::from(json!({"name":"trip.mov","size":video.len(),"sha256":sha(&video)}).to_string()),
    )
    .await?;
    assert_eq!(status, 201, "{started}");
    assert_eq!(started["offset"], 0);
    let chunk = started["chunkSize"].as_u64().unwrap() as usize;
    assert_eq!(chunk, uploads::SUGGESTED_CHUNK_BYTES);
    let id = started["id"].as_str().unwrap();
    let mut offset = 0;
    while offset < video.len() {
        let end = (offset + chunk).min(video.len());
        let (status, ack) = append(&router, id, offset, &video[offset..end]).await?;
        assert_eq!(status, 200, "{ack}");
        assert_eq!(ack["offset"], end);
        // The answer to the first chunk "is lost" and the browser retries it:
        // the backend refuses with the real offset and writes nothing twice.
        if offset == 0 {
            let (status, conflict) = append(&router, id, 0, &video[..end]).await?;
            assert_eq!(status, 409);
            assert!(
                conflict["error"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("offset {end}"))
            );
            let (_, view) =
                call(&router, "GET", &format!("/api/uploads/{id}"), Body::empty()).await?;
            assert_eq!(view["offset"], end);
        }
        offset = end;
    }
    let (status, file) = call(
        &router,
        "POST",
        &format!("/api/uploads/{id}/finish"),
        Body::empty(),
    )
    .await?;
    assert_eq!(status, 200, "{file}");
    assert_eq!(file["mime"], "video/quicktime");
    assert_eq!(file["size"], video.len());
    // The bytes live under the persistence folder; SQLite holds only the path.
    let path = file["path"].as_str().unwrap();
    assert!(path.starts_with("files/"), "{path}");
    assert_eq!(std::fs::read(dir.path().join(path))?, video);
    let stored: Vec<u8> = state.db.0.lock().unwrap().query_row(
        "SELECT bytes FROM files WHERE id=?1",
        [file["id"].as_str().unwrap()],
        |r| r.get(0),
    )?;
    assert!(stored.is_empty());
    // The session and its part file are gone.
    assert_eq!(std::fs::read_dir(dir.path().join("uploads"))?.count(), 0);
    assert_eq!(
        call(&router, "GET", &format!("/api/uploads/{id}"), Body::empty())
            .await?
            .0,
        404
    );
    // The stored video streams back from disk with Range support.
    let response = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/files/{}/stream",
                file["id"].as_str().unwrap()
            ))
            .header("range", "bytes=10-19")
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "video/quicktime");
    assert_eq!(
        response.into_body().collect().await?.to_bytes().as_ref(),
        &video[10..20]
    );
    Ok(())
}

#[tokio::test]
async fn uploads_refuse_bad_requests_and_corrupt_or_non_video_bytes() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    for body in [
        json!({"name":"notes.txt","size":10}),
        json!({"name":"big.mp4","size":uploads::MAX_VIDEO + 1}),
        json!({"name":"empty.mp4","size":0}),
        json!({"name":"bad.mp4","size":10,"sha256":"xyz"}),
    ] {
        let (status, _) = call(
            &router,
            "POST",
            "/api/uploads",
            Body::from(body.to_string()),
        )
        .await?;
        assert_eq!(status, 400, "{body}");
    }
    // A chunk past the declared size, and finishing early, are refused.
    let video = fake_mov(1000);
    let (_, started) = call(
        &router,
        "POST",
        "/api/uploads",
        Body::from(json!({"name":"a.mp4","size":1000,"sha256":sha(&video)}).to_string()),
    )
    .await?;
    let id = started["id"].as_str().unwrap().to_string();
    assert_eq!(append(&router, &id, 0, &[0; 1001]).await?.0, 400);
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("/api/uploads/{id}/finish"),
            Body::empty()
        )
        .await?
        .0,
        400
    );
    // Bytes that do not match the declared checksum are discarded at finish.
    let mut corrupt = video.clone();
    corrupt[500] ^= 1;
    assert_eq!(append(&router, &id, 0, &corrupt).await?.0, 200);
    let (status, error) = call(
        &router,
        "POST",
        &format!("/api/uploads/{id}/finish"),
        Body::empty(),
    )
    .await?;
    assert_eq!(status, 400);
    assert!(error["error"].as_str().unwrap().contains("checksum"));
    assert_eq!(
        call(&router, "GET", &format!("/api/uploads/{id}"), Body::empty())
            .await?
            .0,
        404
    );
    // A .mp4 name whose bytes are not a video is refused and removed.
    let (_, started) = call(
        &router,
        "POST",
        "/api/uploads",
        Body::from(json!({"name":"fake.mp4","size":5}).to_string()),
    )
    .await?;
    let id = started["id"].as_str().unwrap().to_string();
    assert_eq!(append(&router, &id, 0, b"hello").await?.0, 200);
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("/api/uploads/{id}/finish"),
            Body::empty()
        )
        .await?
        .0,
        415
    );
    assert!(state.db.files()?.is_empty());
    assert_eq!(std::fs::read_dir(dir.path().join("uploads"))?.count(), 0);
    // Abort reclaims the part file at once.
    let (_, started) = call(
        &router,
        "POST",
        "/api/uploads",
        Body::from(json!({"name":"clip.mp4","size":1000}).to_string()),
    )
    .await?;
    let id = started["id"].as_str().unwrap().to_string();
    assert_eq!(std::fs::read_dir(dir.path().join("uploads"))?.count(), 1);
    assert_eq!(
        call(
            &router,
            "DELETE",
            &format!("/api/uploads/{id}"),
            Body::empty()
        )
        .await?
        .0,
        204
    );
    assert_eq!(std::fs::read_dir(dir.path().join("uploads"))?.count(), 0);
    Ok(())
}
