use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app, chat};
use causewaybay_shared::{ChatMessage, MessageSync, ThinkRequest};
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
    (status, serde_json::from_slice(&bytes).unwrap())
}

// Seed a completed conversation without a background AI job changing its revision.
fn conversation(s: &AppState, room: &str) -> anyhow::Result<Vec<ChatMessage>> {
    let memo =
        s.db.create("original private note", "memo", "local", "fixture")?;
    s.db.assign_room("memos", &memo.id, room)?;
    chat::attach(
        &s.db,
        room,
        &uuid::Uuid::new_v4().to_string(),
        &memo,
        &ThinkRequest {
            prompt: memo.input.clone(),
            ..Default::default()
        },
    )?;
    s.db.finish(&memo.id, "memo")?;
    let changes = chat::changes(&s.db, room, 0)?;
    let mut latest = std::collections::BTreeMap::new();
    for m in changes.messages {
        latest.insert(m.messageid.clone(), m);
    }
    Ok(latest.into_values().collect())
}

#[tokio::test]
async fn paginated_int64_sync_and_erasure_survive_restart() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let (status, room) = call(
        router.clone(),
        "POST",
        "/api/rooms",
        json!({"title":"Cursor test"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let room = room["roomid"].as_str().unwrap();
    // The protocol promises int64, including values that JS Number cannot represent exactly.
    let floor = 9_007_199_254_740_992_i64;
    state
        .db
        .0
        .lock()
        .unwrap()
        .execute("UPDATE sync_clock SET value=?1 WHERE id=1", [floor])?;
    let message = conversation(&state, room)?
        .into_iter()
        .find(|m| m.role == "user")
        .unwrap();
    let sequence = message.sequence;
    let initial_count = chat::changes(&state.db, room, 0)?.messages.len();
    {
        let mut conn = state.db.0.lock().unwrap();
        let tx = conn.transaction()?;
        for i in 0..260 {
            let mut next = message.clone();
            next.messageid = uuid::Uuid::new_v4().to_string();
            next.sequence = 0;
            next.text = format!("private message {i}");
            chat::write(&tx, &mut next)?;
        }
        tx.commit()?;
    }
    let mut after = 0;
    let mut seen = Vec::new();
    let mut pages = 0;
    loop {
        let (status, body) = call(
            router.clone(),
            "GET",
            &format!("/api/rooms/{room}/sync?after={after}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let batch: MessageSync = serde_json::from_value(body)?;
        assert!(batch.messages.len() <= 128);
        assert!(batch.next > after);
        for m in &batch.messages {
            assert!(m.messagesyncid > floor);
            if m.messageid == message.messageid {
                assert_eq!(m.sequence, sequence);
            }
        }
        after = batch.next;
        seen.extend(batch.messages);
        pages += 1;
        if !batch.has_more {
            break;
        }
    }
    assert_eq!(pages, 3);
    assert!(
        seen.windows(2)
            .all(|pair| pair[1].messagesyncid > pair[0].messagesyncid)
    );
    assert_eq!(seen.last().unwrap().text, "private message 259");
    assert_eq!(seen.len(), initial_count + 260);
    assert!(chat::changes(&state.db, room, after)?.messages.is_empty());

    let (status, cleared) = call(
        router.clone(),
        "POST",
        &format!("/api/rooms/{room}/clear"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cutoff = cleared["messagesyncid"].as_i64().unwrap();
    assert!(cutoff > after);
    drop(router);
    drop(state);
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let mut replay = MessageSync::default();
    loop {
        let page = chat::changes(&state.db, room, replay.next)?;
        replay.next = page.next;
        replay.cleared_through = page.cleared_through;
        replay.messages.extend(page.messages);
        if !page.has_more {
            break;
        }
    }
    assert_eq!(replay.messages.len(), initial_count + 260);
    assert_eq!(replay.cleared_through, cutoff);
    assert!(
        replay
            .messages
            .iter()
            .all(|m| m.deleted && m.text.is_empty() && m.memo_id.is_empty())
    );
    assert_eq!(replay.next, cutoff);
    assert!(
        conversation(&state, room)?
            .iter()
            .filter(|m| !m.deleted)
            .all(|m| m.messagesyncid > cutoff)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_edits_have_one_winner_and_stale_writes_cannot_undo_clear()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let room = call(router.clone(), "POST", "/api/rooms", json!({}))
        .await
        .1;
    let room = room["roomid"].as_str().unwrap();
    let messages = conversation(&state, room)?;
    assert_eq!(messages.len(), 2);
    for message in messages {
        let url = format!("/api/rooms/{room}/messages/{}", message.messageid);
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let mut tasks = Vec::new();
        for text in ["first device edit", "second device edit"] {
            let (router, url, barrier) = (router.clone(), url.clone(), barrier.clone());
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                call(
                    router,
                    "PATCH",
                    &url,
                    json!({"text":text,"messagesyncid":message.messagesyncid}),
                )
                .await
            }));
        }
        let a = tasks.remove(0).await?;
        let b = tasks.remove(0).await?;
        let (winner, loser) = if a.0 == StatusCode::OK {
            (a, b)
        } else {
            (b, a)
        };
        assert_eq!(winner.0, StatusCode::OK);
        assert_eq!(loser.0, StatusCode::CONFLICT);
        let latest = chat::changes(&state.db, room, message.messagesyncid)?
            .messages
            .into_iter()
            .rfind(|m| m.messageid == message.messageid)
            .unwrap();
        assert_eq!(latest.text, winner.1["text"].as_str().unwrap());
        assert_eq!(latest.sequence, message.sequence);
        assert_eq!(
            call(
                router.clone(),
                "DELETE",
                &url,
                json!({"messagesyncid":message.messagesyncid})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
    let before = chat::changes(&state.db, room, 0)?;
    assert_eq!(
        call(
            router.clone(),
            "POST",
            &format!("/api/rooms/{room}/clear"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    for message in before.messages {
        let url = format!("/api/rooms/{room}/messages/{}", message.messageid);
        assert_eq!(
            call(
                router.clone(),
                "PATCH",
                &url,
                json!({"text":"late device edit","messagesyncid":message.messagesyncid})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
    assert!(state.db.lexical("device")?.is_empty());
    assert!(
        chat::changes(&state.db, room, 0)?
            .messages
            .iter()
            .all(|m| m.deleted)
    );
    Ok(())
}

#[tokio::test]
async fn clear_empty_room_always_advances_a_durable_int64_barrier() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state =
        AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let room = call(router.clone(), "POST", "/api/rooms", json!({}))
        .await
        .1["roomid"]
        .as_str()
        .unwrap()
        .to_string();
    let first = call(
        router.clone(),
        "POST",
        &format!("/api/rooms/{room}/clear"),
        Value::Null,
    )
    .await
    .1["messagesyncid"]
        .as_i64()
        .unwrap();
    let second = call(
        router,
        "POST",
        &format!("/api/rooms/{room}/clear"),
        Value::Null,
    )
    .await
    .1["messagesyncid"]
        .as_i64()
        .unwrap();
    assert!(first > 0 && second > first);
    let reopened =
        AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    let sync = chat::changes(&reopened.db, &room, first)?;
    assert_eq!(sync.cleared_through, second);
    assert_eq!(sync.next, second);
    assert!(sync.messages.is_empty());
    assert!(
        conversation(&reopened, &room)?
            .iter()
            .all(|m| m.messagesyncid > second)
    );
    Ok(())
}

#[tokio::test]
async fn message_deletion_erases_revisions_replay_artifacts_and_sqlite_payloads()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state =
        AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let room = call(router.clone(), "POST", "/api/rooms", json!({}))
        .await
        .1["roomid"]
        .as_str()
        .unwrap()
        .to_string();
    let secret = "privacyerasureprobe9fb2b2caa80948589393bd05d1542bf1";
    let memo = state.db.create(secret, "memo", "local", "")?;
    state.db.assign_room("memos", &memo.id, &room)?;
    let file = state
        .db
        .put_file("private.txt", "text/plain", secret.as_bytes())?;
    state.db.assign_room("files", &file.id, &room)?;
    let id = uuid::Uuid::new_v4().to_string();
    chat::attach(
        &state.db,
        &room,
        &id,
        &memo,
        &ThinkRequest {
            prompt: secret.into(),
            file_ids: vec![file.id.clone()],
            ..Default::default()
        },
    )?;
    state.db.set_image(&memo.id, b"\x89PNG\r\n\x1a\nprivate")?;
    state.db.finish(&memo.id, "memo")?;
    let original = chat::changes(&state.db, &room, 0)?
        .messages
        .into_iter()
        .find(|m| m.messageid == id)
        .unwrap();
    let url = format!("/api/rooms/{room}/messages/{id}");
    let edited = call(
        router.clone(),
        "PATCH",
        &url,
        json!({"text":"replacement","messagesyncid":original.messagesyncid}),
    )
    .await
    .1;
    assert!(!serde_json::to_string(&chat::changes(&state.db, &room, 0)?)?.contains(secret));
    // A generated answer, file and saved note must not keep the erased prompt alive.
    state.db.event(&memo.id, "delta", secret)?;
    let generated: Value = serde_json::from_str(
        &causewaybay_server::agent::execute(
            &state,
            &memo.id,
            &causewaybay_server::providers::ToolCall {
                id: "file".into(),
                name: "write_file".into(),
                arguments: json!({"name":"generated-private.rs","content":secret}).to_string(),
            },
            &causewaybay_server::agent::definitions(),
        )
        .await?,
    )?;
    let saved: Value = serde_json::from_str(
        &causewaybay_server::agent::execute(
            &state,
            &memo.id,
            &causewaybay_server::providers::ToolCall {
                id: "note".into(),
                name: "save_memo".into(),
                arguments: json!({"title":"Generated note","content":secret}).to_string(),
            },
            &causewaybay_server::agent::definitions(),
        )
        .await?,
    )?;
    let deleted = call(
        router.clone(),
        "DELETE",
        &url,
        json!({"messagesyncid":edited["messagesyncid"]}),
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert!(deleted.1["deleted"] == true && deleted.1["text"] == "" && deleted.1["memo_id"] == "");
    assert!(state.db.file(&file.id).is_err());
    assert!(file.path.is_empty() || !dir.path().join(&file.path).exists());
    assert!(
        state
            .db
            .file(generated["file"]["id"].as_str().unwrap())
            .is_err()
    );
    assert!(state.db.get(saved["saved"].as_str().unwrap())?.is_none());
    assert!(state.db.get(&memo.id)?.is_none());
    assert!(
        chat::changes(&state.db, &room, 0)?
            .messages
            .iter()
            .all(|m| m.deleted)
    );
    assert!(state.db.lexical(secret)?.is_empty());
    let replay = chat::changes(&state.db, &room, original.messagesyncid)?;
    assert_eq!(
        replay.messages.iter().filter(|m| m.messageid == id).count(),
        1
    );
    assert!(
        replay
            .messages
            .iter()
            .find(|m| m.messageid == id)
            .unwrap()
            .deleted
    );
    assert!(
        causewaybay_server::notifications::changes(&state.db, None, 0)?
            .iter()
            .any(|v| v["kind"] == "message.deleted")
    );
    for name in ["causewaybay.sqlite3", "causewaybay.sqlite3-wal"] {
        let path = dir.path().join(name);
        if path.exists() {
            assert!(
                !std::fs::read(path)?
                    .windows(secret.len())
                    .any(|bytes| bytes == secret.as_bytes()),
                "obsolete private bytes in {name}"
            );
        }
    }
    assert_ne!(
        call(
            router,
            "PATCH",
            &url,
            json!({"text":secret,"messagesyncid":original.messagesyncid})
        )
        .await
        .0,
        StatusCode::OK
    );
    Ok(())
}

#[tokio::test]
async fn deleting_replies_and_shared_attachment_turns_preserves_only_live_content()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state =
        AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let room = call(router.clone(), "POST", "/api/rooms", json!({}))
        .await
        .1["roomid"]
        .as_str()
        .unwrap()
        .to_owned();
    let shared = state
        .db
        .put_file("shared.txt", "text/plain", b"explicitly reused attachment")?;
    state.db.assign_room("files", &shared.id, &room)?;
    let mut turns = Vec::new();
    for prompt in ["first prompt", "second prompt"] {
        let memo = state.db.create(prompt, "memo", "local", "fixture")?;
        state.db.assign_room("memos", &memo.id, &room)?;
        chat::attach(
            &state.db,
            &room,
            &uuid::Uuid::new_v4().to_string(),
            &memo,
            &ThinkRequest {
                prompt: prompt.into(),
                file_ids: vec![shared.id.clone()],
                ..Default::default()
            },
        )?;
        state
            .db
            .event(&memo.id, "delta", &format!("reply to {prompt}"))?;
        state.db.finish(&memo.id, "memo")?;
        let messages = chat::changes(&state.db, &room, 0)?.messages;
        let user = messages
            .iter()
            .find(|m| m.memo_id == memo.id && m.role == "user")
            .unwrap()
            .clone();
        let answer = messages
            .into_iter()
            .find(|m| m.memo_id == memo.id && m.role == "assistant")
            .unwrap();
        turns.push((user, answer));
    }
    let (first, answer) = &turns[0];
    let deleted = call(
        router.clone(),
        "DELETE",
        &format!("/api/rooms/{room}/messages/{}", answer.messageid),
        json!({"messagesyncid":answer.messagesyncid}),
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    let latest = chat::changes(&state.db, &room, 0)?;
    assert_eq!(latest.messages.iter().filter(|m| !m.deleted).count(), 3);
    assert!(
        latest
            .messages
            .iter()
            .any(|m| m.messageid == first.messageid && !m.deleted && m.text == "first prompt")
    );
    assert!(!serde_json::to_string(&latest)?.contains("reply to first prompt"));
    assert!(state.db.get(&first.memo_id)?.unwrap().answer.is_empty());
    assert_eq!(
        state.db.file(&shared.id)?.1,
        b"explicitly reused attachment"
    );

    // Removing the first prompt removes its memo, but the second turn owns a live reference.
    let current = latest
        .messages
        .iter()
        .find(|m| m.messageid == first.messageid)
        .unwrap();
    let deleted = call(
        router.clone(),
        "DELETE",
        &format!("/api/rooms/{room}/messages/{}", first.messageid),
        json!({"messagesyncid":current.messagesyncid}),
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert!(state.db.get(&first.memo_id)?.is_none());
    assert!(state.db.file(&shared.id).is_ok());
    let remaining = chat::changes(&state.db, &room, 0)?;
    let live: Vec<_> = remaining.messages.iter().filter(|m| !m.deleted).collect();
    assert_eq!(live.len(), 2);
    assert!(live.iter().all(|m| m.memo_id == turns[1].0.memo_id));
    assert_eq!(
        state.db.file(&shared.id)?.1,
        b"explicitly reused attachment"
    );

    // The final reference erases the attachment and both messages, including after restart.
    let last = live.iter().find(|m| m.role == "user").unwrap();
    let deleted = call(
        router.clone(),
        "DELETE",
        &format!("/api/rooms/{room}/messages/{}", last.messageid),
        json!({"messagesyncid":last.messagesyncid}),
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert!(state.db.get(&turns[1].0.memo_id)?.is_none());
    assert!(state.db.file(&shared.id).is_err());
    assert!(shared.path.is_empty() || !dir.path().join(&shared.path).exists());
    drop(router);
    drop(state);
    let restored =
        AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    let replay = chat::changes(&restored.db, &room, 0)?;
    assert_eq!(replay.messages.len(), 4);
    assert!(replay.messages.iter().all(|m| m.deleted
        && m.text.is_empty()
        && m.attachments.is_empty()
        && m.memo_id.is_empty()));
    assert!(restored.db.files()?.is_empty());
    assert!(restored.db.all()?.is_empty());
    Ok(())
}
