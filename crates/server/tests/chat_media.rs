use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app, db::Database};
use serde_json::{Value, json};
use tower::ServiceExt;
async fn call(r: Router, method: &str, path: &str, v: Value) -> (StatusCode, Value) {
    let res = r
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let b = axum::body::to_bytes(res.into_body(), 2_000_000)
        .await
        .unwrap();
    (status, serde_json::from_slice(&b).unwrap_or_default())
}
#[tokio::test]
async fn room_ids_idempotency_ordered_sync_and_attachment_deletion() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let (_, room) = call(
        r.clone(),
        "POST",
        "/api/rooms",
        json!({"title":"Test conversation"}),
    )
    .await;
    let room = room["roomid"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(room).is_ok());
    let file =
        s.db.put_file("notes.md", "text/markdown", b"# Notes\nRust is useful.")?;
    let message = uuid::Uuid::new_v4().to_string();
    let body = json!({"room_id":room,"message_id":message,"prompt":"Summarize","file_id":file.id});
    let (a, b) = tokio::join!(
        call(r.clone(), "POST", "/api/think", body.clone()),
        call(r.clone(), "POST", "/api/think", body.clone())
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(a.1["id"], b.1["id"]);
    let mut different = body.clone();
    different["agent"] = json!(true);
    assert_eq!(
        call(r.clone(), "POST", "/api/think", different).await.0,
        StatusCode::BAD_REQUEST
    );
    let id = a.1["id"].as_str().unwrap();
    for _ in 0..100 {
        if s.db.get(id)?.unwrap().status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let sync = causewaybay_server::chat::changes(&s.db, room, 0)?;
    assert!(
        sync.messages
            .windows(2)
            .all(|m| m[0].messagesyncid < m[1].messagesyncid)
    );
    let user = sync
        .messages
        .iter()
        .find(|m| m.messageid == message)
        .unwrap();
    assert_eq!(user.attachments[0].path, file.path);
    assert!(
        sync.messages
            .iter()
            .any(|m| m.role == "assistant" && m.status == "complete")
    );
    assert!(
        causewaybay_server::chat::changes(&s.db, room, sync.next)?
            .messages
            .is_empty()
    );
    assert_eq!(
        call(
            r.clone(),
            "DELETE",
            &format!("/api/files/{}", file.id),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(s.db.file(&file.id).is_err());
    assert!(file.path.is_empty() || !dir.path().join(&file.path).exists());
    let removed = causewaybay_server::chat::changes(&s.db, room, sync.next)?;
    assert_eq!(removed.messages.len(), 1);
    assert!(removed.messages[0].attachments.is_empty());
    s.db.set_image(id, b"\x89PNG\r\n\x1a\nfixture")?;
    s.db.event(id, "image", "saved")?;
    let before = causewaybay_server::chat::changes(&s.db, room, removed.next)?.next;
    assert_eq!(
        call(r, "DELETE", &format!("/api/memos/{id}/image"), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let removed = causewaybay_server::chat::changes(&s.db, room, before)?;
    assert!(removed.messages.iter().all(|m| m.image_path.is_none()));
    Ok(())
}
#[test]
fn legacy_blobs_migrate_to_relative_files_and_survive_root_relocation() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let original = temp.path().join("original");
    std::fs::create_dir(&original)?;
    let db = Database::open(&original.join("causewaybay.sqlite3"))?;
    let m = db.create("screen", "capture", "grok", "test")?;
    let bytes = b"\x89PNG\r\n\x1a\nfixture";
    db.0.lock().unwrap().execute(
        "UPDATE memos SET image=?2,image_path=NULL WHERE id=?1",
        rusqlite::params![m.id, bytes],
    )?;
    db.0.lock().unwrap().execute(
        "INSERT INTO files(id,name,mime,bytes,created_at) VALUES(?1,'old.csv','text/csv',?2,'now')",
        rusqlite::params!["00000000-0000-4000-8000-000000000001", b"x,y\n1,2\n"],
    )?;
    drop(db);
    let db = Database::open(&original.join("causewaybay.sqlite3"))?;
    assert_eq!(db.image(&m.id)?.unwrap(), bytes);
    assert_eq!(
        db.file("00000000-0000-4000-8000-000000000001")?.1,
        b"x,y\n1,2\n"
    );
    assert!(!db.image_path(&m.id)?.unwrap().starts_with('/'));
    assert!(
        !db.file_info("00000000-0000-4000-8000-000000000001")?
            .path
            .starts_with('/')
    );
    let lengths:(bool,i64)=db.0.lock().unwrap().query_row("SELECT image IS NULL,(SELECT length(bytes) FROM files WHERE id='00000000-0000-4000-8000-000000000001') FROM memos WHERE id=?1",[&m.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
    assert_eq!(lengths, (true, 0));
    drop(db);
    let moved = temp.path().join("moved");
    std::fs::rename(&original, &moved)?;
    let db = Database::open(&moved.join("causewaybay.sqlite3"))?;
    assert_eq!(db.image(&m.id)?.unwrap(), bytes);
    assert_eq!(
        db.file("00000000-0000-4000-8000-000000000001")?.1,
        b"x,y\n1,2\n"
    );
    assert!(causewaybay_server::artifacts::resolve(&moved, "../outside").is_err());
    Ok(())
}
#[tokio::test]
async fn markdown_csv_utf16_and_binary_processing_are_persisted() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    for (name, bytes, kind, expected) in [
        ("notes.md", b"# Hello".as_slice(), "markdown", "# Hello"),
        (
            "data.csv",
            b"name,value\n\"a,b\",42\n".as_slice(),
            "csv",
            "a,b",
        ),
        (
            "raw.bin",
            b"\x00\xff\x01".as_slice(),
            "binary",
            "no content decoder",
        ),
        ("utf16.txt", b"\xff\xfeH\0i\0".as_slice(), "text", "Hi"),
    ] {
        let f =
            s.db.put_file(name, causewaybay_server::media::mime(name, bytes), bytes)?;
        let (status, v) = call(
            r.clone(),
            "POST",
            &format!("/api/files/{}/process", f.id),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["kind"], kind);
        assert!(v["text"].as_str().unwrap().contains(expected));
        let saved: String = s.db.0.lock().unwrap().query_row(
            "SELECT document FROM file_processing WHERE file_id=?1",
            [&f.id],
            |r| r.get(0),
        )?;
        assert_eq!(serde_json::from_str::<Value>(&saved)?["kind"], kind);
    }
    let long = "a".repeat(100001);
    assert!(causewaybay_server::media::text_or_raw("a.txt", "text/plain", long.as_bytes())?.2);
    Ok(())
}

#[tokio::test]
async fn time_cursors_edits_conflicts_tombstones_and_restart() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(s.clone(), "dist".into());
    let room = call(router.clone(), "POST", "/api/rooms/thought", json!({}))
        .await
        .1;
    let room = room["roomid"].as_str().unwrap();
    let message = uuid::Uuid::new_v4().to_string();
    let req = json!({"room_id":room,"message_id":message,"prompt":"original text"});
    let submitted = call(router.clone(), "POST", "/api/think", req.clone())
        .await
        .1;
    let memo = submitted["id"].as_str().unwrap();
    for _ in 0..150 {
        if s.db.get(memo)?.unwrap().status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    s.db.event(memo, "tool_result", &json!({"name":"web_search","output":json!({"sources":[{"title":"Verified source","url":"https://example.com/source"}]}).to_string()}).to_string())?;
    let before = causewaybay_server::chat::changes(&s.db, room, 0)?;
    assert!(before.messages.iter().any(|m| {
        m.sources
            .iter()
            .any(|s| s.url == "https://example.com/source")
    }));
    let user = before
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .unwrap();
    assert!(user.messagesyncid > 1_700_000_000_000_000);
    let url = format!("/api/rooms/{room}/messages/{message}");
    let edit = json!({"text":"edited private note","messagesyncid":user.messagesyncid});
    let (status, edited) = call(router.clone(), "PATCH", &url, edit.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(edited["sequence"], user.sequence);
    assert_eq!(
        call(router.clone(), "PATCH", &url, edit).await.0,
        StatusCode::CONFLICT
    );
    assert!(!s.db.lexical("edited")?.is_empty());
    let synced = causewaybay_server::chat::changes(&s.db, room, before.next)?;
    assert!(
        synced
            .messages
            .iter()
            .any(|m| m.text == "edited private note")
    );
    let (status, deleted) = call(
        router.clone(),
        "DELETE",
        &url,
        json!({"messagesyncid":edited["messagesyncid"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted["deleted"], true);
    assert_eq!(deleted["text"], "");
    assert!(s.db.lexical("edited")?.is_empty());
    assert_eq!(
        call(router.clone(), "POST", "/api/think", req).await.0,
        StatusCode::BAD_REQUEST
    );
    let last = deleted["messagesyncid"].as_i64().unwrap();
    // Simulate a persisted future timestamp: server must still advance monotonically.
    s.db.0.lock().unwrap().execute("INSERT INTO message_changes(messagesyncid,roomid,messageid,document) VALUES(?1,'clock-test','clock-test','{}')",[last+10_000_000])?;
    let renamed = call(
        router.clone(),
        "PATCH",
        &format!("/api/rooms/{room}"),
        json!({"title":"Renamed room"}),
    )
    .await;
    assert_eq!(renamed.1["title"], "Renamed room");
    assert_eq!(
        call(
            router.clone(),
            "DELETE",
            &format!("/api/rooms/{room}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let replay = causewaybay_server::chat::changes(&s.db, room, last)?;
    // Clear must never turn a deleted room back into a live room.
    assert_eq!(
        call(
            router.clone(),
            "POST",
            &format!("/api/rooms/{room}/clear"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert!(replay.messages.iter().all(|m| m.deleted));
    assert!(replay.next > last + 10_000_000);
    assert!(s.db.get(memo)?.is_none());
    let restored =
        AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let fresh = call(
        app(restored, "dist".into()),
        "POST",
        "/api/rooms/thought",
        json!({}),
    )
    .await
    .1;
    assert_ne!(fresh["roomid"], room);
    Ok(())
}

#[tokio::test]
async fn clear_erases_room_artifacts_search_history_and_resumes_after_crash() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let room = call(r.clone(), "POST", "/api/rooms/thought", json!({}))
        .await
        .1["roomid"]
        .as_str()
        .unwrap()
        .to_string();
    let unrelated = s.db.put_file("keep.txt", "text/plain", b"keep me")?;
    let upload = call(
        r.clone(),
        "POST",
        "/api/files",
        json!({"name":"private.txt","data_base64":"ZXJhc2UgbWU=","room_id":room}),
    )
    .await
    .1;
    let mid = uuid::Uuid::new_v4().to_string();
    let m=call(r.clone(),"POST","/api/think",json!({"room_id":room,"message_id":mid,"prompt":"room-erasure-secret","file_id":upload["id"]})).await.1;
    let id = m["id"].as_str().unwrap();
    for _ in 0..150 {
        if s.db.get(id)?.unwrap().status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    s.db.set_image(id, b"\x89PNG\r\n\x1a\nerase me")?;
    let screenshot = s.db.image_path(id)?.unwrap();
    s.db.put_embedding(id, &[1., 0.])?;
    let generated =
        s.db.put_file("generated.rs", "text/plain", b"private output")?;
    s.db.assign_room("files", &generated.id, &room)?;
    s.db.save_compilation(&generated.id, &json!({"diagnostics":"private diagnostics"}))?;
    s.db.save_compilation_for(
        &unrelated.id,
        &json!({"diagnostics":"room-owned result on unrelated file"}),
        Some(&room),
    )?;
    let note = s.db.create("private generated note", "memo", "local", "")?;
    s.db.assign_room("memos", &note.id, &room)?;
    s.db.finish(&note.id, "memo")?;
    s.db.client_hello("room-device", "Room device", "macOS")?;
    let capture = s.db.queue_capture("room-device", id, "screen", 1)?;
    assert_eq!(
        call(
            r.clone(),
            "POST",
            &format!("/api/rooms/{room}/clear"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    // Text uploads and generated code are rows, so their erasure is the row's;
    // the screenshot's bytes on disk go with it.
    assert_eq!(upload["path"], "");
    assert!(generated.path.is_empty());
    assert!(!dir.path().join(&screenshot).exists(), "{screenshot}");
    assert!(s.db.file_info(upload["id"].as_str().unwrap()).is_err());
    assert!(s.db.get(id)?.is_none());
    assert!(s.db.get(&note.id)?.is_none());
    assert!(s.db.file_info(&generated.id).is_err());
    assert!(s.db.file_info(&unrelated.id).is_ok());
    assert!(s.db.last_compilation(&unrelated.id)?.is_none());
    assert!(s.db.lexical("room-erasure-secret")?.is_empty());
    assert!(s.db.semantic(&[1., 0.])?.is_empty());
    {
        let conn = s.db.0.lock().unwrap();
        let old: i64 = conn.query_row(
            "SELECT COUNT(*) FROM message_changes WHERE document LIKE '%room-erasure-secret%'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(old, 0);
        let submissions: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chat_submissions WHERE messageid=?1",
            [&mid],
            |r| r.get(0),
        )?;
        assert_eq!(submissions, 0);
    }
    assert!(
        causewaybay_server::chat::changes(&s.db, &room, 0)?
            .messages
            .iter()
            .all(|m| m.deleted && m.text.is_empty() && m.memo_id.is_empty())
    );
    assert_eq!(
        call(r.clone(), "POST", "/api/rooms/thought", json!({}))
            .await
            .1["roomid"],
        room
    );
    let events = causewaybay_server::notifications::changes(&s.db, Some("room-device"), 0)?;
    assert!(
        events
            .iter()
            .any(|v| v["kind"] == "capture.remove" && v["data"]["request_id"] == capture)
    );
    let newfile = s.db.put_file(
        "interrupted.txt",
        "text/plain",
        b"private interrupted content",
    )?;
    s.db.assign_room("files", &newfile.id, &room)?;
    s.db.0
        .lock()
        .unwrap()
        .execute("UPDATE rooms SET deleted=3 WHERE roomid=?1", [&room])?;
    // A process restart completes the durable cleanup intent before serving data.
    let restored =
        AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    assert!(newfile.path.is_empty());
    assert!(restored.db.file(&newfile.id).is_err());
    assert!(causewaybay_server::chat::get_room(&restored.db, &room).is_err());
    Ok(())
}

#[tokio::test]
async fn messages_can_be_edited_and_removed_while_capture_is_pending() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    // Demo mode bypasses the capture queue. Use the real pending-capture path;
    // no result is supplied, so this fixture never reaches a cloud provider.
    let s = AppState::open_with_credentials(
        dir.path().into(),
        false,
        None,
        causewaybay_server::credentials::Credentials::from_values([(
            "XAI_API_KEY".into(),
            "pending-capture-test-key".into(),
        )]),
    )?;
    s.db.client_hello("pending-device", "Pending device", "macOS")?;
    let r = app(s.clone(), "dist".into());
    let room = call(r.clone(), "POST", "/api/rooms", json!({})).await.1["roomid"]
        .as_str()
        .unwrap()
        .to_owned();
    let message = uuid::Uuid::new_v4().to_string();
    let (status, memo) = call(
        r.clone(),
        "POST",
        "/api/think",
        json!({"room_id":room,"message_id":message,"capture":true,"client_id":"pending-device","provider":"grok"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let memo = memo["id"].as_str().unwrap();
    let capture = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(command) = s.db.claim_capture("pending-device").unwrap() {
                break command;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(s.db.get(memo)?.unwrap().status, "running");
    let revisions = causewaybay_server::chat::changes(&s.db, &room, 0)?;
    let ai = revisions
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .unwrap();
    let edited = call(
        r.clone(),
        "PATCH",
        &format!("/api/rooms/{room}/messages/{}", ai.messageid),
        json!({"text":"Manually corrected answer","messagesyncid":ai.messagesyncid}),
    )
    .await;
    assert_eq!(edited.0, StatusCode::OK);
    let revisions = causewaybay_server::chat::changes(&s.db, &room, 0)?;
    let user = revisions
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .unwrap();
    assert_eq!(
        call(
            r.clone(),
            "DELETE",
            &format!("/api/rooms/{room}/messages/{}", user.messageid),
            json!({"messagesyncid":user.messagesyncid})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(s.db.get(memo)?.is_none());
    assert!(
        causewaybay_server::chat::changes(&s.db, &room, 0)?
            .messages
            .iter()
            .all(|m| m.deleted)
    );
    assert!(s.db.claim_capture("pending-device")?.is_none());
    assert!(
        s.db.complete_capture(
            "pending-device",
            &capture.request_id,
            Some(b"late capture"),
            None
        )
        .is_err()
    );
    assert!(s.db.get(memo)?.is_none());
    Ok(())
}

#[tokio::test]
async fn room_erasure_cancels_pending_capture_and_rejects_late_completion() -> anyhow::Result<()> {
    for delete in [false, true] {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(
            dir.path().into(),
            false,
            None,
            causewaybay_server::credentials::Credentials::from_values([(
                "XAI_API_KEY".into(),
                "pending-capture-test-key".into(),
            )]),
        )?;
        s.db.client_hello("pending-device", "Pending device", "macOS")?;
        let router = app(s.clone(), "dist".into());
        let room = call(router.clone(), "POST", "/api/rooms", json!({}))
            .await
            .1;
        let room = room["roomid"].as_str().unwrap();
        let (status, memo) = call(
            router.clone(),
            "POST",
            "/api/think",
            json!({
                "room_id":room,"message_id":uuid::Uuid::new_v4().to_string(),
                "capture":true,"client_id":"pending-device","provider":"grok"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let memo = memo["id"].as_str().unwrap();
        let capture = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(command) = s.db.claim_capture("pending-device").unwrap() {
                    break command;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await?;
        assert_eq!(s.db.get(memo)?.unwrap().status, "running");
        let path = if delete {
            format!("/api/rooms/{room}")
        } else {
            format!("/api/rooms/{room}/clear")
        };
        assert_eq!(
            call(
                router,
                if delete { "DELETE" } else { "POST" },
                &path,
                Value::Null
            )
            .await
            .0,
            StatusCode::OK
        );
        assert!(
            s.db.complete_capture(
                "pending-device",
                &capture.request_id,
                Some(b"late capture"),
                None
            )
            .is_err()
        );
        assert!(s.db.get(memo)?.is_none());
        assert!(s.db.claim_capture("pending-device")?.is_none());
        assert!(
            causewaybay_server::chat::changes(&s.db, room, 0)?
                .messages
                .iter()
                .all(|m| m.deleted)
        );
        let notices = causewaybay_server::notifications::changes(&s.db, Some("pending-device"), 0)?;
        assert!(notices.iter().any(
            |v| v["kind"] == "capture.remove" && v["data"]["request_id"] == capture.request_id
        ));
    }
    Ok(())
}

#[tokio::test]
async fn delete_all_erases_only_the_requesting_wallet_including_unfiled_content()
-> anyhow::Result<()> {
    use causewaybay_server::{PrivateState, chat, notifications};
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let other = s.for_owner("another-wallet");
    let other_room = chat::create(
        PrivateState(other.clone()),
        axum::Json(chat::NewRoom {
            roomid: None,
            title: Some("Keep this room".into()),
        }),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.1))?
    .0;
    let other_note = other
        .db
        .create("keep other wallet content", "memo", "local", "")?;
    let other_file = other
        .db
        .put_file("keep.txt", "text/plain", b"keep other wallet bytes")?;
    let other_notifications = notifications::changes(&other.db, None, 0)?;
    let room = call(r.clone(), "POST", "/api/rooms/thought", json!({}))
        .await
        .1["roomid"]
        .as_str()
        .unwrap()
        .to_string();
    let note =
        s.db.create("secret outside any room", "memo", "local", "")?;
    s.db.event(&note.id, "delta", "private standalone answer")?;
    s.db.finish(&note.id, "memo")?;
    s.db.set_image(&note.id, b"\x89PNG\r\n\x1a\nsecret")?;
    s.db.put_embedding(&note.id, &[1., 0.])?;
    let file =
        s.db.put_file("private.txt", "text/plain", b"standalone private file")?;
    s.db.save_compilation(&file.id, &json!({"diagnostics":"secret compilation"}))?;
    let project = causewaybay_server::coding::create_project(&s, "lua", "secret project")?;
    let project_root = causewaybay_server::coding::folder(&s, &project.id);
    std::fs::write(project_root.join("private.txt"), "secret project file")?;
    {
        let conn = s.db.0.lock().unwrap();
        conn.execute("INSERT INTO skill_runs(project_id,source,success,error,created_at) VALUES(?1,'page',0,'secret result','')", [&project.id])?;
        conn.execute(
            "INSERT INTO project_memos(memo_id,project_id,created_at) VALUES(?1,?2,'')",
            rusqlite::params![note.id, project.id],
        )?;
    }
    s.db.client_hello("all-delete-device", "Device", "macOS")?;
    let capture =
        s.db.queue_capture("all-delete-device", &note.id, "screen", 1)?;
    let pending = call(r.clone(), "POST", "/api/think", json!({"room_id":room,"message_id":uuid::Uuid::new_v4().to_string(),"prompt":"secret running turn","agent":true})).await;
    assert_eq!(pending.0, StatusCode::OK);
    let (status, erased) = call(r.clone(), "DELETE", "/api/user/data", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{erased}");
    let local = s.db.for_owner("local");
    assert!(local.all()?.is_empty());
    assert!(local.files()?.is_empty());
    assert!(local.lexical("secret")?.is_empty());
    assert!(local.semantic(&[1., 0.])?.is_empty());
    assert!(file.path.is_empty());
    assert!(local.file(&file.id).is_err());
    assert!(causewaybay_server::coding::list(&s)?.is_empty());
    assert!(
        !project_root.exists(),
        "project files and Git history must be erased"
    );
    assert!(!dir.path().join("markdown").exists());
    assert!(
        !dir.path()
            .join(format!("screenshots/{}.png", note.id))
            .exists()
    );
    assert!(chat::get_room(&local, &room).is_err());
    assert!(chat::get_room(&other.db, &other_room.roomid).is_ok());
    assert!(other.db.get(&other_note.id)?.is_some());
    assert_eq!(other.db.file(&other_file.id)?.1, b"keep other wallet bytes");
    assert_eq!(
        notifications::changes(&other.db, None, 0)?,
        other_notifications
    );
    assert!(
        notifications::changes(&local, None, 0)?
            .iter()
            .any(|v| v["kind"] == "data.deleted"
                && v["data"]["messagesyncid"] == erased["messagesyncid"])
    );
    assert!(
        notifications::changes(&local, Some("all-delete-device"), 0)?
            .iter()
            .any(|v| v["kind"] == "capture.remove" && v["data"]["request_id"] == capture)
    );
    {
        let conn = s.db.0.lock().unwrap();
        for table in [
            "chat_submissions",
            "events",
            "embeddings",
            "compilations",
            "capture_requests",
            "file_processing",
            "user_purges",
            "projects",
            "project_memos",
            "skill_runs",
            "ai_usage",
            "session_activity",
            "wiki_topics",
            "wiki_tags",
            "wiki_pages",
            "code_notes",
        ] {
            assert_eq!(
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))?,
                0,
                "{table}"
            );
        }
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM message_changes WHERE document LIKE '%secret%'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
    }
    // Deleted rooms stay deleted if an overlapping room-clear finishes late.
    chat::finish_purge(&local, &room, false)?;
    assert!(chat::get_room(&local, &room).is_err());
    let new_room = call(r.clone(), "POST", "/api/rooms/thought", json!({}))
        .await
        .1;
    assert_ne!(new_room["roomid"], room);
    assert_eq!(
        call(r, "POST", "/api/memos", json!({"content":"fresh content"}))
            .await
            .0,
        StatusCode::OK
    );
    Ok(())
}

#[tokio::test]
async fn snapshots_stage_without_messages_and_cannot_resurrect_a_cleared_room() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let baseline = s.db.all()?.len();
    let r = app(s.clone(), "dist".into());
    let room = call(r.clone(), "POST", "/api/rooms", json!({})).await.1["roomid"]
        .as_str()
        .unwrap()
        .to_string();
    for _ in 0..2 {
        let (status, file) = call(
            r.clone(),
            "POST",
            &format!("/api/rooms/{room}/snapshot"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{file}");
        assert_eq!(file["mime"], "image/png");
        assert!(dir.path().join(file["path"].as_str().unwrap()).exists());
    }
    assert_eq!(s.db.files()?.len(), 2);
    assert_eq!(s.db.all()?.len(), baseline);
    assert!(
        causewaybay_server::chat::changes(&s.db, &room, 0)?
            .messages
            .is_empty()
    );
    s.db.client_hello("pending-snapshot", "Pending capture", "macOS")?;
    let (pending_router, pending_room) = (r.clone(), room.clone());
    let task = tokio::spawn(async move {
        call(
            pending_router,
            "POST",
            &format!("/api/rooms/{pending_room}/snapshot"),
            json!({"client_id":"pending-snapshot"}),
        )
        .await
    });
    let mut queued = false;
    for _ in 0..100 {
        queued = s.db.0.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM capture_requests WHERE client_id='pending-snapshot')",
            [],
            |r| r.get(0),
        )?;
        if queued {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(queued);
    assert_eq!(
        call(r, "POST", &format!("/api/rooms/{room}/clear"), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_ne!(task.await?.0, StatusCode::OK);
    assert!(s.db.files()?.is_empty());
    assert_eq!(s.db.all()?.len(), baseline);
    Ok(())
}

#[tokio::test]
async fn interrupted_user_erasure_blocks_new_content_and_finishes_on_restart() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    // A picture is the one kind of file with bytes on disk (text is a row).
    let file = s.db.put_file(
        "interrupted-secret.png",
        "image/png",
        b"\x89PNG\r\n\x1a\nprivate content",
    )?;
    let path = dir.path().join(&file.path);
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?; // Force a real filesystem cleanup error.
    let router = app(s.clone(), "dist".into());
    assert_ne!(
        call(router.clone(), "DELETE", "/api/user/data", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert!(s.db.put_file("late.txt", "text/plain", b"late").is_err());
    assert!(s.db.create("late", "memo", "local", "").is_err());
    assert_ne!(
        call(router.clone(), "POST", "/api/rooms", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_ne!(
        call(router, "POST", "/api/rooms/thought", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    std::fs::remove_dir(&path)?;
    let restored =
        AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    assert!(restored.db.files()?.is_empty());
    assert!(restored.db.all()?.is_empty());
    assert!(
        causewaybay_server::notifications::changes(&restored.db, None, 0)?
            .iter()
            .any(|v| v["kind"] == "data.deleted")
    );
    assert!(
        restored
            .db
            .put_file("fresh.txt", "text/plain", b"fresh")
            .is_ok()
    );
    Ok(())
}

#[tokio::test]
async fn requests_started_before_erasure_cannot_recreate_files_or_messages() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(state.clone(), "dist".into());
    let room = call(router.clone(), "POST", "/api/rooms", json!({}))
        .await
        .1["roomid"]
        .as_str()
        .unwrap()
        .to_owned();
    let cleared = call(
        router.clone(),
        "POST",
        &format!("/api/rooms/{room}/clear"),
        json!({}),
    )
    .await
    .1;
    let original = state.db.files()?.len();
    for (path, body) in [
        (
            "/api/files".to_owned(),
            json!({"name":"stale.txt","data_base64":"c2VjcmV0","room_id":room,"clear_epoch":0,"data_epoch":0}),
        ),
        (
            format!("/api/rooms/{room}/snapshot"),
            json!({"clear_epoch":0,"data_epoch":0}),
        ),
        (
            "/api/think".to_owned(),
            json!({"room_id":room,"message_id":uuid::Uuid::new_v4().to_string(),"prompt":"stale","clear_epoch":0,"data_epoch":0}),
        ),
    ] {
        assert_eq!(
            call(router.clone(), "POST", &path, body).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(state.db.files()?.len(), original);
    assert!(
        causewaybay_server::chat::changes(&state.db, &room, 0)?
            .messages
            .is_empty()
    );
    let current = json!({"name":"current.txt","data_base64":"Y3VycmVudA==","room_id":room,"clear_epoch":cleared["messagesyncid"],"data_epoch":0});
    assert_eq!(
        call(router.clone(), "POST", "/api/files", current).await.0,
        StatusCode::OK
    );
    let erased = call(router.clone(), "DELETE", "/api/user/data", json!({}))
        .await
        .1;
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/files",
            json!({"name":"late.txt","data_base64":"c2VjcmV0","data_epoch":0})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert!(state.db.files()?.is_empty());
    assert_eq!(call(router,"POST","/api/files",json!({"name":"fresh.txt","data_base64":"ZnJlc2g=","data_epoch":erased["messagesyncid"]})).await.0,StatusCode::OK);
    Ok(())
}
