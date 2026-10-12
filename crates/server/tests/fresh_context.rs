use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app, chat, db};
use causewaybay_shared::InputMode;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(router: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .clone()
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
async fn post(router: &Router, path: &str, body: Value) -> Value {
    let (status, response) = call(router, "POST", path, body).await;
    assert_eq!(status, StatusCode::OK, "{path}: {response}");
    response
}
async fn finished(state: &AppState, memo: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let memo = state.db.get(memo).unwrap().unwrap();
            if memo.status != "running" {
                assert_eq!(memo.status, "complete", "{:?}", memo.error);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn repeated_new_thoughts_start_empty_and_preserve_previous_rooms_after_restart()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let mut rooms = Vec::new();
    for index in 0..4 {
        let room = post(&router, "/api/rooms/thought", json!({"fresh":true})).await["roomid"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!rooms.contains(&room));
        assert!(chat::changes(&state.db, &room, 0)?.messages.is_empty());
        let memo = post(
            &router,
            "/api/think",
            json!({"room_id":room,"message_id":db::id(),"prompt":format!("problem-{index}")}),
        )
        .await;
        finished(&state, memo["id"].as_str().unwrap()).await;
        rooms.push(room);
    }
    let restored =
        AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(restored.clone(), "dist".into());
    let resumed = post(&router, "/api/rooms/thought", json!({})).await;
    assert_eq!(resumed["roomid"].as_str(), rooms.last().map(String::as_str));
    for (index, room) in rooms.iter().enumerate() {
        let messages = chat::changes(&restored.db, room, 0)?.messages;
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages.iter().find(|m| m.role == "user").unwrap().text,
            format!("problem-{index}")
        );
    }
    let fresh = post(&router, "/api/rooms/thought", json!({"fresh":true})).await;
    assert!(!rooms.iter().any(|room| fresh["roomid"] == *room));
    assert!(
        chat::changes(&restored.db, fresh["roomid"].as_str().unwrap(), 0)?
            .messages
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn pending_old_answer_and_photos_remain_in_their_original_room() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let old = post(&router, "/api/rooms/thought", json!({"fresh":true})).await["roomid"]
        .as_str()
        .unwrap()
        .to_owned();
    let photo = post(&router, &format!("/api/rooms/{old}/snapshot"), json!({})).await;
    let old_memo = post(&router, "/api/think", json!({"room_id":old,"message_id":db::id(),"prompt":"OLD_PROBLEM","input_mode":"image","file_ids":[photo["id"]]})).await;
    assert_eq!(old_memo["status"], "running");
    for endpoint in ["/api/rooms/thought", "/api/rooms"] {
        let body = if endpoint.ends_with("thought") {
            json!({"fresh":true})
        } else {
            json!({"title":"New problem"})
        };
        let fresh = post(&router, endpoint, body).await["roomid"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(old, fresh);
        assert!(chat::changes(&state.db, &fresh, 0)?.messages.is_empty());
        // Even a stale/malicious submission cannot claim another room's photo.
        let rejected = call(&router, "POST", "/api/think", json!({"room_id":fresh,"message_id":db::id(),"input_mode":"image","file_ids":[photo["id"]]})).await;
        assert_eq!(rejected.0, StatusCode::BAD_REQUEST, "{:?}", rejected.1);
        assert!(chat::changes(&state.db, &fresh, 0)?.messages.is_empty());
        let current = post(&router, "/api/think", json!({"room_id":fresh,"message_id":db::id(),"prompt":"NEW_PROBLEM","capture":true,"input_mode":"image"})).await;
        finished(&state, current["id"].as_str().unwrap()).await;
        finished(&state, old_memo["id"].as_str().unwrap()).await;
        let messages = chat::changes(&state.db, &fresh, 0)?.messages;
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m| m.roomid == fresh
            && m.attachments.is_empty()
            && !m.text.contains("OLD_PROBLEM")));
        assert!(
            chat::history(
                &state.db,
                &fresh,
                current["id"].as_str().unwrap(),
                InputMode::Image,
                16
            )
            .await?
            .is_empty()
        );
    }
    let old_messages = chat::changes(&state.db, &old, 0)?.messages;
    assert_eq!(
        old_messages
            .iter()
            .find(|m| m.role == "user")
            .unwrap()
            .attachments[0]
            .id,
        photo["id"]
    );
    assert!(state.db.file(photo["id"].as_str().unwrap()).is_ok());
    Ok(())
}

#[tokio::test]
async fn invalid_reset_requests_do_not_change_the_current_room() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state, "dist".into());
    let original = post(&router, "/api/rooms/thought", json!({"fresh":true})).await;
    for body in [
        json!({"fresh":"true"}),
        json!({"fresh":true,"unknown":true}),
    ] {
        assert!(
            call(&router, "POST", "/api/rooms/thought", body)
                .await
                .0
                .is_client_error()
        );
        assert_eq!(
            post(&router, "/api/rooms/thought", json!({"fresh":false})).await["roomid"],
            original["roomid"]
        );
    }
    Ok(())
}
