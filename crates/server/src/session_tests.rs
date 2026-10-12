use super::*;
use axum::{body::Body, http::Request};
use tower::ServiceExt;

async fn call(s: &Arc<AppState>, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app(s.clone(), "dist".into())
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
fn setup(demo: bool) -> (tempfile::TempDir, Arc<AppState>) {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::open_with_credentials(
        dir.path().into(),
        demo,
        None,
        credentials::Credentials::from_values([("XAI_API_KEY".into(), "fixture-unused".into())]),
    )
    .unwrap()
    .for_owner("local");
    s.db.set_setting("provider", "grok").unwrap();
    // Cloud AI polling is floored at one minute by default; keep fixtures fast.
    s.db.set_setting(
        "local_ai",
        &json!({"cloud_min_interval_seconds":2,"auto_interval_seconds":2,"local_interval_seconds":2}).to_string(),
    )
    .unwrap();
    (dir, s)
}
async fn room(s: &Arc<AppState>) -> String {
    let (status, value) = call(s, "POST", "/api/rooms", json!({"title":"Shared session"})).await;
    assert_eq!(status, StatusCode::OK);
    value["roomid"].as_str().unwrap().into()
}
fn png(color: [u8; 3]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(vec![]);
    image::RgbImage::from_pixel(80, 60, image::Rgb(color))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}
fn captured(s: &Arc<AppState>, room: &str, bytes: &[u8]) -> String {
    let memo = s.db.create("", "capture", "grok", "fixture").unwrap();
    s.db.assign_room("memos", &memo.id, room).unwrap();
    chat::attach(
        &s.db,
        room,
        &db::id(),
        &memo,
        &ThinkRequest {
            capture: true,
            input_mode: InputMode::Image,
            ..Default::default()
        },
    )
    .unwrap();
    s.db.set_image(&memo.id, bytes).unwrap();
    s.db.event(&memo.id, "image", "Screenshot saved").unwrap();
    s.db.finish(&memo.id, "question").unwrap();
    memo.id
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("condition should become true");
}

#[tokio::test]
async fn session_is_a_room_and_saved_memos_are_shared_context() {
    let (_dir, s) = setup(true);
    let (status, session) = call(
        &s,
        "POST",
        "/api/invisible/sessions",
        json!({"title":"Project context"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = session["id"].as_str().unwrap();
    assert_eq!(chat::get_room(&s.db, id).unwrap().title, "Project context");
    let (status, memo) = call(
        &s,
        "POST",
        "/api/memos",
        json!({"room_id":id,"content":"The project codename is tangerine."}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sync = chat::changes(&s.db, id, 0).unwrap();
    assert_eq!(sync.messages.len(), 1);
    assert_eq!(sync.messages[0].provider, "memo");
    assert_eq!(sync.messages[0].memo_id, memo["id"]);
    let next =
        s.db.create("What is the codename?", "question", "grok", "fixture")
            .unwrap();
    s.db.assign_room("memos", &next.id, id).unwrap();
    chat::attach(
        &s.db,
        id,
        &db::id(),
        &next,
        &ThinkRequest {
            prompt: "What is the codename?".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let context = chat::history(&s.db, id, &next.id, InputMode::Text, 0)
        .await
        .unwrap();
    assert!(matches!(&context[0], providers::Message::User(text) if text.contains("tangerine")));
    assert_eq!(invisible_sessions::read(&s, id).unwrap().memo_count, 1);
    let other = room(&s).await;
    assert_eq!(invisible_sessions::read(&s, &other).unwrap().id, other);
    let foreign = s.for_owner("someone-else");
    assert!(
        save_memo(
            PrivateState(foreign),
            Json(json!({"room_id":id,"content":"No access"}))
        )
        .await
        .is_err()
    );
    assert_eq!(
        call(&s, "POST", &format!("/api/rooms/{id}/clear"), json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(invisible_sessions::read(&s, id).unwrap().memo_count, 0);
}

#[tokio::test]
async fn one_capture_node_is_enforced_for_all_entry_points_and_survives_restart() {
    let (dir, s) = setup(true);
    s.db.client_hello("first", "First Mac", "macos").unwrap();
    s.db.client_hello("second", "Second Mac", "macos").unwrap();
    let id = room(&s).await;
    invisible_sessions::register(&s.db, &id, "first").unwrap();
    let capture = format!("/api/rooms/{id}/capture");
    assert!(
        !call(
            &s,
            "POST",
            &capture,
            json!({"client_ids":["first","second"],"interval_seconds":2,"request":{}})
        )
        .await
        .0
        .is_success()
    );
    assert!(
        !call(
            &s,
            "POST",
            &capture,
            json!({"client_ids":["second"],"interval_seconds":2,"request":{}})
        )
        .await
        .0
        .is_success()
    );
    assert!(
        !call(
            &s,
            "POST",
            "/api/think",
            json!({"room_id":id,"message_id":db::id(),"client_id":"second","capture":true})
        )
        .await
        .0
        .is_success()
    );
    assert!(
        !call(
            &s,
            "POST",
            &format!("/api/rooms/{id}/snapshot"),
            json!({"client_id":"second"})
        )
        .await
        .0
        .is_success()
    );
    assert!(invisible_sessions::register(&s.db, &id, "second").is_err());
    call(&s, "POST", &format!("/api/rooms/{id}/clear"), json!({})).await;
    assert_eq!(
        invisible_sessions::bound_client(&s.db, &id)
            .unwrap()
            .as_deref(),
        Some("first")
    );
    let reopened =
        AppState::open_with_credentials(dir.path().into(), true, None, Default::default())
            .unwrap()
            .for_owner("local");
    assert_eq!(
        invisible_sessions::read(&reopened, &id).unwrap().client_id,
        "first"
    );
    assert_eq!(
        continuous::current_status(&reopened, &id)
            .unwrap()
            .client_ids,
        vec!["first"]
    );
}

#[tokio::test]
async fn unchanged_pixels_drop_the_new_file_but_preserve_the_original_history() {
    let (_dir, s) = setup(false);
    let id = room(&s).await;
    let bytes = png([50, 100, 150]);
    let original = captured(&s, &id, &bytes);
    let duplicate = captured(&s, &id, &bytes);
    let path = s
        .data_dir
        .join(s.db.image_path(&duplicate).unwrap().unwrap());
    assert!(
        capture::discard_unchanged(&s.db, &id, &duplicate, bytes.clone())
            .await
            .unwrap()
    );
    assert!(!path.exists());
    assert!(!s.db.get(&duplicate).unwrap().unwrap().has_image);
    assert_eq!(s.db.image(&original).unwrap().unwrap(), bytes);
    let changed = captured(&s, &id, &png([50, 100, 151]));
    assert!(
        !capture::discard_unchanged(&s.db, &id, &changed, png([50, 100, 151]))
            .await
            .unwrap()
    );
    assert!(s.db.get(&changed).unwrap().unwrap().has_image);
    assert_eq!(invisible_sessions::read(&s, &id).unwrap().capture_count, 2);
}

#[tokio::test]
async fn manual_capture_resets_polling_and_unchanged_frames_do_not_create_history() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_dir, s) = setup(false);
    let id = room(&s).await;
    let bytes = png([60, 110, 160]);
    let original = captured(&s, &id, &bytes);
    s.db.client_hello("capture-node", "Capture Mac", "macos")
        .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let commands = count.clone();
    let worker_state = s.clone();
    let worker = tokio::spawn(async move {
        loop {
            if let Some(command) = worker_state.db.claim_capture("capture-node").unwrap() {
                commands.fetch_add(1, Ordering::SeqCst);
                worker_state
                    .db
                    .complete_capture("capture-node", &command.request_id, Some(&bytes), None)
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let path = format!("/api/rooms/{id}/capture");
    assert_eq!(call(&s, "POST", &path, json!({"client_ids":["capture-node"],"interval_seconds":3,"request":{"input_mode":"image"}})).await.0, StatusCode::OK);
    until(|| {
        continuous::current_status(&s, &id)
            .unwrap()
            .next_capture_at
            .is_some()
    })
    .await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let old_deadline = continuous::current_status(&s, &id)
        .unwrap()
        .next_capture_at
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1600)).await;
    assert_eq!(
        call(
            &s,
            "POST",
            &format!("{path}/now"),
            json!({"input_mode":"image"})
        )
        .await
        .0,
        StatusCode::OK
    );
    until(|| {
        count.load(Ordering::SeqCst) == 2
            && continuous::current_status(&s, &id)
                .unwrap()
                .next_capture_at
                .is_some()
    })
    .await;
    let status = continuous::current_status(&s, &id).unwrap();
    assert!(status.next_capture_at.unwrap() > old_deadline);
    assert_eq!(status.unchanged_count, 2);
    tokio::time::sleep(Duration::from_millis(1700)).await;
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "the old polling deadline must be cancelled"
    );
    until(|| count.load(Ordering::SeqCst) == 3).await;
    until(|| continuous::current_status(&s, &id).unwrap().unchanged_count == 3).await;
    call(&s, "DELETE", &path, json!({})).await;
    assert!(!continuous::current_status(&s, &id).unwrap().active);
    assert_eq!(invisible_sessions::read(&s, &id).unwrap().capture_count, 1);
    let live = chat::changes(&s.db, &id, 0)
        .unwrap()
        .messages
        .into_iter()
        .filter(|m| !m.deleted)
        .collect::<Vec<_>>();
    assert_eq!(live.len(), 2);
    assert!(live.iter().all(|m| m.memo_id == original));
    assert_eq!(
        s.db.all().unwrap().len(),
        1,
        "unchanged frames do not leave memos or AI jobs behind"
    );
    worker.abort();
}

#[tokio::test]
async fn older_room_memos_join_history_once_in_their_original_order() {
    let (dir, s) = setup(false);
    let id = room(&s).await;
    let old =
        s.db.create("Earlier TextEdit note", "question", "apple", "system")
            .unwrap();
    s.db.assign_room("memos", &old.id, &id).unwrap();
    s.db.event(&old.id, "delta", "Earlier local answer")
        .unwrap();
    s.db.finish(&old.id, "question").unwrap();
    let newer =
        s.db.create("Later chat question", "question", "grok", "fixture")
            .unwrap();
    s.db.assign_room("memos", &newer.id, &id).unwrap();
    chat::attach(
        &s.db,
        &id,
        &db::id(),
        &newer,
        &ThinkRequest {
            prompt: "Later chat question".into(),
            ..Default::default()
        },
    )
    .unwrap();
    s.db.finish(&newer.id, "question").unwrap();
    let reopened =
        AppState::open_with_credentials(dir.path().into(), false, None, Default::default())
            .unwrap()
            .for_owner("local");
    let messages = chat::changes(&reopened.db, &id, 0).unwrap().messages;
    assert_eq!(messages.len(), 4);
    let context = chat::history(&reopened.db, &id, &newer.id, InputMode::Text, 0)
        .await
        .unwrap();
    assert!(
        matches!(&context[0], providers::Message::User(text) if text == "Earlier TextEdit note")
    );
    assert!(
        matches!(&context[1], providers::Message::Assistant {text,..} if text == "Earlier local answer")
    );
    chat::restore_history(&reopened.db).unwrap();
    assert_eq!(
        chat::changes(&reopened.db, &id, 0).unwrap().messages.len(),
        4
    );
}

#[tokio::test]
async fn manual_capture_waits_for_the_current_frame_and_only_one_request_is_queued() {
    let (_dir, s) = setup(false);
    let id = room(&s).await;
    let bytes = png([90, 80, 70]);
    captured(&s, &id, &bytes);
    s.db.client_hello("busy-node", "Busy capture", "macos")
        .unwrap();
    let path = format!("/api/rooms/{id}/capture");
    assert_eq!(call(&s, "POST", &path, json!({"client_ids":["busy-node"],"interval_seconds":30,"request":{"input_mode":"image"}})).await.0, StatusCode::OK);
    let mut first = None;
    until(|| {
        first = s.db.claim_capture("busy-node").unwrap();
        first.is_some()
    })
    .await;
    assert_eq!(
        call(
            &s,
            "POST",
            &format!("{path}/now"),
            json!({"input_mode":"image"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &s,
            "POST",
            &format!("{path}/now"),
            json!({"input_mode":"image"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(continuous::current_status(&s, &id).unwrap().capture_queued);
    assert!(s.db.claim_capture("busy-node").unwrap().is_none());
    s.db.complete_capture("busy-node", &first.unwrap().request_id, Some(&bytes), None)
        .unwrap();
    let mut second = None;
    until(|| {
        second = s.db.claim_capture("busy-node").unwrap();
        second.is_some()
    })
    .await;
    assert!(!continuous::current_status(&s, &id).unwrap().capture_queued);
    call(&s, "DELETE", &path, json!({})).await;
    assert!(!continuous::current_status(&s, &id).unwrap().active);
    assert!(
        s.db.complete_capture("busy-node", &second.unwrap().request_id, Some(&bytes), None)
            .is_err(),
        "stopped capture cannot publish a late frame"
    );
}

#[test]
fn unchanged_screen_text_is_detected_per_room() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::open(dir.path().into(), true, None)
        .unwrap()
        .for_owner("local");
    let room = chat::create_room(
        &s.db,
        chat::NewRoom {
            roomid: None,
            title: None,
        },
    )
    .unwrap()
    .roomid;
    let other = chat::create_room(
        &s.db,
        chat::NewRoom {
            roomid: None,
            title: None,
        },
    )
    .unwrap()
    .roomid;
    let first = s.db.create("", "capture", "apple", "system").unwrap();
    s.db.assign_room("memos", &first.id, &room).unwrap();
    assert!(!capture::unchanged_text(&s.db, &room, &first.id, "Hello screen").unwrap());
    s.db.event(&first.id, "ocr_text", "Hello screen\n").unwrap();
    assert!(capture::unchanged_text(&s.db, &room, "next", " Hello screen ").unwrap());
    assert!(!capture::unchanged_text(&s.db, &room, "next", "Hello screen 2").unwrap());
    assert!(!capture::unchanged_text(&s.db, &other, "next", "Hello screen").unwrap());
    assert!(!capture::unchanged_text(&s.db, &room, "next", "   ").unwrap());
    // The comparison never matches the memo being processed itself.
    assert!(!capture::unchanged_text(&s.db, &room, &first.id, "Hello screen").unwrap());
}

#[test]
fn processing_summary_counts_ocr_local_cloud_and_skips_per_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::open(dir.path().into(), true, None)
        .unwrap()
        .for_owner("local");
    let room = chat::create_room(
        &s.db,
        chat::NewRoom {
            roomid: None,
            title: None,
        },
    )
    .unwrap()
    .roomid;
    let local = s.db.create("q", "question", "apple", "system").unwrap();
    let cloud = s.db.create("q", "question", "grok", "grok-4").unwrap();
    for memo in [&local, &cloud] {
        s.db.assign_room("memos", &memo.id, &room).unwrap();
    }
    s.db.event(&local.id, "ocr_text", "screen").unwrap();
    {
        let conn = s.db.0.lock().unwrap();
        conn.execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,elapsed_ms,input_tokens,output_tokens,status) VALUES('a',?1,'local','apple','system',?2,900,NULL,NULL,'complete')",rusqlite::params![local.id,db::now()]).unwrap();
        conn.execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,elapsed_ms,input_tokens,output_tokens,status) VALUES('b',?1,'local','grok','grok-4','2020-01-01T00:00:00+00:00',2000,1000000,0,'complete')",[cloud.id.clone()]).unwrap();
        conn.execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,elapsed_ms,status) VALUES('c',?1,'local','grok','grok-4',?2,100,'running')",rusqlite::params![cloud.id,db::now()]).unwrap();
    }
    activity::log(
        &s.db,
        &room,
        "capture",
        "Screen text unchanged · AI skipped",
        "",
        "",
    );
    activity::log(
        &s.db,
        &room,
        "capture",
        "Thinking with your AI",
        "grok",
        "grok-4",
    );
    let summary = activity::summary(&s.db).unwrap();
    assert_eq!(summary.total.ocr_runs, 1);
    assert_eq!(summary.day.ocr_runs, 1);
    assert_eq!(summary.total.skipped_unchanged, 1);
    assert_eq!(summary.total.local.requests, 1);
    assert_eq!(
        summary.total.local.providers,
        vec![("apple".to_string(), 1)]
    );
    assert_eq!(summary.total.cloud.requests, 2);
    assert_eq!(summary.total.cloud.running, 1);
    assert_eq!(summary.total.cloud.estimated_usd, Some(3.0));
    assert_eq!(summary.total.cloud.input_tokens, Some(1_000_000));
    assert_eq!(summary.day.cloud.requests, 1);
    assert_eq!(summary.day.cloud.estimated_usd, None);
    assert_eq!(summary.price_date, activity::PRICE_DATE);
    assert!(!summary.last_at.is_empty());
    let other = activity::summary(&s.for_owner("someone-else").db).unwrap();
    assert_eq!(other.total.local.requests + other.total.cloud.requests, 0);
    assert_eq!(other.total.ocr_runs, 0);
    // The chat document of the cloud reply carries the summed per-call usage.
    chat::attach(&s.db, &room, &db::id(), &cloud, &ThinkRequest::default()).unwrap();
    s.db.finish(&cloud.id, "question").unwrap();
    let message = chat::changes(&s.db, &room, 0)
        .unwrap()
        .messages
        .into_iter()
        .find(|m| m.role == "assistant" && m.memo_id == cloud.id)
        .unwrap();
    let usage = message.usage.unwrap();
    assert_eq!((usage.calls, usage.elapsed_ms), (2, 2100));
    assert_eq!(usage.estimated_usd, Some(3.0));
}
