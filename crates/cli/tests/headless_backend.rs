//! No browser, web assets, live desktop capture, or paid AI is needed for this acceptance test.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use causewaybay_server::{AppState, app, auth::Auth, credentials::Credentials};
use causewaybay_shared::{CreateInvisibleSession, InvisibleSession};
use serde_json::{Value, json};
use std::{process::Stdio, sync::Arc, time::Duration};

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn rust_cli_delivers_capture_to_rust_backend_without_frontend() -> Result<()> {
    let root = tempfile::tempdir()?;
    let data = root.path().join("backend");
    let mut state =
        AppState::open_with_credentials(data.clone(), true, None, Credentials::default())?;
    Arc::get_mut(&mut state)
        .context("Fresh state must be unique")?
        .auth = Arc::new(Auth::new(None)?);
    // Exercise production-style pairing while keeping inference deterministic.
    Arc::get_mut(&mut state).unwrap().capture_token =
        Some(causewaybay_shared::config::secret(&data, "capture-token")?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    // Deliberately do not build or serve frontend assets.
    let router = app(state.clone(), root.path().join("no-web-assets"));
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let image = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=")?;
    let fixture = root.path().join("fixture.png");
    tokio::fs::write(&fixture, &image).await?;
    let client_dir = root.path().join("capture-client");
    let mut cli = tokio::process::Command::new(env!("CARGO_BIN_EXE_rust-cli"))
        .args([
            "client",
            "--backend",
            &base,
            "--client-id",
            "headless-mac",
            "--name",
            "Headless Mac",
            "--once",
            "--shared",
            "--fixture-image",
        ])
        .arg(&fixture)
        .arg("--config-dir")
        .arg(&client_dir)
        .env("APP_TOKEN", "")
        .env("CAUSEWAYBAY_CONFIG_DIR", &data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !state
            .db
            .clients()?
            .iter()
            .any(|c| c.id == "headless-mac" && c.online)
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let session: InvisibleSession = http
        .post(format!("{base}/api/invisible/sessions"))
        .json(&CreateInvisibleSession {
            client_id: "headless-mac".into(),
            title: Some("Headless capture".into()),
            room_id: None,
            start: false,
            app: None,
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        !session.status.active,
        "Creating a session must not start note writing"
    );
    let snapshot: Value = http
        .post(format!("{base}/api/rooms/{}/snapshot", session.id))
        .json(&json!({"client_id":"headless-mac"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let file = snapshot["id"]
        .as_str()
        .context("Backend snapshot must return a file")?;
    let bytes = http
        .get(format!("{base}/api/files/{file}/download"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    ensure!(
        bytes.as_ref() == image,
        "Actual CLI upload must reach backend storage unchanged"
    );
    ensure!(
        tokio::time::timeout(Duration::from_secs(10), cli.wait())
            .await??
            .success(),
        "Capture CLI failed"
    );

    // Process the captured source through the backend without any frontend connection.
    let memo: Value = http.post(format!("{base}/api/think"))
        .json(&json!({"room_id":session.id,"message_id":uuid::Uuid::new_v4().to_string(),"file_ids":[file],"input_mode":"image","prompt":"Describe this fixture"}))
        .send().await?.error_for_status()?.json().await?;
    let memo_id = memo["id"].as_str().context("No memo ID")?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let memo = state.db.get(memo_id)?.context("Memo disappeared")?;
            ensure!(
                memo.status != "error",
                "Backend job failed: {:?}",
                memo.error
            );
            if memo.status == "complete" {
                ensure!(!memo.answer.is_empty(), "Backend answer is empty");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    ensure!(
        state.db.web_clients()?.is_empty(),
        "This workflow must not create a browser connection"
    );
    let restarted = AppState::open_with_credentials(data, true, None, Credentials::default())?;
    let restored =
        causewaybay_server::invisible_sessions::read(&restarted.for_owner("local"), &session.id)
            .map_err(|e| anyhow::anyhow!(e.1))?;
    ensure!(
        restored.title == "Headless capture" && !restored.status.active,
        "Session did not restore"
    );
    ensure!(
        restarted.db.get(memo_id)?.is_some(),
        "Saved result did not survive reopen"
    );
    drop(server);
    Ok(())
}

/// Audio Capture without a microphone: the CLI replays a WAV fixture as if it
/// were spoken, the demo backend transcribes it with its fixture text, Solve
/// posts the answer into the Audio Capture room, and Stop keeps the notes.
#[tokio::test]
async fn rust_cli_streams_audio_and_backend_solves_it_into_chat() -> Result<()> {
    let root = tempfile::tempdir()?;
    let data = root.path().join("backend");
    let mut state =
        AppState::open_with_credentials(data.clone(), true, None, Credentials::default())?;
    Arc::get_mut(&mut state)
        .context("Fresh state must be unique")?
        .auth = Arc::new(Auth::new(None)?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let router = app(state.clone(), root.path().join("no-web-assets"));
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    // Two seconds of a tone, a pause, and two more: two utterances at 16 kHz.
    let fixture = root.path().join("speech.wav");
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&fixture, spec)?;
        let tone = |i: usize| {
            (0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin()
                * i16::MAX as f32) as i16
        };
        for i in 0..32_000 {
            writer.write_sample(tone(i))?;
        }
        for _ in 0..32_000 {
            writer.write_sample(0i16)?;
        }
        for i in 0..32_000 {
            writer.write_sample(tone(i))?;
        }
        writer.finalize()?;
    }
    let client_dir = root.path().join("capture-client");
    let _cli = tokio::process::Command::new(env!("CARGO_BIN_EXE_rust-cli"))
        .args([
            "client",
            "--backend",
            &base,
            "--client-id",
            "listening-mac",
            "--name",
            "Listening Mac",
            "--shared",
            "--fixture-audio",
        ])
        .arg(&fixture)
        .arg("--config-dir")
        .arg(&client_dir)
        .env("APP_TOKEN", "")
        .env("CAUSEWAYBAY_CONFIG_DIR", &data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let clients: Vec<Value> = http
                .get(format!("{base}/api/clients"))
                .send()
                .await?
                .json()
                .await?;
            if clients
                .iter()
                .any(|c| c["id"] == "listening-mac" && c["online"] == true && c["audio"] == true)
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;

    let started: Value = http
        .post(format!("{base}/api/audio"))
        .json(&json!({"client_id":"listening-mac"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(started["active"] == true, "{started}");
    ensure!(started["client_name"] == "Listening Mac", "{started}");
    let room = started["room_id"]
        .as_str()
        .context("session room")?
        .to_string();
    let mut saw_live = false;
    let mut saw_partial = false;
    let status = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let status: Value = http
                .get(format!("{base}/api/audio"))
                .send()
                .await?
                .json()
                .await?;
            ensure!(status["active"] == true, "session stopped: {status}");
            saw_live |= status["listening"] == true;
            saw_partial |= status["partial_text"]
                .as_str()
                .is_some_and(|t| !t.is_empty());
            if status["segments"].as_array().is_some_and(|s| s.len() >= 2) {
                return Ok::<_, anyhow::Error>(status);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;
    ensure!(
        status["segments"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("seven plus five")),
        "{status}"
    );
    ensure!(status["stage"] == "Listening…", "{status}");
    ensure!(saw_live, "CLI microphone activity must reach the backend");
    ensure!(
        saw_partial,
        "Speech previews must arrive before final utterances"
    );
    ensure!(
        status["heard_seconds"].as_f64().unwrap_or(0.0) > 3.0,
        "{status}"
    );

    let solved: Value = http
        .post(format!("{base}/api/audio/solve"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let memo_id = solved["solve_memo"]
        .as_str()
        .context("Solve returns its memo")?
        .to_string();
    ensure!(solved["solved_through"] == 2, "{solved}");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let memo = state.db.get(&memo_id)?.context("Memo disappeared")?;
            ensure!(memo.status != "error", "{:?}", memo.error);
            if memo.status == "complete" {
                ensure!(memo.input == "Solve", "{}", memo.input);
                ensure!(!memo.answer.is_empty());
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;
    let messages: Value = http
        .get(format!("{base}/api/rooms/{room}/sync"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        messages["messages"]
            .as_array()
            .is_some_and(|m| m.iter().any(|m| m["memo_id"] == memo_id)),
        "the answer is a message in the Audio Capture room: {messages}"
    );

    let stopped: Value = http
        .delete(format!("{base}/api/audio"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(stopped["active"] == false, "{stopped}");
    ensure!(stopped["error"].is_null(), "{stopped}");
    let messages: Value = http
        .get(format!("{base}/api/rooms/{room}/sync"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    // What the Mac heard is a notes message in the room, before the Solve.
    let list = messages["messages"].as_array().context("messages")?;
    let notes = list
        .iter()
        .find(|m| m["memo_kind"] == "audio")
        .context("notes message")?;
    ensure!(
        notes["text"]
            .as_str()
            .is_some_and(|t| t.contains("seven plus five")),
        "{notes}"
    );
    ensure!(notes["client_name"] == "Listening Mac", "{notes}");
    let solve = list
        .iter()
        .find(|m| m["memo_id"] == memo_id && m["role"] == "user")
        .context("Solve message")?;
    ensure!(
        notes["sequence"].as_i64() < solve["sequence"].as_i64(),
        "notes come before the Solve: {messages}"
    );
    ensure!(stopped["room_id"] == room.as_str(), "{stopped}");
    // A new backend instance restores both the transcript and the room's
    // conversation, but never reopens the microphone by itself.
    let reopened = AppState::open_with_credentials(data, true, None, Credentials::default())?;
    let restored = causewaybay_server::audio::current_status(&reopened.for_owner("local"));
    ensure!(!restored.active, "A restart must not resume listening");
    ensure!(restored.room_id == room, "Audio room was not restored");
    ensure!(
        serde_json::to_value(&restored.segments)? == stopped["segments"],
        "Transcript changed across restart"
    );
    ensure!(
        restored.solved_through == stopped["solved_through"].as_u64().unwrap(),
        "Solve coverage changed across restart"
    );
    ensure!(
        reopened.db.get(&memo_id)?.is_some(),
        "Answer was not restored"
    );
    drop(server);
    Ok(())
}
