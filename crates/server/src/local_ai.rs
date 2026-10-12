//! Local workers have a bounded JSON-lines protocol; neither prompts nor audio use argv.
use anyhow::{Context, Result, bail, ensure};
use causewaybay_shared::LocalAiSettings;
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Clone, Debug)]
pub struct Runtime {
    pub activity: std::sync::Arc<crate::ai_activity::Tracker>,
    pub apple_worker: PathBuf,
    /// The Rust MLX worker (`make mlx-ai`), which runs the Qwen models.
    pub mlx_worker: PathBuf,
    pub python: PathBuf,
    pub whisper_worker: PathBuf,
    /// Where downloaded on-device models live: `<data>/models/mlx`.
    pub models_dir: PathBuf,
    /// Hugging Face base URL for explicit model downloads; tests point it at loopback.
    pub hf_base: String,
    /// The Hugging Face cache (`HF_HUB_CACHE`, `HF_HOME`, or `~/.cache/huggingface/hub`),
    /// where models fetched by CausewaybayMaker or `hf download` are reused read-only.
    pub hf_cache: PathBuf,
    /// Unified memory override (`INVISIBLE_UNIFIED_MEMORY_BYTES`); otherwise read from the system.
    pub unified_memory: Option<u64>,
    /// The one model download at a time, and the single-lane MLX worker manager.
    pub downloads: std::sync::Arc<crate::local_models::Downloads>,
    pub mlx: std::sync::Arc<crate::mlx_manager::Manager>,
}
/// Resolve resources beside the installed executable, never the build machine.
pub fn distribution_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(5) {
            if dir.join("bin").is_dir() && dir.join("dist").is_dir() {
                return dir.to_owned();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
impl Default for Runtime {
    fn default() -> Self {
        let root = distribution_root();
        Self {
            activity: Default::default(),
            apple_worker: root.join("bin/invisible-apple-ai"),
            mlx_worker: root.join("bin/invisible-mlx"),
            python: PathBuf::from("python3"),
            whisper_worker: root.join("scripts/local-ai/transcribe.py"),
            models_dir: root.join("data/models/mlx"),
            hf_base: "https://huggingface.co".into(),
            hf_cache: crate::local_models::hf_cache_dir(),
            unified_memory: None,
            downloads: Default::default(),
            mlx: Default::default(),
        }
    }
}
impl Runtime {
    pub fn from_environment() -> Self {
        let mut r = Self::default();
        for (key, path) in [
            ("INVISIBLE_APPLE_WORKER", &mut r.apple_worker),
            ("INVISIBLE_MLX_WORKER", &mut r.mlx_worker),
            ("INVISIBLE_MLX_PYTHON", &mut r.python),
            ("INVISIBLE_WHISPER_WORKER", &mut r.whisper_worker),
        ] {
            if let Some(value) = std::env::var_os(key).filter(|v| !v.is_empty()) {
                *path = value.into();
            }
        }
        if let Some(base) = std::env::var("INVISIBLE_HF_BASE")
            .ok()
            .filter(|v| !v.is_empty())
        {
            r.hf_base = base;
        }
        r.unified_memory = std::env::var("INVISIBLE_UNIFIED_MEMORY_BYTES")
            .ok()
            .and_then(|v| v.trim().parse().ok());
        r
    }
    pub fn with_models_dir(mut self, dir: PathBuf) -> Self {
        self.models_dir = dir;
        self
    }
}

pub fn is_local(provider: &str) -> bool {
    matches!(provider, "apple" | "mlx")
}

/// Numeric loopback only: no DNS rebinding, credentials, proxies or redirects.
pub fn mlx_endpoint(base: &str, resource: &str) -> Result<String> {
    let url = reqwest::Url::parse(base).context("Invalid MLX URL")?;
    ensure!(
        url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "/v1" | "/v1/"),
        "MLX must use http://127.0.0.1:PORT/v1 or http://[::1]:PORT/v1 on the backend"
    );
    Ok(format!("{}/{resource}", base.trim_end_matches('/')))
}

pub fn validate(settings: &LocalAiSettings) -> Result<()> {
    ensure!(
        (2..=300).contains(&settings.auto_interval_seconds)
            && (2..=300).contains(&settings.local_interval_seconds)
            && (2..=300).contains(&settings.cloud_min_interval_seconds),
        "Auto interval must be between 2 and 300 seconds"
    );
    mlx_endpoint(&settings.mlx_url, "chat/completions")?;
    ensure!(
        matches!(settings.mlx_engine.as_str(), "builtin" | "service"),
        "MLX engine must be builtin or service"
    );
    ensure!(
        matches!(
            settings.speech_provider.as_str(),
            "auto" | "apple" | "mlx" | "grok" | "gemini"
        ),
        "Unknown speech provider"
    );
    ensure!(
        !settings.local_only || !matches!(settings.speech_provider.as_str(), "grok" | "gemini"),
        "Local-only mode requires a local speech provider"
    );
    ensure!(
        !settings.speech_locale.is_empty()
            && settings.speech_locale.len() <= 40
            && settings
                .speech_locale
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "Use a speech language such as en-US or ko-KR"
    );
    ensure!(
        settings.mlx_speech_model.len() <= 4096
            && !settings.mlx_speech_model.chars().any(char::is_control)
            && (settings.mlx_speech_model.is_empty()
                || std::path::Path::new(&settings.mlx_speech_model).is_absolute()),
        "MLX speech model must be an absolute path to an installed model directory"
    );
    Ok(())
}

pub fn valid_model(provider: &str, model: &str) -> bool {
    if provider == "apple" {
        return model == "system";
    }
    if crate::cli_ai::is_cli(provider) && model.is_empty() {
        return true;
    }
    !model.is_empty()
        && model.len() <= 200
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:".contains(c))
        && !model.contains("..")
        && (provider == "mlx" || !model.contains(['/', ':']))
}

/// Only the workers' controlled public error codes cross the boundary; each
/// maps to a fixed message, so no runtime diagnostics reach the browser.
pub fn worker_message(code: &str) -> &'static str {
    match code {
        "model_unavailable" => {
            "Apple Intelligence model is unavailable. Enable Apple Intelligence and finish model setup on the backend Mac."
        }
        "speech_model_missing" => {
            "Speech model is not installed. Prepare the language on the backend first."
        }
        "unsupported_language" => "This speech language is not supported on the backend.",
        "mlx_model_missing" => {
            "The on-device model is not installed. Download it in Backend settings."
        }
        "unsupported_audio" => {
            "Apple Speech could not decode this audio. Upload PCM WAV, M4A, or AIFF."
        }
        "request_rejected" => {
            "Apple's model could not answer this request. Try a shorter or rephrased question."
        }
        "spawn_failed" => "The MLX worker could not start on the backend. Run `make mlx-ai` there.",
        _ => "Local AI worker failed. Check its installation and model availability.",
    }
}

/// What a worker produced: its streamed text and the final `done` event, which
/// may carry token counts and timings.
#[derive(Debug, Default)]
pub struct WorkerOutput {
    pub text: String,
    pub done: Value,
}
impl WorkerOutput {
    pub fn count(&self, key: &str) -> Option<u64> {
        self.done[key].as_u64()
    }
}

/// Aborting the request drops and kills the worker, including during stdin writes.
/// stderr is deliberately not exposed: third-party runtimes may echo private inputs.
pub async fn worker(
    executable: &std::path::Path,
    args: &[&std::path::Path],
    request: Value,
    timeout: Duration,
    mut on_text: impl FnMut(&str) -> Result<()>,
) -> Result<WorkerOutput> {
    const MAX_OUTPUT: u64 = 2_000_000;
    let payload = serde_json::to_vec(&request)?;
    ensure!(payload.len() <= 1_000_000, "Local request is too large");
    tokio::time::timeout(timeout, async {
        let mut child = tokio::process::Command::new(executable)
            .args(args)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("HF_HUB_DISABLE_TELEMETRY", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("Local AI worker could not start; check backend installation")?;
        let mut stdin = child.stdin.take().context("Worker input is unavailable")?;
        stdin.write_all(&payload).await?;
        stdin.shutdown().await?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .context("Worker output is unavailable")?;
        let mut reader = BufReader::new(stdout.take(MAX_OUTPUT + 1));
        let mut bytes = Vec::new();
        let mut total = 0;
        let mut done = false;
        let mut done_event = Value::Null;
        let mut text = String::new();
        loop {
            bytes.clear();
            let n = reader.read_until(b'\n', &mut bytes).await?;
            if n == 0 {
                break;
            }
            total += n as u64;
            ensure!(
                total <= MAX_OUTPUT,
                "Local worker exceeded its output limit"
            );
            let event: Value =
                serde_json::from_slice(&bytes).context("Invalid local worker response")?;
            ensure!(!done, "Local worker sent data after completion");
            if let Some(error) = event["error"].as_str() {
                bail!("{}", worker_message(error));
            }
            if let Some(delta) = event["delta"].as_str() {
                text.push_str(delta);
                on_text(delta)?;
            } else if event["done"] == true {
                done = true;
                done_event = event;
            } else {
                bail!("Invalid local worker event");
            }
        }
        ensure!(
            child.wait().await?.success(),
            "Local worker exited unsuccessfully"
        );
        ensure!(done, "Local worker stopped before completing its response");
        ensure!(!text.trim().is_empty(), "Local worker returned no text");
        Ok(WorkerOutput {
            text,
            done: done_event,
        })
    })
    .await
    .context("Local AI timed out; its worker was stopped")?
}

pub async fn generate(
    config: &crate::providers::Config,
    system: &str,
    messages: &[crate::providers::Message],
    on_text: impl FnMut(&str) -> Result<()>,
) -> Result<crate::providers::Turn> {
    use crate::providers::Message;
    let mut prompt = String::new();
    for message in messages {
        let (role, text) = match message {
            Message::User(text) => ("User", text),
            Message::Assistant { text, calls } if calls.is_empty() => ("Assistant", text),
            _ => bail!(
                "Apple local currently supports text and OCR context; tool calls and direct image input are unavailable"
            ),
        };
        prompt.push_str(&format!("\n{role}:\n{text}\n"));
    }
    let output = worker(
        &config.runtime.apple_worker,
        &[],
        json!({"operation":"generate","system":system,"prompt":prompt}),
        Duration::from_secs(180),
        on_text,
    )
    .await?;
    Ok(crate::providers::Turn {
        text: output.text,
        calls: vec![],
        ..Default::default()
    })
}

/// The Qwen models through the backend's single-lane MLX worker manager. A
/// catalog model that is not installed yet is downloaded first, with progress
/// reported to the user; the worker is handed a directory, never a repo id,
/// so a request itself can never download. The model stays loaded for the
/// next request.
pub async fn generate_mlx(
    config: &crate::providers::Config,
    system: &str,
    messages: &[crate::providers::Message],
    on_text: impl FnMut(&str) -> Result<()>,
) -> Result<crate::providers::Turn> {
    use crate::providers::Message;
    ensure!(
        config.runtime.mlx_worker.is_file(),
        "The MLX worker is not built on the backend. Run `make mlx-ai` there."
    );
    let spec = causewaybay_shared::local_model_by_repo("mlx", &config.model);
    let (name, dir) = match spec {
        Some(spec) => (
            spec.name.to_owned(),
            crate::local_models::ensure_installed(&config.runtime, spec, config.progress.as_ref())
                .await?,
        ),
        None => (
            config.model.clone(),
            crate::local_models::resolve(&config.runtime, &config.model).with_context(|| {
                format!(
                    "On-device model {} is not installed. Download it in Backend settings.",
                    config.model
                )
            })?,
        ),
    };
    let mut rows = Vec::new();
    for message in messages {
        let (role, text) = match message {
            Message::User(text) => ("user", text),
            Message::Assistant { text, calls } if calls.is_empty() => ("assistant", text),
            _ => bail!(
                "MLX local currently supports text and OCR context; tool calls and direct image input are unavailable"
            ),
        };
        rows.push(json!({"role":role,"content":text}));
    }
    ensure!(!rows.is_empty(), "Nothing to ask the on-device model");
    let output = config
        .runtime
        .mlx
        .generate(
            &config.runtime.mlx_worker,
            &name,
            json!({"operation":"generate","model_dir":dir,"system":system,"messages":rows,"max_tokens":config.max_tokens.unwrap_or(2048)}),
            config.progress.as_ref(),
            on_text,
        )
        .await?;
    Ok(crate::providers::Turn {
        input_tokens: output.count("input_tokens"),
        output_tokens: output.count("output_tokens"),
        text: output.text,
        calls: vec![],
    })
}

pub async fn transcribe(
    config: &crate::providers::Config,
    provider: &str,
    bytes: &[u8],
) -> Result<String> {
    static SPEECH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let _permit = SPEECH_SLOTS.acquire().await?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= 32 * 1024 * 1024,
        "Audio must be between 1 byte and 32 MB"
    );
    // A private, generated temporary filename, unrelated to the uploaded name.
    let temp = tempfile::NamedTempFile::new()?;
    tokio::fs::write(temp.path(), bytes).await?;
    let request = json!({"operation":"transcribe","audio_path":temp.path(),"locale":config.local.speech_locale,"model":config.local.mlx_speech_model});
    let output = match provider {
        "apple" => {
            worker(
                &config.runtime.apple_worker,
                &[],
                request,
                Duration::from_secs(300),
                |_| Ok(()),
            )
            .await?
        }
        "mlx" => {
            worker(
                &config.runtime.python,
                &[&config.runtime.whisper_worker],
                request,
                Duration::from_secs(300),
                |_| Ok(()),
            )
            .await?
        }
        _ => bail!("Unknown local speech provider"),
    };
    Ok(output.text)
}
