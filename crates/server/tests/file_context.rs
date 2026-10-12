use axum::{Router, body::Body, http::Request};
use causewaybay_server::{AppState, app, chat, db, media, providers::Message};
use causewaybay_shared::{InputMode, ThinkRequest};
use serde_json::{Value, json};
use std::io::{Cursor, Write};
use tower::ServiceExt;

async fn post(router: Router, path: &str, body: Value) -> Value {
    let response = router
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn sound_files_are_detected_without_relying_on_browser_mime() {
    for (name, bytes, expected) in [
        ("voice", b"ID3\x04\0\0".as_slice(), "audio/mpeg"),
        ("voice.mpga", b"\xff\xfb".as_slice(), "audio/mpeg"),
        ("voice.weba", b"\x1a\x45\xdf\xa3".as_slice(), "audio/webm"),
        ("voice.flac", b"fLaC".as_slice(), "audio/flac"),
        ("voice.ogg", b"OggS".as_slice(), "audio/ogg"),
        ("voice.aiff", b"FORM\0\0\0\0AIFF".as_slice(), "audio/aiff"),
        ("notes.markdown", b"# Context".as_slice(), "text/markdown"),
        ("settings.yaml", b"reference: 42".as_slice(), "text/plain"),
        (
            "opaque.data",
            b"\0\xff\x01".as_slice(),
            "application/octet-stream",
        ),
    ] {
        assert_eq!(media::mime(name, bytes), expected, "{name}");
    }
}

#[tokio::test]
async fn markdown_audio_and_arbitrary_files_reach_context_and_are_erased() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let room = post(router.clone(), "/api/rooms", json!({})).await["roomid"]
        .as_str()
        .unwrap()
        .to_owned();
    let markdown = state.db.put_file(
        "brief.md",
        "text/markdown",
        b"# Brief\nLaunch code: ORCHID-48.",
    )?;
    let audio = state
        .db
        .put_file("voice.wav", "audio/wav", b"RIFF\0\0\0\0WAVEfmt ")?;
    let raw = state
        .db
        .put_file("opaque.data", "application/octet-stream", b"\0\xff\x01")?;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )?;
    archive.write_all(b"<document><p>DOCUMENT-FOLLOWUP-734</p></document>")?;
    let docx = state.db.put_file(
        "brief.docx",
        "application/octet-stream",
        &archive.finish()?.into_inner(),
    )?;
    let files = [&markdown, &audio, &raw, &docx];
    let response = post(
        router.clone(),
        "/api/think",
        json!({
            "room_id":room,"message_id":db::id(),"prompt":"A".repeat(9000),
            "file_ids":files.iter().map(|f|&f.id).collect::<Vec<_>>()
        }),
    )
    .await;
    let id = response["id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while state.db.get(id).unwrap().unwrap().status == "running" {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await?;
    let memo = state.db.get(id)?.unwrap();
    assert_eq!(memo.status, "complete", "{:?}", memo.error);
    for expected in [
        "ORCHID-48",
        "What is seven plus five?",
        "opaque.data",
        "no content decoder",
        "DOCUMENT-FOLLOWUP-734",
    ] {
        assert!(memo.input.contains(expected), "{expected}");
    }
    let next = state.db.create("Follow up", "question", "demo", "test")?;
    state.db.assign_room("memos", &next.id, &room)?;
    chat::attach(&state.db, &room, &db::id(), &next, &ThinkRequest::default())?;
    let history = chat::history(&state.db, &room, &next.id, InputMode::Text, 0).await?;
    let context = history
        .iter()
        .filter_map(|message| match message {
            Message::User(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    for expected in [
        "ORCHID-48",
        "What is seven plus five?",
        "opaque.data",
        "no content decoder",
        "DOCUMENT-FOLLOWUP-734",
    ] {
        assert!(
            context.contains(expected),
            "Lost follow-up context: {expected}"
        );
    }
    post(router, &format!("/api/rooms/{room}/clear"), json!({})).await;
    assert!(state.db.get(id)?.is_none());
    for file in files {
        assert!(state.db.file(&file.id).is_err());
        assert!(file.path.is_empty() || !dir.path().join(&file.path).exists());
    }
    let cleared = chat::changes(&state.db, &room, 0)?;
    assert!(cleared.messages.iter().all(|m| m.deleted
        && m.text.is_empty()
        && m.transcript.is_empty()
        && m.attachments.is_empty()));
    Ok(())
}

#[tokio::test]
async fn multiple_large_files_each_receive_context_space() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let mut files = Vec::new();
    for i in 0..8 {
        files.push(state.db.put_file(
            &format!("part-{i}.txt"),
            "text/plain",
            format!("FILE-CONTEXT-{i}\n{}", "한글".repeat(15_000)).as_bytes(),
        )?);
    }
    let response = post(
        router,
        "/api/think",
        json!({
            "prompt":"A".repeat(1_000),
            "file_ids":files.iter().map(|f|&f.id).collect::<Vec<_>>()
        }),
    )
    .await;
    let id = response["id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while state.db.get(id).unwrap().unwrap().status == "running" {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await?;
    let memo = state.db.get(id)?.unwrap();
    assert_eq!(memo.status, "complete", "{:?}", memo.error);
    assert!(memo.input.len() <= 240_000);
    assert!(memo.input.contains("truncated"));
    for i in 0..8 {
        assert!(memo.input.contains(&format!("FILE-CONTEXT-{i}")));
    }
    Ok(())
}
