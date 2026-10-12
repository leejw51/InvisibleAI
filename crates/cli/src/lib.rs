mod callbacks;
mod notes;
#[cfg(target_os = "macos")]
mod screen;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use causewaybay_shared::CaptureCommand;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

use clap::{CommandFactory, Parser, Subcommand};
#[derive(Parser)]
#[command(
    name = "rust-cli",
    args_conflicts_with_subcommands = true,
    after_help = "Connect to a backend (replace backend.example with your server's hostname or IP):\n  rust-cli http://backend.example:3000\n  rust-cli http://backend.example:3000 0xYOUR_WALLET_ADDRESS\n  rust-cli client --backend http://backend.example:3000 --name MacBook\n\nSettings are saved to ~/.causewaybaybotclient/client.jsonl.\nRun rust-cli without arguments to reconnect using the saved URL and owner wallet.\nIf no connection is saved, connection help is shown. Use --help to show help at any time.\nThe client waits and reconnects automatically if the backend is unavailable.",
    version,
    about = "Capture your MacBook screen for your Causewaybay backend"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// Backend URL (HTTP, HTTPS, WS, or WSS)
    backend_url: Option<String>,
    /// Only this wallet may capture; omit to share with approved workspace users
    #[arg(requires = "backend_url")]
    owner_wallet_address: Option<String>,
}
#[derive(Subcommand)]
enum Command {
    Client(Config),
}
#[derive(clap::Args, Default)]
struct Config {
    #[arg(long, conflicts_with = "shared")]
    owner_wallet: Option<String>,
    #[arg(long, help = "Allow any approved workspace user to capture")]
    shared: bool,
    #[arg(long, env = "CAUSEWAYBAY_BACKEND")]
    backend: Option<String>,
    #[arg(long, env = "APP_TOKEN", hide_env_values = true)]
    token: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long = "client-id")]
    id: Option<String>,
    #[arg(long, env = "CAUSEWAYBAY_CLIENT_CONFIG_DIR")]
    config_dir: Option<PathBuf>,
    #[arg(
        long = "fixture-image",
        help = "TEST ONLY: send this image instead of capturing"
    )]
    fixture: Option<PathBuf>,
    #[arg(long, help = "Exit after delivering one capture")]
    once: bool,
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..=60))]
    retry_seconds: Option<u64>,
}
struct Resolved {
    owner_wallet: Option<String>,
    config_dir: PathBuf,
    capture_dir: PathBuf,
    backend: String,
    token: Option<String>,
    name: String,
    id: String,
    fixture: Option<PathBuf>,
    once: bool,
    retry_seconds: u64,
}
#[derive(Default)]
struct CaptureTask(Option<tokio::task::JoinHandle<()>>);
impl Drop for CaptureTask {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}
#[tokio::main]
pub async fn run() -> Result<()> {
    let parsed = Args::parse();
    let resume_saved = parsed.command.is_none() && parsed.backend_url.is_none();
    let args = match (parsed.command, parsed.backend_url) {
        (Some(Command::Client(config)), _) => config,
        (None, backend_url) => Config {
            shared: backend_url.is_some() && parsed.owner_wallet_address.is_none(),
            backend: backend_url,
            owner_wallet: parsed.owner_wallet_address,
            token: std::env::var("APP_TOKEN").ok(),
            config_dir: std::env::var_os("CAUSEWAYBAY_CLIENT_CONFIG_DIR").map(PathBuf::from),
            ..Default::default()
        },
    };
    let Some(mut config) = resolve_config(args, resume_saved)? else {
        Args::command().print_help()?;
        println!();
        return Ok(());
    };
    let automatic_token = config.token.is_none();
    persist(&config)?;
    println!(
        "Config: {}",
        config.config_dir.join("client.jsonl").display()
    );
    println!(
        "Causewaybay client · {}\nConnecting to {}",
        config.name, config.backend
    );
    println!(
        "Capture access: {}",
        config
            .owner_wallet
            .as_deref()
            .unwrap_or("All approved workspace users")
    );
    println!(
        "TextEdit Auto: {}",
        if config.owner_wallet.is_some() {
            "account enrolled; start automatic replies in the web UI"
        } else {
            "shared capture only on wallet backends; use the account command in Devices to enable replies"
        }
    );
    if config.fixture.is_some() {
        println!("TEST FIXTURE MODE: no live screen will be captured.");
    } else {
        #[cfg(target_os = "macos")]
        println!(
            "Screen capture permission: {}",
            if screen::has_access() {
                "granted"
            } else {
                "not confirmed by preflight; Capture will try macOS screencapture and request access if needed"
            }
        );
    }
    loop {
        if automatic_token {
            let local = reqwest::Url::parse(&config.backend)
                .ok()
                .is_some_and(|url| {
                    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
                });
            if local {
                config.token = causewaybay_shared::config::directory()
                    .ok()
                    .and_then(|dir| std::fs::read_to_string(dir.join("capture-token")).ok());
            }
        }
        tokio::select! {
            result=connected(&mut config)=>{match result{Ok(()) if config.once=>return Ok(()),Ok(())=>eprintln!("Connection closed; reconnecting…"),Err(e)=>{eprintln!("Connection failed: {e}");if config.once{return Err(e);}}}},
            _=tokio::signal::ctrl_c()=>return Ok(())
        }
        if !wait_for_retry(Duration::from_secs(config.retry_seconds), async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        {
            return Ok(());
        }
    }
}
fn resolve_config(args: Config, resume_saved: bool) -> Result<Option<Resolved>> {
    let dir = args
        .config_dir
        .unwrap_or_else(causewaybay_shared::config::client_directory);
    let stored = causewaybay_shared::config::read(&dir, "client")?;
    if resume_saved
        && stored["backend"]
            .as_str()
            .is_none_or(|url| url.trim().is_empty())
    {
        return Ok(None);
    }
    let owner_wallet = if args.shared {
        None
    } else {
        args.owner_wallet
            .or_else(|| stored["owner_wallet"].as_str().map(str::to_string))
    };
    let owner_wallet = owner_wallet
        .map(|v| causewaybay_shared::identity::normalize_address(&v))
        .transpose()
        .map_err(anyhow::Error::msg)?;
    let config = Resolved {
        owner_wallet,
        config_dir: dir.clone(),
        capture_dir: dir.join("screenshots"),
        backend: args
            .backend
            .or_else(|| stored["backend"].as_str().map(str::to_string))
            .unwrap_or_else(|| "http://127.0.0.1:3000".into()),
        token: args.token.filter(|v| !v.is_empty()),
        name: args
            .name
            .or_else(|| stored["name"].as_str().map(str::to_string))
            .unwrap_or_else(computer_name),
        id: args
            .id
            .or_else(|| stored["client_id"].as_str().map(str::to_string))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        fixture: args.fixture,
        once: args.once,
        retry_seconds: args
            .retry_seconds
            .or_else(|| stored["retry_seconds"].as_u64())
            .unwrap_or(3)
            .clamp(1, 60),
    };
    ensure!(
        ["http://", "https://", "ws://", "wss://"]
            .iter()
            .any(|prefix| config.backend.starts_with(prefix)),
        "Use an HTTP(S) or WS(S) backend URL"
    );
    Ok(Some(config))
}

async fn wait_for_retry(delay: Duration, shutdown: impl std::future::Future<Output = ()>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = shutdown => false,
    }
}

async fn connected(c: &mut Resolved) -> Result<()> {
    let url = c
        .backend
        .trim_end_matches('/')
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
        + "/api/client/ws";
    let mut request = url.into_client_request()?;
    if let Some(token) = &c.token {
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse()?);
    }
    let (socket, _) =
        tokio::time::timeout(Duration::from_secs(15), connect_async(request)).await??;
    let (mut sink, mut stream) = socket.split();
    sink.send(Message::Text(
        json!({"type":"hello","id":c.id,"name":c.name,"platform":std::env::consts::OS,"callbacks":"sse","device_key":causewaybay_shared::config::secret(&c.config_dir, "device-key")?,"owner_wallet":c.owner_wallet,"textedit_notes_v1":cfg!(target_os="macos"),"mac_actions_v1":cfg!(target_os="macos")})
            .to_string()
            .into(),
    ))
    .await?;
    let (tx, mut rx) = {
        let (a, b) = tokio::sync::mpsc::channel::<Value>(1);
        (a, b)
    };
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    let mut capture_task = CaptureTask::default();
    let mut notes_task = CaptureTask::default();
    let mut note_generation = String::new();
    let mut note_seen = std::collections::VecDeque::new();
    let mut active_request = String::new();
    let mut callbacks_task = CaptureTask::default();
    let (callback_tx, mut callback_rx) = tokio::sync::mpsc::channel(8);
    let mut seen = std::collections::VecDeque::new();
    let mut last_received = tokio::time::Instant::now();
    loop {
        let v: Value = tokio::select! {
            _=heartbeat.tick()=>{
                ensure!(last_received.elapsed()<Duration::from_secs(35), "Backend heartbeat timed out");
                sink.send(Message::Text(json!({"type":"heartbeat"}).to_string().into())).await?;
                continue;
            },
            Some(result)=rx.recv()=>{sink.send(Message::Text(result.to_string().into())).await?;continue;},
            Some(callback)=callback_rx.recv()=>callback,
            message=stream.next()=>{
                let Some(message)=message else{break};
                let message=message?;
                last_received=tokio::time::Instant::now();
                let Message::Text(text)=message else{if message.is_close(){break;}continue};
                serde_json::from_str(&text)?
            }
        };
        match v["type"].as_str().unwrap_or("") {
            "ready" => {
                println!("Connected. Waiting for capture or notes commands from the Rust backend.");
                let session = v["session"]
                    .as_str()
                    .context("Backend does not support SSE callbacks; update the backend")?
                    .to_string();
                callbacks_task.0 = Some(tokio::spawn(callbacks::run(
                    c.backend.clone(),
                    c.id.clone(),
                    session,
                    c.token.clone(),
                    callback_tx.clone(),
                )));
            }
            "notification" => {
                if v["kind"] == "capture.remove" {
                    if let Some(request) = v["data"]["request_id"].as_str() {
                        if active_request == request
                            && let Some(task) = capture_task.0.take()
                        {
                            task.abort();
                            let _ = task.await;
                        }
                        remove_capture(&c.capture_dir, request).await?;
                    }
                } else {
                    println!("Notification: {}", v["data"]);
                }
            }
            "node_config" => {
                if let Some(name) = v["name"].as_str()
                    && c.name != name
                {
                    c.name = name.to_string();
                    persist(c)?;
                    println!("Computer name updated: {}", c.name);
                }
            }
            "notes_command" => {
                let mut command = v.clone();
                let id = v["request_id"].as_str().unwrap_or("").to_string();
                let generation = v["generation"].as_str().unwrap_or("").to_string();
                let operation = v["operation"].as_str().unwrap_or("");
                if operation == "stop" {
                    if note_generation == generation {
                        note_generation.clear();
                        if let Some(task) = notes_task.0.take() {
                            task.abort();
                            let _ = task.await;
                        }
                    }
                    sink.send(Message::Text(
                        json!({"type":"notes_result","request_id":id,"ok":true})
                            .to_string()
                            .into(),
                    ))
                    .await?;
                    continue;
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis() as i64;
                let error = if note_seen.contains(&id) {
                    Some("Note command already handled")
                } else if generation.is_empty()
                    || v["expires_at"]
                        .as_i64()
                        .is_none_or(|t| t < now || t > now + 60_000)
                {
                    Some("Expired note command")
                } else if notes_task.0.as_ref().is_some_and(|t| !t.is_finished()) {
                    Some("TextEdit is busy")
                } else if operation != "begin"
                    && operation != "open_url"
                    && generation != note_generation
                {
                    // Note reads and appends need the generation armed by `begin`.
                    // `open_url` is a one-shot action: remote-only runs never send
                    // `begin`, and the backend already requires a live generation.
                    Some("Auto has stopped")
                } else {
                    None
                };
                if let Some(error) = error {
                    sink.send(Message::Text(
                        json!({"type":"notes_result","request_id":id,"ok":false,"error":error})
                            .to_string()
                            .into(),
                    ))
                    .await?;
                    continue;
                }
                if operation == "begin" {
                    note_generation = generation;
                    command["operation"] = json!("read");
                }
                note_seen.push_back(id.clone());
                if note_seen.len() > 256 {
                    note_seen.pop_front();
                }
                let sender = tx.clone();
                let open = operation == "open_url";
                notes_task.0 = Some(tokio::spawn(async move {
                    let result = if open {
                        notes::open_url(&command).await
                    } else {
                        notes::execute(&command).await
                    };
                    let response = match result {
                        Ok(snapshot) => {
                            json!({"type":"notes_result","request_id":id,"ok":true,"snapshot":snapshot["snapshot"]})
                        }
                        Err(e) => {
                            json!({"type":"notes_result","request_id":id,"ok":false,"error":e.to_string(),"error_code":e.downcast_ref::<causewaybay_shared::NoteError>(),"app":command["app"].as_str().filter(|a|*a!="auto")})
                        }
                    };
                    let _ = sender.send(response).await;
                }));
            }
            "capture" => {
                let command: CaptureCommand = serde_json::from_value(v)?;
                if seen.contains(&command.request_id) {
                    continue;
                }
                seen.push_back(command.request_id.clone());
                if seen.len() > 256 {
                    seen.pop_front();
                }
                let sender = tx.clone();
                let fixture = c.fixture.clone();
                let capture_dir = c.capture_dir.clone();
                if capture_task.0.as_ref().is_some_and(|t| !t.is_finished()) {
                    sender.send(json!({"type":"capture_result","request_id":command.request_id,"error":"This client is already capturing. Try again shortly."})).await?;
                    continue;
                }
                active_request = command.request_id.clone();
                println!(
                    "Capturing display {} ({}) via SSE…",
                    command.display, command.mode
                );
                capture_task.0 = Some(tokio::spawn(async move {
                    let result = capture(&command, fixture, &capture_dir).await;
                    let response = match result {
                        Ok(image) => {
                            println!("Uploading screenshot ({} bytes)…", image.len());
                            json!({"type":"capture_result","request_id":command.request_id,"image_base64":STANDARD.encode(image)})
                        }
                        Err(e) => {
                            let error = format!("{e:#}");
                            eprintln!("Capture failed: {error}");
                            json!({"type":"capture_result","request_id":command.request_id,"error":error})
                        }
                    };
                    let _ = sender.send(response).await;
                }));
            }
            "capture_ack" => {
                if v["ok"] == true {
                    println!("Screenshot received by backend.");
                } else {
                    eprintln!(
                        "Capture failed: {}",
                        v["error"].as_str().unwrap_or("Backend rejected screenshot")
                    );
                }
                if c.once {
                    ensure!(v["ok"] == true, "Backend rejected capture: {}", v["error"]);
                    if let Some(task) = capture_task.0.take() {
                        let _ = task.await;
                    }
                    return Ok(());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn remove_capture(directory: &std::path::Path, request: &str) -> Result<()> {
    let request = uuid::Uuid::parse_str(request).context("Invalid capture deletion GUID")?;
    match tokio::fs::remove_file(directory.join(format!("{request}.png"))).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    // Also erase captures interrupted by a process crash before final rename.
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let prefix = format!("pending-{request}-");
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_string_lossy().starts_with(&prefix)
            && entry.file_type().await?.is_dir()
        {
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
    }
    Ok(())
}

async fn capture(
    command: &CaptureCommand,
    fixture: Option<PathBuf>,
    directory: &std::path::Path,
) -> Result<Vec<u8>> {
    if let Some(path) = fixture {
        return tokio::fs::read(&path)
            .await
            .with_context(|| format!("Could not read capture fixture {}", path.display()));
    }
    #[cfg(target_os = "macos")]
    {
        screen::prepare(&command.mode, command.display)?;
        std::fs::create_dir_all(directory)
            .context("Could not create the client screenshots directory")?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        // screencapture silently rejects hidden destinations such as NamedTempFile's .tmp*.png.
        let request =
            uuid::Uuid::parse_str(&command.request_id).context("Invalid capture request GUID")?;
        let file = directory.join(format!("{request}.png"));
        // Aborting capture removes the parent directory before erasure finishes.
        // A late OS capture process therefore cannot recreate the final original.
        let pending = tempfile::Builder::new()
            .prefix(&format!("pending-{request}-"))
            .tempdir_in(directory)?;
        let staged = pending.path().join("capture.png");
        let mut cmd = tokio::process::Command::new("/usr/sbin/screencapture");
        cmd.args(["-x", "-t", "png"]);
        if command.mode == "region" {
            cmd.arg("-i");
        } else {
            cmd.arg(format!("-D{}", command.display));
        }
        cmd.arg(&staged).kill_on_drop(true);
        let result = tokio::time::timeout(Duration::from_secs(90), cmd.output())
            .await
            .map_err(|_| anyhow::anyhow!("Screen selection timed out"))?
            .context("Could not start /usr/sbin/screencapture on the capture computer")?;
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        screen::complete(result.status.success() && staged.is_file())
            .map_err(|e| anyhow::anyhow!("{e} {}", diagnostic.trim()))?;
        let bytes = tokio::fs::read(&staged).await.with_context(||format!("Screenshot was not created on the capture Mac. Finish the region selection, or unlock the Mac and retry display {}. {}",command.display,diagnostic.trim()))?;
        ensure!(!bytes.is_empty(), "Capture was cancelled");
        ensure!(bytes.len() <= 25 * 1024 * 1024, "Screenshot exceeds 25 MB");
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&staged, &file)?;
        println!("Screenshot saved: {}", file.display());
        Ok(bytes)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (command, directory);
        anyhow::bail!(
            "Live capture currently requires macOS. --fixture-image is available for transport testing."
        )
    }
}

fn persist(c: &Resolved) -> Result<()> {
    causewaybay_shared::config::append(
        &c.config_dir,
        "client",
        json!({"backend":c.backend,"name":c.name,"client_id":c.id,"retry_seconds":c.retry_seconds,"owner_wallet":c.owner_wallet}),
    )?;
    Ok(())
}
fn computer_name() -> String {
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[
            ("/usr/sbin/scutil", &["--get", "ComputerName"]),
            ("/bin/hostname", &[]),
        ]
    } else {
        &[("hostname", &[])]
    };
    for (program, args) in commands {
        if let Ok(output) = std::process::Command::new(program).args(*args).output()
            && output.status.success()
        {
            let name = String::from_utf8_lossy(&output.stdout)
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(60)
                .collect::<String>();
            let mut name = name;
            while name.len() > 120 {
                name.pop();
            }
            if !name.is_empty() {
                return name;
            }
        }
    }
    "Capture computer".into()
}
#[cfg(test)]
mod argument_tests {
    use super::*;
    #[test]
    fn shortcut_and_explicit_command_are_supported() {
        assert!(
            Args::try_parse_from(["rust-cli"])
                .unwrap()
                .command
                .is_none()
        );
        assert_eq!(
            Args::try_parse_from(["rust-cli", "http://backend.example:3000"])
                .unwrap()
                .backend_url
                .as_deref(),
            Some("http://backend.example:3000")
        );
        assert!(matches!(
            Args::try_parse_from([
                "rust-cli",
                "client",
                "--backend",
                "http://localhost:3000",
                "--name",
                "My laptop"
            ])
            .unwrap()
            .command,
            Some(Command::Client(_))
        ));
    }
}

#[cfg(test)]
mod connection_tests {
    use super::*;

    const BACKEND: &str = "http://backend.example:3000";
    const OWNER: &str = "0x1111111111111111111111111111111111111111";

    fn saved_config() -> Value {
        json!({
            "backend": BACKEND,
            "owner_wallet": OWNER,
            "client_id": "saved-node",
            "name": "Saved computer",
            "retry_seconds": 7,
        })
    }

    fn args(dir: &std::path::Path) -> Config {
        Config {
            config_dir: Some(dir.to_path_buf()),
            ..Default::default()
        }
    }

    #[test]
    fn no_saved_connection_requests_help_without_creating_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for stored in [Value::Null, json!({"backend":""}), json!({"backend":" "})] {
            if !stored.is_null() {
                causewaybay_shared::config::append(dir.path(), "client", stored)?;
            }
            let before = std::fs::read(dir.path().join("client.jsonl")).ok();
            assert!(resolve_config(args(dir.path()), true)?.is_none());
            assert_eq!(std::fs::read(dir.path().join("client.jsonl")).ok(), before);
        }
        Ok(())
    }

    #[test]
    fn saved_connection_restores_url_owner_identity_name_and_retry() -> Result<()> {
        let dir = tempfile::tempdir()?;
        causewaybay_shared::config::append(dir.path(), "client", saved_config())?;
        let config = resolve_config(args(dir.path()), true)?.unwrap();
        assert_eq!(config.backend, BACKEND);
        assert_eq!(config.owner_wallet.as_deref(), Some(OWNER));
        assert_eq!(config.id, "saved-node");
        assert_eq!(config.name, "Saved computer");
        assert_eq!(config.retry_seconds, 7);
        Ok(())
    }

    #[test]
    fn explicit_settings_replace_saved_values_and_shared_access_clears_owner() -> Result<()> {
        let dir = tempfile::tempdir()?;
        causewaybay_shared::config::append(dir.path(), "client", saved_config())?;
        let config = resolve_config(
            Config {
                backend: Some("https://new-backend.example".into()),
                owner_wallet: Some("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into()),
                retry_seconds: Some(2),
                ..args(dir.path())
            },
            false,
        )?
        .unwrap();
        assert_eq!(config.backend, "https://new-backend.example");
        assert_eq!(
            config.owner_wallet.as_deref(),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(config.retry_seconds, 2);
        let shared = resolve_config(
            Config {
                shared: true,
                ..args(dir.path())
            },
            false,
        )?
        .unwrap();
        assert!(shared.owner_wallet.is_none());
        persist(&shared)?;
        assert!(
            resolve_config(args(dir.path()), true)?
                .unwrap()
                .owner_wallet
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn persisted_connection_omits_token_and_survives_interrupted_append() -> Result<()> {
        use std::io::Write;
        let dir = tempfile::tempdir()?;
        let config = resolve_config(
            Config {
                backend: Some(BACKEND.into()),
                owner_wallet: Some(OWNER.into()),
                token: Some("unit-test-token-not-for-persistence".into()),
                name: Some("Test computer".into()),
                ..args(dir.path())
            },
            false,
        )?
        .unwrap();
        persist(&config)?;
        let path = dir.path().join("client.jsonl");
        let text = std::fs::read_to_string(&path)?;
        assert!(!text.contains("unit-test-token"));
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(b"{\"config\":")?;
        let restored = resolve_config(args(dir.path()), true)?.unwrap();
        assert_eq!(restored.backend, BACKEND);
        assert_eq!(restored.owner_wallet.as_deref(), Some(OWNER));
        assert_eq!(restored.id, config.id);
        assert!(restored.token.is_none());
        Ok(())
    }

    #[test]
    fn retry_defaults_to_three_seconds_and_saved_values_are_bounded() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for (stored, expected) in [
            (Value::Null, 3),
            (json!(0), 1),
            (json!(9), 9),
            (json!(100), 60),
        ] {
            let mut value = saved_config();
            value["retry_seconds"] = stored;
            causewaybay_shared::config::append(dir.path(), "client", value)?;
            assert_eq!(
                resolve_config(args(dir.path()), true)?
                    .unwrap()
                    .retry_seconds,
                expected
            );
        }
        Ok(())
    }

    #[test]
    fn invalid_saved_owner_or_url_is_rejected_without_overwriting_settings() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for (field, invalid) in [
            ("owner_wallet", "invalid-wallet"),
            ("backend", "file:///invalid"),
        ] {
            let mut value = saved_config();
            value[field] = json!(invalid);
            causewaybay_shared::config::append(dir.path(), "client", value)?;
            let before = std::fs::read(dir.path().join("client.jsonl"))?;
            assert!(resolve_config(args(dir.path()), true).is_err());
            assert_eq!(std::fs::read(dir.path().join("client.jsonl"))?, before);
        }
        Ok(())
    }

    #[tokio::test]
    async fn capture_deletion_is_idempotent_and_confined_to_request_guid() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let id = uuid::Uuid::new_v4().to_string();
        let file = dir.path().join(format!("{id}.png"));
        std::fs::write(&file, b"private screenshot")?;
        std::fs::write(dir.path().join("keep.png"), b"keep")?;
        let pending = dir.path().join(format!("pending-{id}-interrupted"));
        std::fs::create_dir(&pending)?;
        std::fs::write(
            pending.join("capture.png"),
            b"private interrupted screenshot",
        )?;
        remove_capture(dir.path(), &id).await?;
        remove_capture(dir.path(), &id).await?;
        assert!(!pending.exists());
        assert!(!file.exists());
        assert!(remove_capture(dir.path(), "../keep").await.is_err());
        assert!(dir.path().join("keep.png").exists());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn retry_wait_does_not_finish_before_the_configured_delay() {
        for seconds in [1, 3, 60] {
            let wait = wait_for_retry(Duration::from_secs(seconds), std::future::pending());
            tokio::pin!(wait);
            assert!(futures_util::poll!(wait.as_mut()).is_pending());
            tokio::time::advance(Duration::from_millis(seconds * 1000 - 1)).await;
            assert!(futures_util::poll!(wait.as_mut()).is_pending());
            tokio::time::advance(Duration::from_millis(1)).await;
            assert!(wait.await);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_interrupts_retry_sleep_without_waiting_for_the_deadline() {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let start = tokio::time::Instant::now();
        let wait = wait_for_retry(Duration::from_secs(60), async {
            let _ = stopped.await;
        });
        tokio::pin!(wait);
        assert!(futures_util::poll!(wait.as_mut()).is_pending());
        tokio::time::advance(Duration::from_secs(1)).await;
        stop.send(()).unwrap();
        assert!(!wait.await);
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }
}
