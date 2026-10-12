use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(router: axum::Router, method: &str, url: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(url)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn credentials_are_backend_owned_and_never_returned_or_written() -> anyhow::Result<()> {
    use causewaybay_server::credentials::Credentials;
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(
        dir.path().into(),
        false,
        None,
        Credentials::from_values([("XAI_API_KEY".into(), "backend-only-test-key".into())]),
    )?;
    // Legacy server/browser choices must not mask the available backend provider.
    s.db.set_setting("provider", "anthropic")?;
    s.db.set_setting("key.openai", "legacy-encrypted-value")?;
    assert_eq!(s.provider_config()?.provider, "grok");
    assert_eq!(s.provider_config()?.key, "backend-only-test-key");
    let router = app(s.clone(), "dist".into());
    let (status, body) = call(router.clone(), "GET", "/api/settings", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["provider"], "grok");
    let providers = body["providers"].as_array().unwrap();
    // Codex and Claude Code report whether their CLI is installed on this machine,
    // which is not a credential; only key-backed providers are counted here.
    assert_eq!(
        providers
            .iter()
            .filter(|p| p["configured"] == true
                && !matches!(p["id"].as_str(), Some("codex" | "claude_code")))
            .count(),
        1
    );
    assert!(
        providers
            .iter()
            .any(|p| p["id"] == "grok" && p["configured"] == true)
    );
    assert!(!body.to_string().contains("backend-only-test-key"));
    for (method, url, request) in [
        (
            "PATCH",
            "/api/settings",
            json!({"provider":"openai", "api_key":"browser-key"}),
        ),
        ("PATCH", "/api/settings", json!({"remove_key":true})),
        (
            "POST",
            "/api/think",
            json!({"prompt":"hello", "api_key":"browser-key"}),
        ),
    ] {
        let (status, _) = call(router.clone(), method, url, request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert!(s.db.get_setting("key.grok")?.is_none());
    assert_eq!(s.provider_config()?.key, "backend-only-test-key");
    let (status, body) = call(
        router.clone(),
        "PATCH",
        "/api/settings",
        json!({"provider":"grok", "model":"grok-4", "input_mode":"image"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["input_mode"], "image");
    let (status, _) = call(
        router,
        "PATCH",
        "/api/settings",
        json!({"input_mode":"invalid"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn missing_credentials_direct_users_to_backend_configuration() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
    let (status, body) = call(
        app(s.clone(), "dist".into()),
        "POST",
        "/api/think",
        json!({"provider":"grok", "prompt":"hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("backend environment or .env")
    );
    assert_eq!(s.db.count()?, 0);
    Ok(())
}

#[tokio::test]
async fn backend_settings_are_jsonl_persistent_and_validate_before_writing() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(s.clone(), "dist".into());
    let (status, _) = call(
        router.clone(),
        "PATCH",
        "/api/settings",
        json!({"provider":"grok","model":"grok-4","input_mode":"image","display":2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let before = std::fs::read_to_string(dir.path().join("settings.jsonl"))?;
    let record: Value = serde_json::from_str(before.lines().last().unwrap())?;
    assert_eq!(record["config"]["display"], 2);
    assert_eq!(record["config"]["provider"], "grok");
    let (status, _) = call(
        router,
        "PATCH",
        "/api/settings",
        json!({"input_mode":"text","display":99}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(s.settings()?.input_mode, "image");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("settings.jsonl"))?,
        before
    );
    s.db.set_setting("input_mode", "text")?;
    drop(s);
    let restored =
        AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    assert_eq!(restored.settings()?.input_mode, "image");
    assert_eq!(restored.settings()?.display, 2);
    Ok(())
}

#[tokio::test]
async fn admin_ai_default_persists_without_overwriting_personal_modes() -> anyhow::Result<()> {
    use causewaybay_server::ai_preferences;
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(s.clone(), "dist".into());
    let default = json!({"mode":"hybrid","local_provider":"apple","cloud_provider":"grok"});
    let (status, body) = call(
        router.clone(),
        "PATCH",
        "/api/settings",
        json!({"provider":"apple","default_ai":default}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["default_ai"], default);
    let alice = s.for_owner("alice");
    let bob = s.for_owner("bob");
    assert_eq!(ai_preferences::read(&alice)?.unwrap().mode, "hybrid");
    assert_eq!(alice.provider_config()?.provider, "apple");
    assert!(ai_preferences::allow(&alice, "grok").is_err());
    bob.db.set_setting(
        "user.ai.bob",
        &json!({"mode":"ondevice","local_provider":"mlx","cloud_provider":"grok"}).to_string(),
    )?;
    let (status, _) = call(
        router.clone(),
        "PATCH",
        "/api/settings",
        json!({"default_ai":{"mode":"cloudai","local_provider":"apple","cloud_provider":"grok"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ai_preferences::read(&alice)?.unwrap().mode, "cloudai");
    assert_eq!(ai_preferences::read(&bob)?.unwrap().mode, "ondevice");
    assert!(ai_preferences::allow(&bob, "grok").is_err());
    let before = std::fs::read(dir.path().join("settings.jsonl"))?;
    for bad in [
        json!({"default_ai":{"mode":"automatic","local_provider":"apple","cloud_provider":"grok"}}),
        json!({"default_ai":{"mode":"hybrid","local_provider":"grok","cloud_provider":"apple"}}),
        json!({"data_dir":"/outside"}),
        json!({"config_dir":"/outside"}),
    ] {
        let (status, _) = call(router.clone(), "PATCH", "/api/settings", bad).await;
        assert!(status.is_client_error());
        assert_eq!(std::fs::read(dir.path().join("settings.jsonl"))?, before);
    }
    drop(router);
    drop(alice);
    drop(bob);
    drop(s);
    let restored =
        AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    assert_eq!(restored.settings()?.default_ai.unwrap().mode, "cloudai");
    assert_eq!(
        ai_preferences::read(&restored.for_owner("bob"))?
            .unwrap()
            .mode,
        "ondevice"
    );
    assert!(!restored.settings()?.local_ai.local_only);
    Ok(())
}

#[tokio::test]
async fn token_protects_api_and_cross_origin_requests_are_denied() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open(
        dir.path().into(),
        false,
        Some("private-access-token".into()),
    )?;
    let router = app(s, "dist".into());
    let (status, _) = call(router.clone(), "GET", "/api/settings", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/settings")
                .header("authorization", "Bearer private-access-token")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/settings")
                .header("authorization", "Bearer private-access-token")
                .header("host", "localhost:3000")
                .header("origin", "https://evil.example")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/session")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"token":"private-access-token"}).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"].to_str()?;
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    Ok(())
}

#[tokio::test]
async fn memo_search_filters_tags_and_keeps_durable_sse_cursor() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open(dir.path().into(), false, None)?;
    let router = app(s.clone(), "dist".into());
    let (_, memo) = call(
        router.clone(),
        "POST",
        "/api/memos",
        json!({"title":"Rust guide","content":"Borrowing is useful #rust #learning"}),
    )
    .await;
    let id = memo["id"].as_str().unwrap();
    let (_, result) = call(
        router.clone(),
        "GET",
        "/api/memos?q=borrowing%20%23rust",
        json!({}),
    )
    .await;
    assert_eq!(result["total"], 1);
    let (_, result) = call(router.clone(), "GET", "/api/memos?q=%23python", json!({})).await;
    assert_eq!(result["total"], 0);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/memos/{id}/events"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let text = String::from_utf8(response.into_body().collect().await?.to_bytes().to_vec())?;
    assert!(text.contains("\"kind\":\"delta\"") && text.contains("\"kind\":\"done\""));
    let cursor = s.db.events(id, 0)?[0].id;
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/api/memos/{id}/events"))
                .header("last-event-id", cursor.to_string())
                .body(Body::empty())?,
        )
        .await?;
    let text = String::from_utf8(response.into_body().collect().await?.to_bytes().to_vec())?;
    assert!(!text.contains("\"kind\":\"delta\""));
    assert!(text.contains("\"kind\":\"done\""));
    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn demo_agent_pipeline_runs_real_tools_and_saves_result() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open(dir.path().into(), true, None)?;
    let router = app(s.clone(), "dist".into());
    let (status, m) = call(
        router.clone(),
        "POST",
        "/api/think",
        json!({"prompt":"write Rust addition","agent":true,"input_mode":"text"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = m["id"].as_str().unwrap();
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/api/memos/{id}/events"))
                .body(Body::empty())?,
        )
        .await?;
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        response.into_body().collect(),
    )
    .await??;
    let text = String::from_utf8(bytes.to_bytes().to_vec())?;
    assert!(
        text.contains("write_file")
            && text.contains("compile_code")
            && text.contains("Compilation succeeded")
    );
    assert_eq!(s.db.get(id)?.unwrap().status, "complete");
    assert!(s.db.files()?.iter().any(|f| f.name == "answer.rs"));
    Ok(())
}

#[tokio::test]
async fn cancel_keeps_partial_answer_and_restart_marks_interrupted_work() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    {
        let s = AppState::open(dir.path().into(), false, None)?;
        let m = s.db.create("unfinished", "question", "local", "")?;
        s.db.event(&m.id, "delta", "partial answer")?;
    }
    let s = AppState::open(dir.path().into(), false, None)?;
    let m = s.db.all()?.remove(0);
    assert_eq!(m.status, "error");
    assert_eq!(m.answer, "partial answer");
    assert!(m.error.unwrap().contains("restarted"));
    let router = app(s.clone(), "dist".into());
    let (status, _) = call(router, "POST", "/api/think", json!({"prompt":""})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn image_payloads_and_text_mode_are_distinct() -> anyhow::Result<()> {
    use causewaybay_server::providers::{Config, Message, request_body};
    for provider in ["anthropic", "openai", "grok", "gemini"] {
        let config = Config {
            provider: provider.into(),
            model: "test".into(),
            key: "never-in-body".into(),
            ..Default::default()
        };
        let image = request_body(
            &config,
            "system",
            &[Message::Image {
                text: "read this".into(),
                images: vec![
                    ("image/png".into(), "BASE64_IMAGE_BYTES".into()),
                    ("image/jpeg".into(), "SECOND_PHOTO".into()),
                ],
            }],
            &[],
        );
        assert!(image.to_string().contains("BASE64_IMAGE_BYTES"));
        assert!(image.to_string().contains("SECOND_PHOTO"));
        let parts = if provider == "gemini" {
            &image["contents"][0]["parts"]
        } else if provider == "anthropic" {
            &image["messages"][0]["content"]
        } else {
            &image["messages"][1]["content"]
        };
        assert_eq!(parts.as_array().unwrap().len(), 3);
        assert!(parts[1].to_string().contains("SECOND_PHOTO"));
        assert_eq!(parts[2]["text"], "read this");
        assert!(!image.to_string().contains("never-in-body"));
        let text = request_body(&config, "system", &[Message::User("OCR text".into())], &[]);
        assert!(text.to_string().contains("OCR text"));
        assert!(!text.to_string().contains("BASE64_IMAGE_BYTES"));
        match provider {
            "anthropic" => assert_eq!(image["messages"][0]["content"][0]["type"], "image"),
            "gemini" => assert_eq!(
                image["contents"][0]["parts"][0]["inlineData"]["mimeType"],
                "image/png"
            ),
            _ => assert_eq!(image["messages"][1]["content"][0]["type"], "image_url"),
        }
    }
    Ok(())
}

#[tokio::test]
async fn file_versions_and_exact_edits_preserve_original() -> anyhow::Result<()> {
    use causewaybay_server::{agent, providers::ToolCall};
    let dir = tempfile::tempdir()?;
    let s = AppState::open(dir.path().into(), false, None)?;
    let m = s.db.create("edit", "agent", "local", "")?;
    let old =
        s.db.put_file("main.rs", "text/plain", b"pub fn answer()->i32{41}")?;
    let call = ToolCall {
        id: "1".into(),
        name: "edit_file".into(),
        arguments: json!({"file_id":old.id,"find":"41","replace":"42"}).to_string(),
    };
    let output: Value =
        serde_json::from_str(&agent::execute(&s, &m.id, &call, &agent::definitions()).await?)?;
    assert_ne!(output["file"]["id"], old.id);
    assert!(String::from_utf8(s.db.file(&old.id)?.1)?.contains("41"));
    assert_eq!(s.db.files()?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn cancelling_a_live_job_stops_generation_and_preserves_replay() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open(dir.path().into(), true, None)?;
    let router = app(s.clone(), "dist".into());
    let (_, m) = call(
        router.clone(),
        "POST",
        "/api/think",
        json!({"capture":true}),
    )
    .await;
    let id = m["id"].as_str().unwrap();
    let (status, _) = call(
        router,
        "POST",
        &format!("/api/memos/{id}/cancel"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    let memo = s.db.get(id)?.unwrap();
    assert_eq!(memo.status, "error");
    assert!(memo.error.unwrap().contains("Stopped by you"));
    let events = s.db.events(id, 0)?;
    assert_eq!(events.last().unwrap().kind, "error");
    assert!(!events.iter().any(|e| e.kind == "done"));
    Ok(())
}

#[tokio::test]
async fn node_names_survive_restart_and_invalid_updates_are_not_saved() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open(dir.path().into(), true, None)?;
    s.db.client_hello("laptop", "Automatic name", "macOS")?;
    let router = app(s.clone(), "dist".into());
    let (status, body) = call(
        router.clone(),
        "PATCH",
        "/api/clients/laptop",
        json!({"name":"Travel Mac"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Travel Mac");
    let saved = std::fs::read_to_string(dir.path().join("nodes.jsonl"))?;
    for name in ["", "\n", "a\u{0000}b"] {
        assert_eq!(
            call(
                router.clone(),
                "PATCH",
                "/api/clients/laptop",
                json!({"name":name})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("nodes.jsonl"))?,
        saved
    );
    // Restore aliases from JSONL even if the query mirror has been lost.
    s.db.0
        .lock()
        .unwrap()
        .execute("DELETE FROM settings WHERE key LIKE 'node.%'", [])?;
    drop(router);
    drop(s);
    let restored = AppState::open(dir.path().into(), true, None)?;
    restored
        .db
        .client_hello("laptop", "Old name from offline device", "macOS")?;
    assert_eq!(restored.db.clients()?[0].name, "Travel Mac");
    Ok(())
}

#[tokio::test]
async fn capture_batch_snapshots_online_selection_and_callbacks_require_session()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open(dir.path().into(), true, None)?;
    s.db.client_hello("a", "A", "macOS")?;
    s.db.client_hello("b", "B", "macOS")?;
    s.db.client_hello("offline", "Offline", "macOS")?;
    s.db.release_captures("offline")?;
    let router = app(s.clone(), "dist".into());
    assert_eq!(
        call(
            router.clone(),
            "GET",
            "/api/client/events?client_id=a",
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let request = json!({"provider":"anthropic","input_mode":"text","prompt":"batch test"});
    for ids in [json!([]), json!(["offline"]), json!(["missing"])] {
        assert!(
            call(
                router.clone(),
                "POST",
                "/api/capture/batch",
                json!({"client_ids":ids,"request":request})
            )
            .await
            .0
            .is_client_error()
        );
    }
    let (status, body) = call(
        router.clone(),
        "POST",
        "/api/capture/batch",
        json!({"all":true,"request":request}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i["client"]["online"] == true
        && i["memo"]["status"] == "running"
        && i["error"].is_null()));
    assert_ne!(items[0]["memo"]["id"], items[1]["memo"]["id"]);
    for item in items {
        call(
            router.clone(),
            "POST",
            &format!("/api/memos/{}/cancel", item["memo"]["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
    }
    Ok(())
}

#[tokio::test]
async fn workspace_pages_return_success_on_direct_navigation_and_reload() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let dist = dir.path().join("frontend");
    std::fs::create_dir(&dist)?;
    std::fs::write(
        dist.join("index.html"),
        "<html>Invisible AI workspace</html>",
    )?;
    let state =
        AppState::open_with_credentials(dir.path().join("data"), true, None, Default::default())?;
    let router = app(state, dist);
    for path in [
        "/cloud",
        "/wiki",
        "/wiki/0123456789abcdef",
        "/wiki/map",
        "/wiki/tag/bitcoin",
        "/assistance",
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert!(
            response.headers()["content-type"]
                .to_str()?
                .starts_with("text/html")
        );
        assert_eq!(
            response.into_body().collect().await?.to_bytes(),
            "<html>Invisible AI workspace</html>"
        );
    }
    let (status, _) = call(router, "GET", "/not-a-page", Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}
