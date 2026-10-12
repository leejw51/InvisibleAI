use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app, chat, db, providers::Message};
use causewaybay_shared::{ChatMessage, InputMode, ThinkRequest};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(router: Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
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
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}
fn photo(color: [u8; 3]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(vec![]);
    image::RgbImage::from_pixel(40, 40, image::Rgb(color))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}
fn user_message(db: &db::Database, room: &str, id: &str) -> ChatMessage {
    let doc: String =
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT document FROM chat_messages WHERE roomid=?1 AND messageid=?2",
                rusqlite::params![room, id],
                |r| r.get(0),
            )
            .unwrap();
    serde_json::from_str(&doc).unwrap()
}
fn images(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| {
            if let Message::Image { images, .. } = m {
                images.len()
            } else {
                0
            }
        })
        .sum()
}
fn text(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|m| match m {
            Message::User(text) | Message::Image { text, .. } | Message::Assistant { text, .. } => {
                text.as_str()
            }
            Message::Tool { output, .. } => output.as_str(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn photos_comments_edits_and_deletions_use_current_room_context() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let (_, room) = call(
        r.clone(),
        "POST",
        "/api/rooms",
        json!({"title":"Photo context"}),
    )
    .await;
    let room = room["roomid"].as_str().unwrap();
    let red = s.db.put_file("red.png", "image/png", &photo([255, 0, 0]))?;
    let blue =
        s.db.put_file("blue.png", "image/png", &photo([0, 0, 255]))?;
    let doc =
        s.db.put_file("notes.txt", "text/plain", b"The reference price is 42.")?;
    let message = db::id();
    let body = json!({"room_id":room,"message_id":message,"prompt":"Compare the old samples","input_mode":"image","file_ids":[red.id,blue.id,doc.id]});
    let (status, memo) = call(r.clone(), "POST", "/api/think", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    assert_eq!(
        call(r.clone(), "POST", "/api/think", body.clone()).await.1["id"],
        memo["id"]
    );
    let mut different = body;
    different["file_ids"] = json!([blue.id, red.id, doc.id]);
    assert_eq!(
        call(r.clone(), "POST", "/api/think", different).await.0,
        StatusCode::BAD_REQUEST
    );
    let memo_id = memo["id"].as_str().unwrap();
    // Two photos go through Apple Vision before the answer; give that the
    // same 15 seconds the browser specs allow instead of a 4-second budget.
    for _ in 0..750 {
        if s.db.get(memo_id)?.unwrap().status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let memo = s.db.get(memo_id)?.unwrap();
    assert_eq!(memo.status, "complete", "{:?}", memo.error);
    assert!(memo.input.contains("Photo 1") && memo.input.contains("Photo 2"));
    assert!(memo.input.contains("red.png") && memo.input.contains("blue.png"));
    let m = user_message(&s.db, room, &message);
    assert_eq!(m.attachments.len(), 3);
    assert!(
        m.image_path.is_none(),
        "Uploads render from their files without a duplicate screenshot"
    );

    // A later comment uses original photos, their comment, and the document text.
    let next =
        s.db.create("And compare these", "question", "demo", "test")?;
    s.db.assign_room("memos", &next.id, room)?;
    chat::attach(&s.db, room, &db::id(), &next, &ThinkRequest::default())?;
    let history = chat::history(&s.db, room, &next.id, InputMode::Image, 16).await?;
    assert_eq!(images(&history), 2);
    assert!(text(&history).contains("Compare the old samples"));
    assert!(text(&history).contains("reference price is 42"));
    if let Message::Image { images, .. } = &history[0] {
        assert_ne!(
            images[0].1, images[1].1,
            "Both distinct pictures reach the provider"
        );
    } else {
        panic!("First message must include both photos");
    }
    assert_eq!(
        images(&chat::history(&s.db, room, &next.id, InputMode::Image, 1).await?),
        1
    );
    assert_eq!(
        images(&chat::history(&s.db, room, &next.id, InputMode::Text, 16).await?),
        0
    );

    // Ignore the old generated answer to isolate authoritative user context below.
    let assistant: String = s.db.0.lock().unwrap().query_row(
        "SELECT messageid FROM chat_messages WHERE memo_id=?1 AND role='assistant'",
        [memo_id],
        |r| r.get(0),
    )?;
    let ai = user_message(&s.db, room, &assistant);
    assert_eq!(
        call(
            r.clone(),
            "DELETE",
            &format!("/api/rooms/{room}/messages/{assistant}"),
            json!({"messagesyncid":ai.messagesyncid})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, _) = call(r.clone(), "PATCH", &format!("/api/rooms/{room}/messages/{message}"), json!({"text":"Use the corrected comment","messagesyncid":user_message(&s.db,room,&message).messagesyncid})).await;
    assert_eq!(status, StatusCode::OK);
    let history = chat::history(&s.db, room, &next.id, InputMode::Image, 16).await?;
    assert_eq!(images(&history), 2);
    assert!(text(&history).contains("corrected comment"));
    assert!(!text(&history).contains("old samples"));
    assert_eq!(
        call(
            r.clone(),
            "DELETE",
            &format!("/api/files/{}", red.id),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        images(&chat::history(&s.db, room, &next.id, InputMode::Image, 16).await?),
        1
    );
    assert!(red.path.is_empty() || !dir.path().join(&red.path).exists());
    let current = user_message(&s.db, room, &message);
    assert_eq!(
        call(
            r.clone(),
            "DELETE",
            &format!("/api/rooms/{room}/messages/{message}"),
            json!({"messagesyncid":current.messagesyncid})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        chat::history(&s.db, room, &next.id, InputMode::Image, 16)
            .await?
            .is_empty()
    );
    assert_eq!(
        call(r, "DELETE", &format!("/api/rooms/{room}"), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    for file in [red, blue, doc] {
        assert!(s.db.file_info(&file.id).is_err());
        assert!(file.path.is_empty() || !dir.path().join(&file.path).exists());
    }
    Ok(())
}

#[tokio::test]
async fn attachment_set_claims_are_atomic_and_owned() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let (_, first) = call(r.clone(), "POST", "/api/rooms", json!({"title":"First"})).await;
    let (_, second) = call(r.clone(), "POST", "/api/rooms", json!({"title":"Second"})).await;
    let first = first["roomid"].as_str().unwrap();
    let second = second["roomid"].as_str().unwrap();
    let free = s.db.put_file("free.txt", "text/plain", b"free")?;
    let claimed = s.db.put_file("claimed.txt", "text/plain", b"claimed")?;
    s.db.assign_room("files", &claimed.id, first)?;
    let request = json!({"room_id":second,"message_id":db::id(),"file_ids":[free.id,claimed.id]});
    assert_eq!(
        call(r.clone(), "POST", "/api/think", request).await.0,
        StatusCode::BAD_REQUEST
    );
    let assigned: Option<String> = s.db.0.lock().unwrap().query_row(
        "SELECT roomid FROM files WHERE id=?1",
        [&free.id],
        |r| r.get(0),
    )?;
    assert!(
        assigned.is_none(),
        "The first file must not move if the second fails"
    );
    assert!(chat::changes(&s.db, second, 0)?.messages.is_empty());
    assert!(
        s.db.for_owner("another-wallet")
            .assign_files_to_room(&[&free.id], second)
            .is_err()
    );
    assert!(
        chat::history(
            &s.db.for_owner("another-wallet"),
            first,
            "absent",
            InputMode::Image,
            16
        )
        .await
        .is_err()
    );
    let request = json!({"room_id":second,"message_id":db::id(),"file_ids":vec![free.id;9]});
    assert_eq!(
        call(r, "POST", "/api/think", request).await.0,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}
