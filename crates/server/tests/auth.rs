use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app, auth::Auth};
use causewaybay_shared::identity::Identity;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
const ROOT: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const MEMBER: &str = "test test test test test test test test test test test junk";
async fn call(
    router: Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, Value, String) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "localhost:3000")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_default(),
        cookie,
    )
}
async fn login(router: &Router, phrase: &str) -> String {
    login_at(router, phrase, 0, None).await
}
async fn login_at(router: &Router, phrase: &str, index: u32, name: Option<&str>) -> String {
    let id = Identity::from_mnemonic_at(phrase, index).unwrap();
    let (status, c, _) = call(
        router.clone(),
        "POST",
        "/api/auth/challenge",
        "",
        json!({"address":id.address()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let signature = id.sign(c["message"].as_str().unwrap()).unwrap();
    let body = json!({"nonce":c["nonce"],"signature":signature,"wallet_index":index,"name":name});
    let (status, _, cookie) =
        call(router.clone(), "POST", "/api/auth/verify", "", body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(router.clone(), "POST", "/api/auth/verify", "", body)
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "challenge is one-use"
    );
    cookie
}
fn setup(path: &std::path::Path) -> Arc<AppState> {
    let mut state = AppState::open_with_credentials(
        path.into(),
        true,
        Some("test-node-token-012345".into()),
        Default::default(),
    )
    .unwrap();
    Arc::get_mut(&mut state).unwrap().auth =
        Arc::new(Auth::new(Some(Identity::from_mnemonic(ROOT).unwrap().address())).unwrap());
    state
}

#[tokio::test]
async fn wiki_and_assistance_settings_are_private_even_from_admin() {
    let dir = tempfile::tempdir().unwrap();
    let state = setup(dir.path());
    let router = app(state.clone(), "dist".into());
    let admin = login(&router, ROOT).await;
    let member = login(&router, MEMBER).await;
    let address = Identity::from_mnemonic(MEMBER).unwrap().address();
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &admin,
            json!({"role":"user"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(router.clone(), "GET", "/api/wiki", "", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _, _) = call(
        router.clone(),
        "PATCH",
        "/api/assistance/preferences",
        &member,
        json!({"wiki_enabled":true,"textedit_style":"detailed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, created, _) = call(
        router.clone(),
        "POST",
        "/api/memos",
        &member,
        json!({"content":"Only this member's secret #private"}),
    )
    .await;
    // A kept memo joins the Wiki once it has been tidied and judged in the
    // background, so wait for it.
    let mut wiki = Value::Null;
    for _ in 0..200 {
        wiki = call(router.clone(), "GET", "/api/wiki", &member, Value::Null)
            .await
            .1;
        if wiki["total"] == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(wiki["total"], 1, "{wiki}");
    assert!(
        wiki["items"][0]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "private"),
        "{wiki}"
    );
    let id = wiki["tags"][0]["document"].as_str().unwrap();
    assert_eq!(
        call(
            router.clone(),
            "GET",
            &format!("/api/wiki/{id}"),
            &admin,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, prefs, _) = call(
        router.clone(),
        "GET",
        "/api/assistance/preferences",
        &admin,
        Value::Null,
    )
    .await;
    // The admin sees only the defaults (Wiki on, concise answers), never the
    // member's private choices.
    assert_eq!(prefs["wiki_enabled"], true);
    assert_eq!(prefs["textedit_style"], "concise");
    assert_eq!(
        call(
            router.clone(),
            "DELETE",
            &format!("/api/memos/{}", created["id"].as_str().unwrap()),
            &member,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(router, "GET", "/api/wiki", &member, Value::Null)
            .await
            .1["total"],
        0
    );
}

#[tokio::test]
async fn same_wallet_rooms_and_current_thought_replay_over_sse_after_restart() -> anyhow::Result<()>
{
    use causewaybay_server::notifications;
    use futures_util::StreamExt;

    let dir = tempfile::tempdir()?;
    let state = setup(dir.path());
    let router = app(state.clone(), "dist".into());
    let first = login(&router, ROOT).await;
    let second = login(&router, ROOT).await;
    assert_ne!(first, second);
    let member = login(&router, MEMBER).await;
    let address = Identity::from_mnemonic(MEMBER).unwrap().address();
    call(
        router.clone(),
        "POST",
        "/api/auth/apply",
        &member,
        json!({"name":"Other wallet"}),
    )
    .await;
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &first,
            json!({"role":"user"})
        )
        .await
        .0,
        StatusCode::OK
    );
    for cookie in [&first, &second, &member] {
        let (status, current, _) = call(
            router.clone(),
            "GET",
            "/api/rooms/thought",
            cookie,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            current.is_null(),
            "Reading the current thought must not create one"
        );
    }
    let room_id = uuid::Uuid::new_v4().to_string();
    for _ in 0..2 {
        assert_eq!(
            call(
                router.clone(),
                "POST",
                "/api/rooms",
                &first,
                json!({"roomid":room_id,"title":"Shared room"})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/rooms/{room_id}"),
            &first,
            json!({"title":"Renamed on another device"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let old = call(
        router.clone(),
        "POST",
        "/api/rooms/thought",
        &first,
        json!({"fresh":true}),
    )
    .await
    .1;
    let latest = call(
        router.clone(),
        "POST",
        "/api/rooms/thought",
        &second,
        json!({"fresh":true}),
    )
    .await
    .1;
    assert_ne!(old["roomid"], latest["roomid"]);
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/rooms/thought",
            &first,
            Value::Null
        )
        .await
        .1,
        latest
    );
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/rooms/thought",
            &second,
            json!({})
        )
        .await
        .1,
        latest
    );
    assert!(
        call(
            router.clone(),
            "GET",
            "/api/rooms/thought",
            &member,
            Value::Null
        )
        .await
        .1
        .is_null()
    );
    assert!(
        call(router.clone(), "GET", "/api/rooms", &member, Value::Null)
            .await
            .1
            .as_array()
            .unwrap()
            .is_empty()
    );
    let owner = state
        .db
        .for_owner(&Identity::from_mnemonic(ROOT).unwrap().address());
    let events = notifications::changes(&owner, None, 0)?;
    assert_eq!(
        events
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "room.created",
            "room.renamed",
            "room.thought",
            "room.thought"
        ]
    );
    assert_eq!(events.last().unwrap()["data"]["roomid"], latest["roomid"]);
    assert!(notifications::changes(&state.db.for_owner(&address), None, 0)?.is_empty());

    drop(owner);
    drop(router);
    drop(state);
    let restored = setup(dir.path());
    let router = app(restored, "dist".into());
    let cookie = login(&router, ROOT).await;
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/rooms/thought",
            &cookie,
            Value::Null
        )
        .await
        .1,
        latest
    );
    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/notifications?after=9223372036854775807")
                .header("cookie", cookie)
                .header("last-event-id", events[1]["id"].to_string())
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut stream = response.into_body().into_data_stream();
    let replay = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut buffer = String::new();
        let mut received = Vec::new();
        while received.len() < 2 {
            buffer.push_str(std::str::from_utf8(&stream.next().await.unwrap().unwrap()).unwrap());
            while let Some(end) = buffer.find("\n\n") {
                let frame = buffer[..end].to_owned();
                buffer.drain(..end + 2);
                if let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data: ")) {
                    let event: Value = serde_json::from_str(data).unwrap();
                    assert!(
                        frame
                            .lines()
                            .any(|line| line == format!("id: {}", event["id"]))
                    );
                    received.push(event);
                }
            }
        }
        received
    })
    .await?;
    // Resuming from a cursor is catching up on missed news, not on history:
    // nothing is flagged as a replay.
    assert_eq!(replay, events[2..]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_device_resets_have_one_shared_current_thought() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = setup(dir.path());
    let router = app(state.clone(), "dist".into());
    let first = login(&router, ROOT).await;
    let second = login(&router, ROOT).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut resets = Vec::new();
    for cookie in [first.clone(), second.clone()] {
        let (router, barrier) = (router.clone(), barrier.clone());
        resets.push(tokio::spawn(async move {
            barrier.wait().await;
            call(
                router,
                "POST",
                "/api/rooms/thought",
                &cookie,
                json!({"fresh":true}),
            )
            .await
        }));
    }
    let a = resets.remove(0).await?;
    let b = resets.remove(0).await?;
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert_ne!(a.1["roomid"], b.1["roomid"]);
    let owner = state
        .db
        .for_owner(&Identity::from_mnemonic(ROOT).unwrap().address());
    let events = causewaybay_server::notifications::changes(&owner, None, 0)?;
    assert_eq!(events.len(), 2);
    for cookie in [&first, &second] {
        let current = call(
            router.clone(),
            "GET",
            "/api/rooms/thought",
            cookie,
            Value::Null,
        )
        .await
        .1;
        assert_eq!(current["roomid"], events.last().unwrap()["data"]["roomid"]);
        assert!(
            causewaybay_server::chat::changes(&owner, current["roomid"].as_str().unwrap(), 0)?
                .messages
                .is_empty()
        );
    }
    Ok(())
}

#[tokio::test]
async fn notification_sse_replays_pages_privately_and_stops_on_logout() -> anyhow::Result<()> {
    use causewaybay_server::notifications;
    use futures_util::StreamExt;
    use std::time::Duration;

    let dir = tempfile::tempdir()?;
    let state = setup(dir.path());
    let router = app(state.clone(), "dist".into());
    let cookie = login(&router, ROOT).await;
    let root = state
        .db
        .for_owner(&Identity::from_mnemonic(ROOT).unwrap().address());
    let member = state
        .db
        .for_owner(&Identity::from_mnemonic(MEMBER).unwrap().address());
    for index in 0..131 {
        notifications::wallet(
            &root,
            None,
            "work.progress",
            json!({"index":index,"text":"私の仕事"}),
        )?;
        notifications::wallet(
            &member,
            None,
            "private.member",
            json!({"secret":"other wallet"}),
        )?;
        notifications::client(
            &state,
            "private-device",
            "private.client",
            json!({"secret":"device only"}),
        )?;
    }
    let first = notifications::changes(&root, None, 0)?[0]["id"]
        .as_i64()
        .unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                // Last-Event-ID must take precedence over a stale URL cursor.
                .uri("/api/notifications?after=9223372036854775807")
                .header("cookie", &cookie)
                .header("last-event-id", first.to_string())
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut stream = response.into_body().into_data_stream();
    let mut buffer = String::new();
    let mut received = Vec::new();
    let mut last = first;
    while received.len() < 130 {
        let chunk = tokio::time::timeout(Duration::from_secs(3), stream.next())
            .await?
            .expect("SSE ended before replay completed")?;
        buffer.push_str(std::str::from_utf8(&chunk)?);
        while let Some(end) = buffer.find("\n\n") {
            let frame = buffer[..end].to_owned();
            buffer.drain(..end + 2);
            if let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data: ")) {
                let event: Value = serde_json::from_str(data)?;
                assert_eq!(event["kind"], "work.progress");
                assert_eq!(event["data"]["text"], "私の仕事");
                let id = event["id"].as_i64().unwrap();
                assert!(id > last);
                assert!(frame.lines().any(|line| line == format!("id: {id}")));
                last = id;
                received.push(event["data"]["index"].as_u64().unwrap());
            }
        }
    }
    assert_eq!(received, (1..131).collect::<Vec<_>>());
    assert_eq!(
        call(router, "POST", "/api/auth/logout", &cookie, json!({}))
            .await
            .0,
        StatusCode::OK
    );
    notifications::wallet(&root, None, "work.progress", json!({"index":131}))?;
    // An already-open stream must recheck its session, not leak a final notification.
    assert!(
        tokio::time::timeout(Duration::from_secs(3), stream.next())
            .await?
            .is_none()
    );
    Ok(())
}
#[tokio::test]
async fn registration_approval_promotion_revocation_and_jsonl_restore() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = setup(dir.path());
    let router = app(s.clone(), "dist".into());
    assert_eq!(
        call(router.clone(), "GET", "/api/settings", "", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let member = login(&router, MEMBER).await;
    let address = Identity::from_mnemonic(MEMBER).unwrap().address();
    let info = call(
        router.clone(),
        "GET",
        "/api/auth/info",
        &member,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(info["authenticated"], true);
    assert_eq!(info["role"], "pending");
    assert_eq!(info["approved"], false);
    assert_eq!(
        call(router.clone(), "GET", "/api/rooms", &member, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/auth/apply",
            &member,
            json!({"name":"Test member"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/auth/info",
            &member,
            Value::Null
        )
        .await
        .1["role"],
        "pending"
    );
    let root = login(&router, ROOT).await;
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/admin/users",
            &member,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &root,
            json!({"role":"admin"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &root,
            json!({"role":"user"})
        )
        .await
        .0,
        StatusCode::OK
    );
    // Model downloads are server setup too: a member can look but not fetch.
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/local-ai/models",
            &member,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    for (method, path) in [
        ("POST", "/api/admin/local-ai/models/fetch"),
        ("DELETE", "/api/admin/local-ai/models/fetch"),
    ] {
        assert_eq!(
            call(
                router.clone(),
                method,
                path,
                &member,
                json!({"id":"qwen3.5-4b"})
            )
            .await
            .0,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }
    for body in [
        json!({"provider":"mlx","local_ai":{"mlx_enabled":true,"mlx_url":"http://127.0.0.1:8080/v1"}}),
        json!({"provider":"apple","local_ai":{"apple_enabled":true,"speech_provider":"apple"}}),
        json!({"local_ai":{"local_only":false}}),
    ] {
        assert_eq!(
            call(router.clone(), "PATCH", "/api/settings", &member, body)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(router.clone(), "GET", "/api/settings", &member, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            "/api/settings",
            &member,
            json!({"display":2})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &root,
            json!({"role":"admin"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            "/api/settings",
            &member,
            json!({"display":2})
        )
        .await
        .0,
        StatusCode::OK
    );
    let owner = Identity::from_mnemonic(ROOT).unwrap().address();
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{owner}"),
            &member,
            json!({"role":"blocked"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &root,
            json!({"role":"blocked"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(router.clone(), "GET", "/api/files", &member, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/auth/apply",
            &member,
            json!({"name":"Bypass"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let records = std::fs::read_to_string(dir.path().join("users.jsonl"))?;
    assert_eq!(records.lines().count(), 5);
    assert!(!records.contains(MEMBER));
    let last: Value = serde_json::from_str(records.lines().last().unwrap())?;
    assert_eq!(last["config"]["users"][0]["role"], "blocked");
    s.db.0
        .lock()
        .unwrap()
        .execute("UPDATE users SET role='admin' WHERE address=?1", [&address])?;
    causewaybay_server::users::restore(&s)?;
    assert_eq!(causewaybay_server::users::role(&s, &address)?, "blocked");
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/auth/logout",
            &root,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(router.clone(), "GET", "/api/settings", &root, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let conn = s.db.0.lock().unwrap();
    let hashes: Vec<String> = conn
        .prepare("SELECT token_hash FROM auth_sessions")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    assert!(hashes.iter().all(|h| h.len() == 64 && !member.contains(h)));
    Ok(())
}
#[tokio::test]
async fn owner_wallet_restricts_single_batch_and_continuous_capture() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = setup(dir.path());
    let router = app(s.clone(), "dist".into());
    let root = login(&router, ROOT).await;
    let member = login(&router, MEMBER).await;
    let address = Identity::from_mnemonic(MEMBER).unwrap().address();
    call(
        router.clone(),
        "POST",
        "/api/auth/apply",
        &member,
        json!({"name":"Owner"}),
    )
    .await;
    call(
        router.clone(),
        "PATCH",
        &format!("/api/admin/users/{address}"),
        &root,
        json!({"role":"user"}),
    )
    .await;
    s.db.client_hello("private", "Private Mac", "macOS")?;
    s.db.client_hello("shared", "Shared Mac", "macOS")?;
    s.db.0.lock().unwrap().execute(
        "UPDATE capture_clients SET owner_wallet=?1 WHERE id='private'",
        [&address],
    )?;
    let clients = call(router.clone(), "GET", "/api/clients", &root, Value::Null)
        .await
        .1;
    assert!(
        clients
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != "private")
    );
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/think",
            &root,
            json!({"capture":true,"client_id":"private"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/think",
            &member,
            json!({"capture":true,"client_id":"private"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let all = call(
        router.clone(),
        "POST",
        "/api/capture/batch",
        &root,
        json!({"all":true,"request":{"input_mode":"text"}}),
    )
    .await;
    assert_eq!(all.0, StatusCode::OK);
    assert_eq!(all.1.as_array().unwrap().len(), 1);
    assert_eq!(all.1[0]["client"]["id"], "shared");
    let room = call(router.clone(), "POST", "/api/rooms", &member, json!({}))
        .await
        .1;
    let path = format!("/api/rooms/{}/capture", room["roomid"].as_str().unwrap());
    assert_eq!(
        call(
            router.clone(),
            "POST",
            &path,
            &root,
            json!({"client_ids":["private"],"interval_seconds":2,"request":{}})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            router.clone(),
            "POST",
            &path,
            &member,
            json!({"client_ids":["private"],"interval_seconds":2,"request":{}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(router, "DELETE", &path, &member, Value::Null).await.1["active"],
        false
    );
    Ok(())
}
#[tokio::test]
async fn challenge_rejects_bad_signatures_cross_origin_and_node_token_admin_access()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = setup(dir.path());
    let router = app(s, "dist".into());
    let id = Identity::from_mnemonic(ROOT).unwrap();
    let c = call(
        router.clone(),
        "POST",
        "/api/auth/challenge",
        "",
        json!({"address":id.address()}),
    )
    .await
    .1;
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/auth/verify",
            "",
            json!({"nonce":c["nonce"],"signature":"0xdead"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let good =
        json!({"nonce":c["nonce"],"signature":id.sign(c["message"].as_str().unwrap()).unwrap()});
    let cross = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/verify")
                .header("host", "localhost:3000")
                .header("origin", "https://evil.example")
                .header("content-type", "application/json")
                .body(Body::from(good.to_string()))?,
        )
        .await?;
    assert_eq!(cross.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        call(router.clone(), "POST", "/api/auth/verify", "", good)
            .await
            .0,
        StatusCode::OK,
        "invalid attempts do not burn valid challenge"
    );
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/settings")
                .header("authorization", "Bearer test-node-token-012345")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        call(
            router,
            "POST",
            "/api/session",
            "",
            json!({"token":"test-node-token-012345"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(Auth::new(Some("not-a-wallet".into())).is_err());
    Ok(())
}

#[tokio::test]
async fn private_wallet_data_isolated_even_from_admin_and_ai_tools() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = setup(dir.path());
    let router = app(s.clone(), "dist".into());
    let root = login(&router, ROOT).await;
    let member = login(&router, MEMBER).await;
    let address = Identity::from_mnemonic(MEMBER).unwrap().address();
    call(
        router.clone(),
        "POST",
        "/api/auth/apply",
        &member,
        json!({}),
    )
    .await;
    call(
        router.clone(),
        "PATCH",
        &format!("/api/admin/users/{address}"),
        &root,
        json!({"role":"user"}),
    )
    .await;
    let owner = s.for_owner(&address);
    let root_owner = s.for_owner(&Identity::from_mnemonic(ROOT).unwrap().address());
    s.db.set_setting("draft", "legacy private draft")?;
    for cookie in [&root, &member] {
        assert_eq!(
            call(router.clone(), "GET", "/api/settings", cookie, Value::Null)
                .await
                .1["draft"],
            ""
        );
    }
    causewaybay_server::notifications::wallet(
        &owner.db,
        None,
        "test.private",
        json!({"private":"payload"}),
    )?;
    assert!(causewaybay_server::notifications::changes(&root_owner.db, None, 0)?.is_empty());
    assert_eq!(
        causewaybay_server::notifications::changes(&owner.db, None, 0)?[0]["data"]["private"],
        "payload"
    );
    let (_, memo, _) = call(
        router.clone(),
        "POST",
        "/api/memos",
        &member,
        json!({"content":"tangerine-private-needle #private"}),
    )
    .await;
    let id = memo["id"].as_str().unwrap();
    owner.db.set_image(id, b"\x89PNG\r\n\x1a\nprivate")?;
    // Tidying changes searchable content and correctly invalidates old vectors.
    // Install this synthetic embedding only after that background work finishes.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while owner.db.get(id).unwrap().unwrap().kind != "note" {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    owner.db.put_embedding(id, &[1., 0.])?;
    let (_, file, _) = call(
        router.clone(),
        "POST",
        "/api/files",
        &member,
        json!({"name":"private.txt","data_base64":"cHJpdmF0ZQ=="}),
    )
    .await;
    let fid = file["id"].as_str().unwrap();
    let (_, room, _) = call(
        router.clone(),
        "POST",
        "/api/rooms/thought",
        &member,
        json!({}),
    )
    .await;
    let rid = room["roomid"].as_str().unwrap();
    let again = call(
        router.clone(),
        "POST",
        "/api/rooms/thought",
        &member,
        json!({}),
    )
    .await
    .1;
    assert_eq!(again["roomid"], room["roomid"]);
    assert_ne!(
        call(
            router.clone(),
            "POST",
            "/api/rooms/thought",
            &root,
            json!({})
        )
        .await
        .1["roomid"],
        room["roomid"]
    );
    for path in [
        format!("/api/memos/{id}"),
        format!("/api/memos/{id}/image"),
        format!("/api/memos/{id}/thumbnail"),
        format!("/api/memos/{id}/events"),
        format!("/api/memos/{id}/export"),
        format!("/api/files/{fid}/download"),
        format!("/api/files/{fid}/thumbnail"),
        format!("/api/files/{fid}/image"),
        format!("/api/files/{fid}/text"),
        format!("/api/rooms/{rid}/sync"),
        format!("/api/rooms/{rid}/events"),
        format!("/api/rooms/{rid}/capture"),
    ] {
        assert_eq!(
            call(router.clone(), "GET", &path, &root, Value::Null)
                .await
                .0,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    for (method, path) in [
        ("DELETE", format!("/api/memos/{id}")),
        ("POST", format!("/api/memos/{id}/cancel")),
        ("DELETE", format!("/api/files/{fid}")),
        ("POST", format!("/api/files/{fid}/process")),
        ("PATCH", format!("/api/rooms/{rid}")),
        ("DELETE", format!("/api/rooms/{rid}")),
    ] {
        assert_eq!(
            call(router.clone(), method, &path, &root, json!({}))
                .await
                .0,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    for payload in [
        json!({"prompt":"read this","file_id":fid}),
        json!({"prompt":"read these","file_ids":[fid]}),
        json!({"prompt":"continue","parent_id":id}),
        json!({"prompt":"steal room","room_id":rid,"message_id":uuid::Uuid::new_v4().to_string()}),
    ] {
        assert!(
            !call(router.clone(), "POST", "/api/think", &root, payload)
                .await
                .0
                .is_success()
        );
    }
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/memos?q=tangerine-private-needle",
            &root,
            Value::Null
        )
        .await
        .1["total"],
        0
    );
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/memos?q=tangerine-private-needle",
            &member,
            Value::Null
        )
        .await
        .1["total"],
        1
    );
    assert!(root_owner.db.lexical("tangerine")?.is_empty());
    assert!(root_owner.db.semantic(&[1., 0.])?.is_empty());
    assert_eq!(owner.db.semantic(&[1., 0.])?, vec![id]);
    assert!(root_owner.db.file(fid).is_err());
    assert!(root_owner.db.get(id)?.is_none());
    let tool = causewaybay_server::providers::ToolCall {
        name: "read_file".into(),
        arguments: json!({"file_id":fid}).to_string(),
        ..Default::default()
    };
    assert!(
        causewaybay_server::agent::execute(
            &root_owner,
            "unused",
            &tool,
            &causewaybay_server::agent::definitions(),
        )
        .await
        .is_err()
    );
    assert_eq!(
        call(router.clone(), "GET", "/api/files", &root, Value::Null)
            .await
            .1,
        json!([])
    );
    assert_eq!(
        call(router.clone(), "GET", "/api/images", &root, Value::Null)
            .await
            .1,
        json!([])
    );
    let name = call(
        router.clone(),
        "GET",
        "/api/auth/info",
        &member,
        Value::Null,
    )
    .await
    .1["name"]
        .clone();
    assert_eq!(
        name,
        causewaybay_shared::identity::username(&address).unwrap()
    );
    Ok(())
}

#[tokio::test]
async fn status_scopes_content_and_package_uses_authenticated_wallet() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = setup(dir.path());
    let root = Identity::from_mnemonic(ROOT).unwrap().address();
    let member = Identity::from_mnemonic(MEMBER).unwrap().address();
    s.addresses
        .set(vec![causewaybay_shared::BackendAddress {
            label: "Tailscale IP".into(),
            web_url: "http://100.64.0.20:3000".into(),
        }])
        .unwrap();
    let router = app(s.clone(), "dist".into());
    let admin = login(&router, ROOT).await;
    let cookie = login(&router, MEMBER).await;
    call(
        router.clone(),
        "POST",
        "/api/auth/apply",
        &cookie,
        json!({"name":"Member"}),
    )
    .await;
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{member}"),
            &admin,
            json!({"role":"user"})
        )
        .await
        .0,
        StatusCode::OK
    );
    s.for_owner(&root)
        .db
        .put_file("admin-private.txt", "text/plain", b"private admin data")?;
    s.for_owner(&member)
        .db
        .put_file("member-private.txt", "text/plain", b"member data")?;
    s.for_owner(&member)
        .db
        .create("member secret", "memo", "local", "test")?;
    s.for_owner(&member)
        .db
        .web_connect("member-web", "Member browser", "test", "connected")?;
    let (_, mine, _) = call(router.clone(), "GET", "/api/status", &cookie, json!(null)).await;
    assert_eq!(mine["stats"]["files"], 1);
    assert_eq!(mine["stats"]["file_bytes"], 11);
    assert_eq!(mine["stats"]["memos"], 1);
    assert_eq!(mine["stats"]["web_online"], 1);
    assert!(mine["system"].is_null());
    assert_eq!(mine["package"]["owner_wallet"], member);
    let (_, all, _) = call(router.clone(), "GET", "/api/status", &admin, json!(null)).await;
    assert_eq!(all["stats"]["files"], 1);
    assert!(all["system"]["stats"]["files"].as_u64().unwrap() >= 2);
    assert_eq!(all["system"]["users"]["admins"], 1);
    assert_eq!(all["system"]["users"]["approved"], 1);
    assert_eq!(all["system"]["stats"]["web_online"], 1);
    assert!(!all.to_string().contains("member secret"));
    assert!(!all.to_string().contains("member-private.txt"));
    // The download must NOT live under /client/*, which accepts unauthenticated nodes.
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/download/client.zip",
            "",
            json!(null)
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // Status must include all clients, beyond the older Devices snapshot's cap.
    for i in 0..110 {
        s.for_owner(&member).db.web_connect(
            &format!("extra-{i}"),
            "Test browser",
            "test",
            "connected",
        )?;
    }
    let (_, expanded, _) = call(router.clone(), "GET", "/api/status", &cookie, json!(null)).await;
    assert_eq!(expanded["stats"]["web_online"], 111);
    assert_eq!(
        expanded["connections"]["web_clients"]
            .as_array()
            .unwrap()
            .len(),
        111
    );
    if mine["package"]["available"] == true {
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/download/client.zip")
                    .header("cookie", &cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/zip");
        let bytes = axum::body::to_bytes(response.into_body(), 100_000_000).await?;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        let mut script = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("go.sh")?, &mut script)?;
        assert!(script.contains(&format!("owner_wallet='{member}'")));
        assert!(script.contains("backend_url='http://100.64.0.20:3000'"));
        assert!(!script.contains("test-node-token"));
        assert!(!script.contains(ROOT));
    }
    Ok(())
}

#[tokio::test]
async fn repeated_account_erasure_uses_session_owner_and_rejects_old_epochs_after_restart()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = setup(dir.path());
    let router = app(state.clone(), "dist".into());
    let admin = login(&router, ROOT).await;
    let member = login(&router, MEMBER).await;
    let address = Identity::from_mnemonic(MEMBER).unwrap().address();
    let root = Identity::from_mnemonic(ROOT).unwrap().address();
    assert_eq!(
        call(
            router.clone(),
            "POST",
            "/api/auth/apply",
            &member,
            json!({"name":"Member"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &admin,
            json!({"role":"user"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let keep =
        state
            .for_owner(&root)
            .db
            .put_file("keep.txt", "text/plain", b"other wallet data")?;
    state
        .for_owner(&address)
        .db
        .put_file("erase.txt", "text/plain", b"private member data")?;
    assert_eq!(
        call(router.clone(), "DELETE", "/api/user/data", "", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(state.for_owner(&address).db.files()?.len(), 1);
    // A supplied owner is not authority: only the authenticated wallet can be erased.
    let (status, erased, _) = call(
        router.clone(),
        "DELETE",
        &format!("/api/user/data?owner_wallet={root}"),
        &member,
        json!({"owner_wallet":root}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first_epoch = erased["messagesyncid"].as_i64().unwrap();
    assert!(first_epoch > 0);
    assert!(state.for_owner(&address).db.files()?.is_empty());
    assert_eq!(
        state.for_owner(&root).db.file(&keep.id)?.1,
        b"other wallet data"
    );
    // Delete all preserves login and approval and excludes tombstones from Dashboard totals.
    let (status, dashboard, _) =
        call(router.clone(), "GET", "/api/status", &member, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    for field in [
        "rooms",
        "messages",
        "memos",
        "files",
        "images",
        "file_bytes",
        "screenshot_bytes",
        "running",
    ] {
        assert_eq!(dashboard["stats"][field], 0, "{field}");
    }
    assert_eq!(dashboard["package"]["owner_wallet"], address);
    assert!(dashboard["system"].is_null());
    let (status, erased, _) = call(
        router.clone(),
        "DELETE",
        "/api/user/data",
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let last_epoch = erased["messagesyncid"].as_i64().unwrap();
    assert!(
        last_epoch > first_epoch,
        "Even an empty erasure must invalidate older writers"
    );
    drop(router);
    drop(state);
    let state = setup(dir.path());
    let router = app(state.clone(), "dist".into());
    let member = login(&router, MEMBER).await;
    for epoch in [0, first_epoch] {
        assert_eq!(
            call(
                router.clone(),
                "POST",
                "/api/files",
                &member,
                json!({"name":"stale.txt","data_base64":"c3RhbGU=","data_epoch":epoch})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert!(state.for_owner(&address).db.files()?.is_empty());
    let (status, _, _) = call(
        router,
        "POST",
        "/api/files",
        &member,
        json!({"name":"fresh.txt","data_base64":"ZnJlc2g=","data_epoch":last_epoch}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.for_owner(&address).db.files()?.len(), 1);
    assert_eq!(
        state.for_owner(&root).db.file(&keep.id)?.1,
        b"other wallet data"
    );
    Ok(())
}

#[tokio::test]
async fn indexed_login_and_editable_profiles_preserve_approval_and_ownership() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let state = setup(dir.path());
    let router = app(state.clone(), "dist".into());
    let cookie = login_at(&router, MEMBER, 1, Some("  My wallet  ")).await;
    let address = Identity::from_mnemonic_at(MEMBER, 1).unwrap().address();
    let info = call(
        router.clone(),
        "GET",
        "/api/auth/info",
        &cookie,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(info["address"], address);
    assert_eq!(info["wallet_index"], 1);
    assert_eq!(info["name"], "My wallet");
    assert_eq!(info["role"], "pending");
    assert_eq!(info["approved"], false);
    for route in [
        "/api/settings",
        "/api/rooms",
        "/api/files",
        "/api/admin/users",
        "/api/notifications",
    ] {
        assert_eq!(
            call(router.clone(), "GET", route, &cookie, Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            "/api/auth/profile",
            "",
            json!({"name":"No session"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for name in [
        "".to_string(),
        " ".into(),
        "x".repeat(81),
        "line\nbreak".into(),
    ] {
        assert_eq!(
            call(
                router.clone(),
                "PATCH",
                "/api/auth/profile",
                &cookie,
                json!({"name":name})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            "/api/auth/profile",
            &cookie,
            json!({"name":"Changed", "role":"admin"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            "/api/auth/profile",
            &cookie,
            json!({"name":"Changed", "address":Identity::from_mnemonic(ROOT).unwrap().address()})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            "/api/auth/profile",
            &cookie,
            json!({"name":"내 지갑"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let root = login(&router, ROOT).await;
    let users = call(
        router.clone(),
        "GET",
        "/api/admin/users",
        &root,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(
        users["users"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["address"] == address)
            .unwrap()["name"],
        "내 지갑"
    );
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &root,
            json!({"role":"user"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let again = login_at(&router, MEMBER, 1, None).await;
    let info = call(router.clone(), "GET", "/api/auth/info", &again, Value::Null)
        .await
        .1;
    assert_eq!(info["name"], "내 지갑");
    assert_eq!(info["approved"], true);
    let other = login_at(&router, MEMBER, 2, None).await;
    let info = call(router.clone(), "GET", "/api/auth/info", &other, Value::Null)
        .await
        .1;
    assert_eq!(info["role"], "pending");
    assert_ne!(info["address"], address);
    assert_eq!(
        call(
            router.clone(),
            "PATCH",
            &format!("/api/admin/users/{address}"),
            &root,
            json!({"role":"blocked"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let blocked = login_at(&router, MEMBER, 1, Some("Renamed while blocked")).await;
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/auth/info",
            &blocked,
            Value::Null
        )
        .await
        .1["role"],
        "blocked"
    );
    assert_eq!(
        call(router.clone(), "GET", "/api/rooms", &blocked, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    causewaybay_server::users::restore(&state)?;
    assert_eq!(
        causewaybay_server::users::name(&state, &address)?,
        "Renamed while blocked"
    );
    assert_eq!(
        causewaybay_server::users::role(&state, &address)?,
        "blocked"
    );
    assert!(!std::fs::read_to_string(dir.path().join("users.jsonl"))?.contains(MEMBER));
    Ok(())
}

#[tokio::test]
async fn textedit_enrollment_is_separate_from_shared_capture_permission() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = setup(dir.path());
    let router = app(s.clone(), "dist".into());
    let root = login(&router, ROOT).await;
    let member = login(&router, MEMBER).await;
    let address = Identity::from_mnemonic(MEMBER).unwrap().address();
    call(
        router.clone(),
        "PATCH",
        &format!("/api/admin/users/{address}"),
        &root,
        json!({"role":"user"}),
    )
    .await;
    for id in ["shared", "owned", "legacy", "offline", "linux"] {
        s.db.client_hello(id, id, if id == "linux" { "linux" } else { "macos" })?;
        if id != "shared" {
            s.db.0.lock().unwrap().execute(
                "UPDATE capture_clients SET owner_wallet=?1 WHERE id=?2",
                [&address, id],
            )?;
        }
        if id != "legacy" {
            s.notes
                .capable
                .lock()
                .unwrap()
                .insert(id.into(), "connection".into());
        }
    }
    s.db.0.lock().unwrap().execute(
        "UPDATE capture_clients SET last_seen=0 WHERE id='offline'",
        [],
    )?;
    for path in ["/api/clients", "/api/dashboard"] {
        let (status, value, _) = call(router.clone(), "GET", path, &member, Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        let clients = if path == "/api/clients" {
            &value
        } else {
            &value["rust_clients"]
        };
        for (id, expected) in [
            ("shared", "needs_enrollment"),
            ("owned", "ready"),
            ("legacy", "update_required"),
            ("offline", "offline"),
            ("linux", "unavailable"),
        ] {
            let client = clients
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap();
            assert_eq!(client["can_capture"], true);
            assert_eq!(client["textedit"], expected, "{path}: {id}");
        }
    }
    for cookie in [&root, &member] {
        let (status, value, _) = call(
            router.clone(),
            "POST",
            "/api/invisible/sessions",
            cookie,
            json!({"client_id":"shared"}),
        )
        .await;
        // Generic chat/capture sessions may use shared nodes; TextEdit still requires enrollment.
        assert_eq!(status, StatusCode::OK);
        let id = value["id"].as_str().unwrap();
        let (status, value, _) = call(
            router.clone(),
            "POST",
            &format!("/api/invisible/sessions/{id}/start"),
            cookie,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(value["error"].as_str().unwrap().contains("enrolled"));
        assert_eq!(
            call(
                router.clone(),
                "POST",
                "/api/think",
                cookie,
                json!({"capture":true,"client_id":"shared"})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let created = call(
        router.clone(),
        "POST",
        "/api/invisible/sessions",
        &member,
        json!({"client_id":"owned"}),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(created.1["status"]["active"], false);
    let session_path = format!(
        "/api/invisible/sessions/{}/start",
        created.1["id"].as_str().unwrap()
    );
    assert_eq!(
        call(router.clone(), "POST", &session_path, &root, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let clients = call(router.clone(), "GET", "/api/clients", &root, Value::Null)
        .await
        .1;
    assert!(
        clients
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] == "shared")
    );
    Ok(())
}
