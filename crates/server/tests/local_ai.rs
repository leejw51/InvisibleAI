//! Real HTTP and subprocess boundaries, without paid inference or model downloads.
use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode},
    routing::post,
};
use causewaybay_server::{
    AppState, app,
    credentials::Credentials,
    local_ai, media, preferences,
    providers::{self, Config, Message},
};
use causewaybay_shared::{LocalAiSettings, SettingsUpdate};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

fn settings() -> LocalAiSettings {
    LocalAiSettings {
        apple_enabled: true,
        mlx_enabled: true,
        // These tests exercise the OpenAI-compatible service path; the built-in
        // Rust worker has its own tests in local_models.rs.
        mlx_engine: "service".into(),
        ..Default::default()
    }
}
fn configure(s: &AppState, provider: &str, local: LocalAiSettings) -> anyhow::Result<()> {
    preferences::update(
        s,
        SettingsUpdate {
            provider: Some(provider.into()),
            local_ai: Some(local),
            ..Default::default()
        },
    )
}
async fn call(s: &Arc<AppState>, method: &str, path: &str, value: Value) -> (StatusCode, Value) {
    let response = app(s.clone(), "dist".into())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}
async fn finished(s: &AppState, id: &str) -> causewaybay_shared::Memo {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let memo = s.db.get(id).unwrap().unwrap();
            if memo.status != "running" {
                return memo;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("job must finish")
}
#[cfg(unix)]
fn script(path: &Path, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/usr/bin/env python3\n{source}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn local_urls_cannot_escape_loopback() {
    for url in ["http://127.0.0.1:8080/v1", "http://[::1]:8080/v1/"] {
        assert!(
            local_ai::mlx_endpoint(url, "chat/completions").is_ok(),
            "{url}"
        );
    }
    for url in [
        "https://api.openai.com/v1",
        "http://localhost:8080/v1",
        "http://192.168.1.2/v1",
        "http://127.0.0.1.attacker.test/v1",
        "http://127.0.0.1@attacker.test/v1",
        "http://user:secret@127.0.0.1/v1",
        "http://127.0.0.1/v1?host=remote",
        "http://127.0.0.1/v1#x",
        "file:///tmp/model",
        "http://127.0.0.1/other",
    ] {
        assert!(
            local_ai::mlx_endpoint(url, "chat/completions").is_err(),
            "{url}"
        );
    }
}

#[test]
fn models_allow_mlx_ids_but_not_paths_or_commands_for_cloud() {
    assert!(local_ai::valid_model(
        "mlx",
        "mlx-community/Qwen3.5-9B-4bit"
    ));
    assert!(local_ai::valid_model("mlx", "/models/qwen"));
    assert!(local_ai::valid_model("apple", "system"));
    for (provider, model) in [
        ("apple", "qwen"),
        ("mlx", "../secrets"),
        ("mlx", "a;echo hi"),
        ("mlx", ""),
        ("openai", "/models/a"),
        ("grok", "abc\nsecret"),
    ] {
        assert!(!local_ai::valid_model(provider, model));
    }
}

#[tokio::test]
async fn settings_persist_and_reject_unsafe_updates_atomically() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(
        dir.path().into(),
        false,
        None,
        Credentials::from_values([("XAI_API_KEY".into(), "secret".into())]),
    )?;
    let mut local = settings();
    local.local_only = true;
    configure(&s, "mlx", local.clone())?;
    let (status, body) = call(&s, "GET", "/api/settings", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.to_string().contains("secret"));
    assert!(
        body["providers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| !local_ai::is_local(p["id"].as_str().unwrap()))
            .all(|p| p["configured"] == false)
    );
    let before = std::fs::read(dir.path().join("settings.jsonl"))?;
    for bad in [
        json!({"mlx_url":"https://remote.test/v1"}),
        json!({"speech_provider":"grok","local_only":true}),
        json!({"speech_locale":"en; exit"}),
        json!({"mlx_speech_model":"relative/path"}),
        json!({"executable":"/bin/sh"}),
    ] {
        let mut proposed = serde_json::to_value(&local)?;
        proposed
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let (status, _) = call(&s, "PATCH", "/api/settings", json!({"local_ai":proposed})).await;
        assert!(status.is_client_error());
        assert_eq!(before, std::fs::read(dir.path().join("settings.jsonl"))?);
    }
    drop(s);
    let restored =
        AppState::open_with_credentials(dir.path().into(), false, None, Credentials::default())?;
    assert_eq!(restored.settings()?.local_ai, local);
    assert_eq!(restored.provider_config()?.provider, "mlx");
    Ok(())
}

#[tokio::test]
async fn disabled_local_provider_never_falls_back_to_a_configured_cloud() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(
        dir.path().into(),
        false,
        None,
        Credentials::from_values([("XAI_API_KEY".into(), "unused-cloud-key".into())]),
    )?;
    configure(&s, "mlx", LocalAiSettings::default())?;
    assert_eq!(s.provider_config()?.provider, "mlx");
    let (status, _) = call(&s, "POST", "/api/think", json!({"prompt":"Keep private"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(s.db.count()?, 0);
    Ok(())
}

#[tokio::test]
async fn local_only_rejects_cloud_even_in_demo_and_before_job_creation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Credentials::default())?;
    let mut local = settings();
    local.local_only = true;
    configure(&s, "apple", local)?;
    let count = s.db.count()?;
    for provider in ["openai", "anthropic", "grok", "gemini"] {
        let (status, body) = call(
            &s,
            "POST",
            "/api/think",
            json!({"provider":provider,"prompt":"private"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("Local-only"));
    }
    assert_eq!(s.db.count()?, count);
    let config = s.provider_config()?;
    assert!(causewaybay_server::productivity::search_request(&config, "private").is_err());
    Ok(())
}

#[tokio::test]
async fn audio_routing_uses_each_jobs_provider_not_the_server_default() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(
        dir.path().into(),
        false,
        None,
        Credentials::from_values([("XAI_API_KEY".into(), "unused-cloud-key".into())]),
    )?;
    assert_eq!(media::audio_provider(&s).as_deref(), Some("grok"));
    for provider in ["apple", "mlx"] {
        let mut config = Config {
            provider: provider.into(),
            local: settings(),
            ..Default::default()
        };
        assert_eq!(media::speech_provider(&s, &config)?, provider);
        config.local.speech_provider = "gemini".into();
        assert!(media::speech_provider(&s, &config).is_err());
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn apple_worker_streams_unicode_and_receives_data_on_stdin() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("worker");
    script(
        &path,
        r#"import sys,json,os
r=json.load(sys.stdin)
assert len(sys.argv)==1
assert os.environ['HF_HUB_OFFLINE']=='1'
assert 'private prompt' in r['prompt']
for text in ['Hello ', '안녕 👋']:
 print(json.dumps({'delta':text}),flush=True)
print(json.dumps({'done':True}),flush=True)"#,
    );
    let config = Config {
        provider: "apple".into(),
        model: "system".into(),
        local: settings(),
        runtime: local_ai::Runtime {
            apple_worker: path,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut deltas = vec![];
    let turn = providers::stream(
        &config,
        "system",
        &[Message::User("private prompt".into())],
        &[],
        |v| {
            deltas.push(v.to_string());
            Ok(())
        },
    )
    .await?;
    assert_eq!(deltas, ["Hello ", "안녕 👋"]);
    assert_eq!(turn.text, "Hello 안녕 👋");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn worker_errors_are_bounded_and_do_not_leak_diagnostics() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    for (index, code) in [
        "print('not json')",
        "print('{\"delta\":\"partial\"}')",
        "print('{\"done\":true}')",
        "print('{\"error\":\"private key from runtime\"}')",
        "print('x'*2000001)",
        "print('{\"delta\":\"ok\"}'); print('{\"done\":true}'); print('{\"delta\":\"late\"}')",
    ]
    .iter()
    .enumerate()
    {
        let path = dir.path().join(format!("worker{index}"));
        script(&path, &format!("import sys\nsys.stdin.read()\n{code}"));
        let err = local_ai::worker(&path, &[], json!({}), Duration::from_secs(5), |_| Ok(()))
            .await
            .unwrap_err();
        assert!(!err.to_string().contains("private key"));
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_and_cancellation_kill_the_worker() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("worker");
    script(
        &path,
        "import sys,json,os,time\nr=json.load(sys.stdin)\nopen(r['pid_file'],'w').write(str(os.getpid()))\ntime.sleep(60)",
    );
    for abort in [false, true] {
        let pid_file = dir.path().join(format!("pid{abort}"));
        let executable = path.clone();
        let request = json!({"pid_file":pid_file});
        let task = tokio::spawn(async move {
            local_ai::worker(
                &executable,
                &[],
                request,
                // Long enough that a slow Python start under a parallel build
                // still writes the pid file before the timeout fires.
                Duration::from_millis(if abort { 10000 } else { 2500 }),
                |_| Ok(()),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(15), async {
            while !pid_file.is_file() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        if abort {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            assert!(task.await?.is_err());
        }
        let pid = std::fs::read_to_string(pid_file)?;
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if !std::process::Command::new("/bin/kill")
                    .args(["-0", pid.trim()])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn mlx_http_stream_and_audio_attachment_finish_without_cloud() -> anyhow::Result<()> {
    let requests = Arc::new(Mutex::new(vec![]));
    let seen = requests.clone();
    let router = Router::new().route("/v1/chat/completions", post(move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
        let seen = seen.clone(); async move {
            assert!(!headers.contains_key("authorization"));
            seen.lock().unwrap().push(body);
            ([("content-type","text/event-stream")], "data: {\"choices\":[{\"delta\":{\"content\":\"A private answer.\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/v1", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = tempfile::tempdir()?;
    let mut s =
        AppState::open_with_credentials(dir.path().into(), false, None, Credentials::default())?;
    let path = dir.path().join("worker");
    #[cfg(unix)]
    script(
        &path,
        "import sys,json,pathlib\nr=json.load(sys.stdin)\nassert pathlib.Path(r['audio_path']).read_bytes()==b'RIFF audio fixture'\nprint(json.dumps({'delta':'Tell me about Bitcoin.'}));print(json.dumps({'done':True}))",
    );
    Arc::get_mut(&mut s).unwrap().local_runtime.apple_worker = path;
    let local = LocalAiSettings {
        mlx_url: url,
        speech_provider: "apple".into(),
        local_only: true,
        ..settings()
    };
    configure(&s, "mlx", local)?;
    let file =
        s.db.put_file("speech.wav", "audio/wav", b"RIFF audio fixture")?;
    let (status, value) = call(
        &s,
        "POST",
        "/api/think",
        json!({"prompt":"Please answer","file_id":file.id,"input_mode":"image"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let memo = finished(&s, value["id"].as_str().unwrap()).await;
    assert_eq!(memo.status, "complete", "{:?}", memo.error);
    assert_eq!(memo.provider, "mlx");
    assert_eq!(memo.answer, "A private answer.");
    assert!(memo.input.contains("Tell me about Bitcoin."));
    // The Wiki's on-device tagger may call the same local model for this
    // message; only the chat completion itself is judged here.
    let sent = requests.lock().unwrap();
    let chat: Vec<&Value> = sent
        .iter()
        .filter(|r| {
            r["messages"][0]["content"]
                .as_str()
                .is_some_and(|c| c.starts_with("You are Invisible AI"))
        })
        .collect();
    assert_eq!(chat.len(), 1, "{sent:?}");
    assert!(chat[0].get("tools").is_none());
    assert!(chat[0].get("max_tokens").is_some());
    assert!(chat[0].to_string().contains("Tell me about Bitcoin."));
    server.abort();
    Ok(())
}

#[tokio::test]
async fn mlx_refuses_redirects_and_unfinished_streams() -> anyhow::Result<()> {
    for redirect in [true, false] {
        let router = Router::new().route(
            "/v1/chat/completions",
            post(move || async move {
                if redirect {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [("location", "https://example.com/private")],
                        "",
                    )
                } else {
                    (
                        StatusCode::OK,
                        [("content-type", "text/event-stream")],
                        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let local = LocalAiSettings {
            mlx_url: format!("http://{}/v1", listener.local_addr()?),
            ..settings()
        };
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let config = Config {
            provider: "mlx".into(),
            model: "local/model".into(),
            local,
            ..Default::default()
        };
        let error = providers::stream(
            &config,
            "system",
            &[Message::User("private".into())],
            &[],
            |_| Ok(()),
        )
        .await
        .unwrap_err();
        if redirect {
            assert!(error.to_string().contains("307"));
        }
        server.abort();
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "Requires make apple-ai, Apple Intelligence, and an already-installed en-US speech model"]
async fn live_apple_transcribes_audio_and_answers_through_rust_http() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let audio = dir.path().join("speech.wav");
    let status = std::process::Command::new("/usr/bin/say")
        .args([
            "-v",
            "Samantha",
            "-r",
            "140",
            "--file-format=WAVE",
            "--data-format=LEI16@16000",
            "-o",
        ])
        .arg(&audio)
        .arg("Welcome to Invisible AI. This is a private speech test.")
        .status()?;
    anyhow::ensure!(
        status.success(),
        "Could not create the synthetic speech fixture"
    );
    let s = AppState::open_with_credentials(
        dir.path().join("data"),
        false,
        None,
        Credentials::default(),
    )?;
    configure(
        &s,
        "apple",
        LocalAiSettings {
            local_only: true,
            speech_provider: "apple".into(),
            ..settings()
        },
    )?;
    let bytes = std::fs::read(&audio)?;
    let (transcript, processor) = media::transcribe(&s, "speech.wav", "audio/wav", bytes).await?;
    assert_eq!(processor, "apple local");
    assert!(
        transcript.to_lowercase().contains("private speech test"),
        "{transcript}"
    );
    let (status, value) = call(
        &s,
        "POST",
        "/api/think",
        json!({"prompt":format!("Repeat the following sentence exactly: {transcript}")}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let memo = finished(&s, value["id"].as_str().unwrap()).await;
    assert_eq!(memo.status, "complete", "{:?}", memo.error);
    assert_eq!(memo.provider, "apple");
    assert!(
        memo.answer.to_lowercase().contains("private speech test"),
        "{}",
        memo.answer
    );
    Ok(())
}

#[tokio::test]
#[ignore = "Requires an already-installed MLX server on loopback:18081; no model is downloaded"]
async fn live_mlx_answers_through_rust_http_and_records_usage() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s =
        AppState::open_with_credentials(dir.path().into(), false, None, Credentials::default())?;
    configure(
        &s,
        "mlx",
        LocalAiSettings {
            local_only: true,
            mlx_enabled: true,
            mlx_engine: "service".into(),
            mlx_url: "http://127.0.0.1:18081/v1".into(),
            ..Default::default()
        },
    )?;
    let (status,memo)=call(&s,"POST","/api/think",json!({"provider":"mlx","prompt":"What is 2 + 3? Answer in one short sentence.","input_mode":"text"})).await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    let id = memo["id"].as_str().unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let m = s.db.get(id).unwrap().unwrap();
            if m.status != "running" {
                break m;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?;
    assert_eq!(answer.status, "complete", "{:?}", answer.error);
    assert!(answer.answer.contains('5') || answer.answer.to_lowercase().contains("five"));
    assert_eq!(answer.provider, "mlx");
    let (status, usage) = call(&s, "GET", "/api/usage", json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    let mlx = usage["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == "mlx")
        .unwrap();
    assert!(mlx["elapsed_ms"].as_u64().unwrap() > 0);
    assert!(mlx["output_tokens"].as_u64().unwrap_or(0) > 0, "{mlx}");
    Ok(())
}
