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
