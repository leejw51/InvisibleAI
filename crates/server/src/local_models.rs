//! On-device model catalog: what is installed, whether it fits this Mac's GPU
//! memory, and the one download running at a time. A download starts when the
//! admin asks in Backend settings or when a user's request needs the admin's
//! chosen model; the worker itself never downloads. Progress is pushed to
//! Backend settings over SSE and to the asking user through their memo events.
use crate::{Api, AppState, PrivateState, local_ai::Runtime};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use causewaybay_shared::{
    LOCAL_MODELS, LocalModel, LocalModelSpec, LocalModels, ModelDownload, ModelProgress,
    SetupCheck, SetupReport,
};
use futures_util::StreamExt;
use serde_json::Value;
use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
/// Least time between two catalog snapshots on the SSE stream.
pub const SNAPSHOT_GAP: Duration = Duration::from_millis(650);

/// Apple Silicon unified memory: the CPU and GPU share it, so this is the GPU
/// memory a model can use. `INVISIBLE_UNIFIED_MEMORY_BYTES` overrides it for tests.
pub fn unified_memory_bytes(runtime: &Runtime) -> Option<u64> {
    if runtime.unified_memory.is_some() {
        return runtime.unified_memory;
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        String::from_utf8(out.stdout).ok()?.trim().parse().ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
/// Metal's recommended working set: about two thirds of unified memory on
/// Macs with 36 GB or less, three quarters above that.
pub fn gpu_budget_bytes(total: u64) -> u64 {
    if total <= 36 * GIB as u64 {
        total / 3 * 2
    } else {
        total / 4 * 3
    }
}
/// `fits`, `tight` (within 15% of the budget), `too_large`, or `unknown`.
pub fn fit(memory_gb: f64, budget: Option<u64>) -> &'static str {
    let Some(budget) = budget else {
        return "unknown";
    };
    let need = memory_gb * GIB;
    let budget = budget as f64;
    if need <= 0.0 {
        "fits"
    } else if need > budget {
        "too_large"
    } else if need > budget * 0.85 {
        "tight"
    } else {
        "fits"
    }
}

/// The Hugging Face cache layout CausewaybayMaker and `hf download` use, so a
/// model fetched there is found without a second copy.
pub fn hf_cache_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("HF_HUB_CACHE").filter(|v| !v.is_empty()) {
        return p.into();
    }
    let home = std::env::var_os("HF_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache/huggingface")
        });
    home.join("hub")
}
pub fn cached_snapshot(hf_cache: &Path, repo: &str) -> Option<PathBuf> {
    let base = hf_cache.join(format!("models--{}", repo.replace('/', "--")));
    let rev = std::fs::read_to_string(base.join("refs/main")).ok()?;
    let dir = base.join("snapshots").join(rev.trim());
    complete(&dir).then_some(dir)
}
/// The backend's own copy: `<data>/models/mlx/<org>--<name>/`, flat files.
pub fn local_dir(models_dir: &Path, repo: &str) -> PathBuf {
    models_dir.join(repo.replace('/', "--"))
}
fn file_size(path: &Path) -> Option<u64> {
    // Follows symlinks: the Hugging Face cache links snapshots to blobs.
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}
/// Every weight the index names (or model.safetensors) is present and non-empty,
/// nothing is still a `.part`, and the tokenizer and config are there.
pub fn complete(dir: &Path) -> bool {
    if file_size(&dir.join("config.json")).is_none_or(|n| n == 0)
        || file_size(&dir.join("tokenizer.json")).is_none_or(|n| n == 0)
    {
        return false;
    }
    let weights: Vec<String> =
        match std::fs::read_to_string(dir.join("model.safetensors.index.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        {
            Some(index) => {
                let mut names: Vec<String> = index["weight_map"]
                    .as_object()
                    .map(|m| {
                        m.values()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                names.sort();
                names.dedup();
                names
            }
            None => vec!["model.safetensors".into()],
        };
    !weights.is_empty()
        && weights.iter().all(|w| {
            !w.contains("..")
                && !dir.join(format!("{w}.part")).exists()
                && file_size(&dir.join(w)).is_some_and(|n| n > 0)
        })
}
/// An absolute installed directory, the backend's copy, or the Hugging Face cache.
pub fn resolve(runtime: &Runtime, model: &str) -> Option<PathBuf> {
    let p = Path::new(model);
    if p.is_absolute() {
        return complete(p).then(|| p.to_path_buf());
    }
    let own = local_dir(&runtime.models_dir, model);
    if complete(&own) {
        return Some(own);
    }
    cached_snapshot(&runtime.hf_cache, model)
}
pub fn installed(runtime: &Runtime, spec: &LocalModelSpec) -> bool {
    spec.provider == "apple" || resolve(runtime, spec.model).is_some()
}

pub struct Downloads {
    current: Mutex<Option<ModelDownload>>,
    cancel: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Bumped on every change so SSE watchers and waiting jobs wake up.
    changed: tokio::sync::watch::Sender<u64>,
}
impl Default for Downloads {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
            cancel: AtomicBool::new(false),
            task: Mutex::new(None),
            changed: tokio::sync::watch::channel(0).0,
        }
    }
}
impl std::fmt::Debug for Downloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Downloads({:?})", self.current())
    }
}
impl Downloads {
    pub fn current(&self) -> Option<ModelDownload> {
        self.current.lock().unwrap().clone()
    }
    pub fn watch(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }
    fn bump(&self) {
        self.changed.send_modify(|v| *v += 1);
    }
    fn update(&self, f: impl FnOnce(&mut ModelDownload)) {
        if let Some(d) = self.current.lock().unwrap().as_mut() {
            f(d);
        }
        self.bump();
    }
    pub fn cancel(&self) -> bool {
        let active = self.current().is_some_and(|d| d.active());
        if active {
            self.cancel.store(true, Ordering::SeqCst);
        }
        active
    }
}

pub fn list(s: &AppState) -> LocalModels {
    let total = unified_memory_bytes(&s.local_runtime);
    let budget = total.map(gpu_budget_bytes);
    let download = s.local_runtime.downloads.current();
    let mlx = s.local_runtime.mlx.status();
    let models = LOCAL_MODELS
        .iter()
        .map(|m| LocalModel {
            id: m.id.into(),
            provider: m.provider.into(),
            model: m.model.into(),
            name: m.name.into(),
            note: m.note.into(),
            size_gb: m.size_gb,
            memory_gb: m.memory_gb,
            installed: installed(&s.local_runtime, m),
            fit: fit(m.memory_gb, budget).into(),
            download: download.clone().filter(|d| d.id == m.id),
        })
        .collect();
    LocalModels {
        unified_memory_bytes: total,
        gpu_budget_bytes: budget,
        worker_available: s.local_runtime.mlx_worker.is_file(),
        apple_available: s.local_runtime.apple_worker.is_file(),
        loaded_model: mlx.loaded_model,
        busy: mlx.busy,
        waiting: mlx.waiting,
        models,
    }
}
pub async fn get(PrivateState(s): PrivateState) -> Api<Json<LocalModels>> {
    Ok(Json(list(&s)))
}
/// The catalog as a live stream: a snapshot now, then one whenever a download
/// progresses or the worker loads, unloads, starts or finishes a request.
pub async fn events(
    PrivateState(s): PrivateState,
    headers: HeaderMap,
) -> Api<impl axum::response::IntoResponse> {
    let mut downloads = s.local_runtime.downloads.watch();
    let mut mlx = s.local_runtime.mlx.watch();
    downloads.mark_changed();
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(Event::default().retry(Duration::from_secs(2)).comment("models ready"));
        yield Ok(Event::default().json_data(list(&s)).unwrap());
        loop {
            if crate::auth::authorize(&s, &headers, "GET", "/api/local-ai/models").is_err() { break; }
            tokio::select! {
                changed = downloads.changed() => { if changed.is_err() { break; } }
                changed = mlx.changed() => { if changed.is_err() { break; } }
                _ = tokio::time::sleep(Duration::from_secs(15)) => {}
            }
            // A download changes many times a second; one snapshot every
            // SNAPSHOT_GAP lets the browser's eased bar finish each move.
            tokio::time::sleep(SNAPSHOT_GAP).await;
            downloads.mark_unchanged();
            mlx.mark_unchanged();
            yield Ok(Event::default().json_data(list(&s)).unwrap());
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

/// The directory of an installed catalog model, downloading it first when a
/// user is asking for it. The model is the admin's choice (users cannot pick
/// MLX models), so its download is authorized; one that cannot fit GPU memory
/// is refused. A background job (Wiki tagging, notes, remote prompts) reports
/// no progress and never starts a download: it waits on one already running
/// or fails plainly, so a 16 GB fetch only ever begins on a person's request.
pub async fn ensure_installed(
    runtime: &Runtime,
    spec: &LocalModelSpec,
    progress: Option<&crate::providers::ProgressSink>,
) -> Result<PathBuf> {
    if let Some(dir) = resolve(runtime, spec.model) {
        return Ok(dir);
    }
    let interactive = progress.is_some();
    if !interactive
        && !runtime
            .downloads
            .current()
            .is_some_and(|d| d.id == spec.id && d.active())
    {
        bail!(
            "On-device model {} is not installed. Download it in Backend settings or ask it a question.",
            spec.name
        );
    }
    let report = |p: ModelProgress| {
        if let Some(sink) = progress {
            sink(p);
        }
    };
    let downloads = &runtime.downloads;
    let mut watch = downloads.watch();
    loop {
        if let Some(dir) = resolve(runtime, spec.model) {
            return Ok(dir);
        }
        match downloads.current() {
            Some(d) if d.id == spec.id && d.active() => {
                report(ModelProgress {
                    label: format!("Downloading {}", spec.name),
                    detail: format!("{} of {}{}", gb(d.done_bytes), gb(d.total_bytes), eta(&d)),
                    fraction: Some(d.fraction()),
                });
            }
            Some(d) if d.id == spec.id && d.finished => {
                if let Some(e) = d.error {
                    bail!("Downloading {} failed: {e}", spec.name);
                }
                if d.cancelled {
                    bail!("Downloading {} was cancelled", spec.name);
                }
                // Finished but not resolvable: fall through and start again.
                start(downloads, runtime, spec.id, false)?;
            }
            Some(d) if d.active() && interactive => {
                report(ModelProgress {
                    label: format!("Waiting for another download ({})", d.model),
                    detail: String::new(),
                    fraction: None,
                });
            }
            _ if interactive => {
                start(downloads, runtime, spec.id, false)?;
            }
            _ => bail!(
                "On-device model {} is not installed. Download it in Backend settings or ask it a question.",
                spec.name
            ),
        }
        if tokio::time::timeout(Duration::from_millis(500), watch.changed())
            .await
            .is_ok_and(|r| r.is_err())
        {
            bail!("Download tracking stopped");
        }
    }
}
fn gb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}
fn eta(d: &ModelDownload) -> String {
    if d.bytes_per_second == 0 || d.total_bytes <= d.done_bytes {
        return String::new();
    }
    let secs = (d.total_bytes - d.done_bytes) / d.bytes_per_second;
    if secs >= 60 {
        format!(" · {}m {}s left", secs / 60, secs % 60)
    } else {
        format!(" · {secs}s left")
    }
}

#[derive(serde::Deserialize, Default)]
pub struct DownloadRequest {
    pub id: String,
    #[serde(default)]
    pub force: bool,
}
/// Admin only (the `/admin/` prefix). One download at a time; a finished or
/// failed one is replaced. Models that cannot fit GPU memory are refused
/// unless `force` is set, so a 16 GB download never ends in an unusable model.
pub async fn download(
    State(s): State<Arc<AppState>>,
    Json(req): Json<DownloadRequest>,
) -> Api<Json<ModelDownload>> {
    Ok(Json(start(
        &s.local_runtime.downloads,
        &s.local_runtime,
        &req.id,
        req.force,
    )?))
}
pub fn start(
    downloads: &Arc<Downloads>,
    runtime: &Runtime,
    id: &str,
    force: bool,
) -> Result<ModelDownload> {
    let spec = causewaybay_shared::local_model_spec(id).context("Unknown on-device model")?;
    ensure!(
        spec.provider == "mlx",
        "Apple's model is part of macOS and needs no download"
    );
    if let Some(d) = downloads.current()
        && d.active()
    {
        if d.id == id {
            return Ok(d);
        }
        bail!("Another model is downloading; wait for it or cancel it first");
    }
    if installed(runtime, spec) {
        bail!("{} is already installed", spec.name);
    }
    let budget = unified_memory_bytes(runtime).map(gpu_budget_bytes);
    if fit(spec.memory_gb, budget) == "too_large" && !force {
        bail!(
            "{} needs {:.1} GB of GPU memory; this Mac can give a model about {:.0} GB",
            spec.name,
            spec.memory_gb,
            budget.unwrap_or(0) as f64 / GIB
        );
    }
    let progress = ModelDownload {
        id: spec.id.into(),
        model: spec.model.into(),
        ..Default::default()
    };
    *downloads.current.lock().unwrap() = Some(progress.clone());
    downloads.cancel.store(false, Ordering::SeqCst);
    downloads.bump();
    let shared = downloads.clone();
    let runtime = runtime.clone();
    let task = tokio::spawn(async move {
        let result = run(&shared, &runtime, spec).await;
        shared.update(|d| {
            d.finished = true;
            d.cancelled = shared.cancel.load(Ordering::SeqCst);
            if let Err(e) = result
                && !d.cancelled
            {
                d.error = Some(e.to_string());
            }
        });
    });
    *downloads.task.lock().unwrap() = Some(task);
    Ok(progress)
}
pub async fn cancel(State(s): State<Arc<AppState>>) -> Api<Json<Value>> {
    let cancelled = s.local_runtime.downloads.cancel();
    s.local_runtime.downloads.bump();
    Ok(Json(serde_json::json!({"cancelled": cancelled})))
}

/// Files the worker reads. Vision towers, READMEs and git metadata stay on the Hub.
fn wanted(path: &str) -> bool {
    path.ends_with(".safetensors")
        || matches!(
            path,
            "config.json"
                | "tokenizer.json"
                | "tokenizer_config.json"
                | "model.safetensors.index.json"
                | "chat_template.jinja"
                | "generation_config.json"
        )
}
fn safe_name(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && !path.contains('/')
        && !path.contains("..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

async fn run(downloads: &Downloads, runtime: &Runtime, spec: &LocalModelSpec) -> Result<()> {
    let base = runtime.hf_base.trim_end_matches('/').to_owned();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(90))
        .user_agent("CausewaybayBot/0.1 (invisible-ai model download)")
        .build()?;
    let listing: Vec<Value> = client
        .get(format!("{base}/api/models/{}/tree/main", spec.model))
        .send()
        .await
        .context("Could not reach Hugging Face")?
        .error_for_status()
        .context("Hugging Face refused the model listing")?
        .json()
        .await
        .context("Unexpected model listing")?;
    let files: Vec<(String, u64)> = listing
        .iter()
        .filter(|f| f["type"] == "file")
        .filter_map(|f| Some((f["path"].as_str()?.to_owned(), f["size"].as_u64()?)))
        .filter(|(p, _)| wanted(p) && safe_name(p))
        .collect();
    ensure!(
        files.iter().any(|(p, _)| p == "config.json")
            && files.iter().any(|(p, _)| p == "tokenizer.json")
            && files.iter().any(|(p, _)| p.ends_with(".safetensors")),
        "The model listing is missing its config, tokenizer or weights"
    );
    let dir = local_dir(&runtime.models_dir, spec.model);
    std::fs::create_dir_all(&dir)?;
    let total: u64 = files.iter().map(|f| f.1).sum();
    let mut done_before: u64 = 0;
    let mut rate = Rate::new();
    downloads.update(|d| {
        d.total_bytes = total;
        d.total_files = files.len();
    });
    for (i, (name, size)) in files.iter().enumerate() {
        ensure!(
            !downloads.cancel.load(Ordering::SeqCst),
            "Download cancelled"
        );
        let target = dir.join(name);
        downloads.update(|d| {
            d.file = name.clone();
            d.done_files = i;
            d.done_bytes = done_before;
        });
        if file_size(&target) == Some(*size) {
            done_before += size;
            continue;
        }
        let part = dir.join(format!("{name}.part"));
        let have = file_size(&part).unwrap_or(0).min(*size);
        let mut request = client.get(format!("{base}/{}/resolve/main/{name}", spec.model));
        if have > 0 {
            request = request.header("range", format!("bytes={have}-"));
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("Could not download {name}"))?;
        let status = response.status();
        ensure!(
            status.is_success(),
            "Hugging Face returned HTTP {} for {name}",
            status.as_u16()
        );
        let resume = status == reqwest::StatusCode::PARTIAL_CONTENT && have > 0;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resume)
            .truncate(!resume)
            .open(&part)
            .await?;
        let mut written = if resume { have } else { 0 };
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            ensure!(
                !downloads.cancel.load(Ordering::SeqCst),
                "Download cancelled"
            );
            let chunk = chunk
                .with_context(|| format!("The connection dropped while downloading {name}"))?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            written += chunk.len() as u64;
            ensure!(written <= *size, "{name} is larger than announced");
            let speed = rate.observe(done_before + written);
            downloads.update(|d| {
                d.done_bytes = done_before + written;
                d.bytes_per_second = speed;
            });
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        drop(file);
        ensure!(written == *size, "{name} ended early; retry to resume");
        tokio::fs::rename(&part, &target).await?;
        done_before += size;
    }
    downloads.update(|d| {
        d.done_files = files.len();
        d.done_bytes = total;
        d.file.clear();
    });
    ensure!(complete(&dir), "The downloaded model is incomplete");
    Ok(())
}
/// Bytes per second over the last second of transfer.
struct Rate {
    mark: Instant,
    bytes: u64,
    speed: u64,
}
impl Rate {
    fn new() -> Self {
        Self {
            mark: Instant::now(),
            bytes: 0,
            speed: 0,
        }
    }
    fn observe(&mut self, total: u64) -> u64 {
        let elapsed = self.mark.elapsed().as_secs_f64();
        if self.bytes == 0 {
            self.bytes = total;
            self.mark = Instant::now();
        } else if elapsed >= 1.0 {
            self.speed = ((total.saturating_sub(self.bytes)) as f64 / elapsed) as u64;
            self.bytes = total;
            self.mark = Instant::now();
        }
        self.speed
    }
}

/// The backend checks its own AI setup and reports to the admin: which mode
/// and provider are the default, whether that runtime is switched on and
/// built, whether its model is installed and fits GPU memory, and, when all
/// of that holds, whether the model actually answers. Run after every save
/// of Backend settings and on request.
pub async fn setup_report(s: &Arc<AppState>) -> SetupReport {
    let mut checks: Vec<SetupCheck> = vec![];
    let mut check = |id: &str, label: String, ok: bool, detail: String| {
        checks.push(SetupCheck {
            id: id.into(),
            label,
            ok,
            detail,
        });
    };
    let local = s.local_ai().unwrap_or_default();
    let default = crate::ai_preferences::default_policy(s).ok().flatten();
    let provider = default
        .as_ref()
        .map(|p| p.provider().to_owned())
        .or_else(|| s.selected_provider().ok())
        .unwrap_or_default();
    let mode = default
        .as_ref()
        .map(|p| p.mode.clone())
        .unwrap_or_else(|| "cloudai".into());
    let mode_name = match mode.as_str() {
        "ondevice" => "On-device",
        "hybrid" => "Hybrid",
        "codex" => "Codex",
        "claude_code" => "Claude Code",
        _ => "Cloud AI",
    };
    check(
        "mode",
        "Default AI mode".into(),
        true,
        format!("{mode_name} · {provider}"),
    );
    let mut runnable = true;
    match provider.as_str() {
        "mlx" => {
            let model =
                s.db.get_setting("model.mlx")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            if !local.mlx_enabled {
                runnable = false;
                check(
                    "enabled",
                    "MLX local enabled".into(),
                    false,
                    "Turn on Enable MLX local and save".into(),
                );
            } else {
                check("enabled", "MLX local enabled".into(), true, String::new());
            }
            if local.mlx_engine == "service" {
                check(
                    "engine",
                    "MLX engine".into(),
                    true,
                    format!("OpenAI-compatible service at {}", local.mlx_url),
                );
            } else {
                let built = s.local_runtime.mlx_worker.is_file();
                runnable &= built;
                check(
                    "worker",
                    "MLX worker built".into(),
                    built,
                    if built {
                        s.local_runtime.mlx_worker.display().to_string()
                    } else {
                        "Run `make mlx-ai` on the backend Mac".into()
                    },
                );
                let spec = causewaybay_shared::local_model_by_repo("mlx", &model);
                let installed = resolve(&s.local_runtime, &model).is_some();
                let download = s.local_runtime.downloads.current();
                let name = spec.map(|m| m.name).unwrap_or(model.as_str());
                let detail = if installed {
                    "installed".to_owned()
                } else if let Some(d) = download.as_ref().filter(|d| d.model == model && d.active())
                {
                    format!("downloading · {:.0}%", d.fraction() * 100.0)
                } else if let Some(e) = download
                    .as_ref()
                    .filter(|d| d.model == model)
                    .and_then(|d| d.error.clone())
                {
                    format!("download failed: {e}")
                } else if spec.is_some() {
                    "not installed · press Download, or ask it a question".into()
                } else {
                    "not installed and not in the catalog".into()
                };
                runnable &= installed;
                check("model", format!("Model {name}"), installed, detail);
                if let Some(spec) = spec {
                    let budget = unified_memory_bytes(&s.local_runtime).map(gpu_budget_bytes);
                    let fit = fit(spec.memory_gb, budget);
                    check(
                        "memory",
                        "GPU memory".into(),
                        fit != "too_large",
                        format!(
                            "needs {:.1} GB · budget {:.0} GB · {}",
                            spec.memory_gb,
                            budget.unwrap_or(0) as f64 / GIB,
                            fit.replace('_', " ")
                        ),
                    );
                    runnable &= fit != "too_large";
                }
            }
        }
        "apple" => {
            runnable = local.apple_enabled && s.local_runtime.apple_worker.is_file();
            check(
                "enabled",
                "Apple local enabled".into(),
                local.apple_enabled,
                if local.apple_enabled {
                    String::new()
                } else {
                    "Turn on Enable Apple local and save".into()
                },
            );
            check(
                "worker",
                "Apple worker built".into(),
                s.local_runtime.apple_worker.is_file(),
                if s.local_runtime.apple_worker.is_file() {
                    String::new()
                } else {
                    "Run `make apple-ai` on the backend Mac".into()
                },
            );
        }
        other => {
            runnable = s.provider_configured(other);
            check(
                "configured",
                format!("{other} configured"),
                runnable,
                if runnable {
                    String::new()
                } else {
                    "Set its API key or sign in its command-line tool on the backend".into()
                },
            );
        }
    }
    if s.demo {
        check(
            "answer",
            "Test answer".into(),
            true,
            "Demo mode · no model call".into(),
        );
    } else if runnable && crate::local_ai::is_local(&provider) {
        let started = Instant::now();
        let result = match s.provider_config_for(&provider) {
            Ok(config) => {
                let messages = [crate::providers::Message::User(
                    "Reply with OK only.".into(),
                )];
                crate::providers::stream(
                    &config,
                    "This is a setup check. Reply OK.",
                    &messages,
                    &[],
                    |_| Ok(()),
                )
                .await
                .map(|t| t.text)
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(text) => check(
                "answer",
                "Test answer".into(),
                true,
                format!(
                    "answered in {:.1} s: {}",
                    started.elapsed().as_secs_f64(),
                    text.trim().chars().take(40).collect::<String>()
                ),
            ),
            Err(e) => check("answer", "Test answer".into(), false, e.to_string()),
        }
    } else if !runnable {
        check(
            "answer",
            "Test answer".into(),
            false,
            "skipped until the checks above pass".into(),
        );
    }
    SetupReport {
        ok: checks.iter().all(|c| c.ok),
        checks,
        checked_at: crate::db::now(),
    }
}
pub async fn setup_check(State(s): State<Arc<AppState>>) -> Api<Json<SetupReport>> {
    Ok(Json(setup_report(&s).await))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_budget_follows_metal_working_set_rule() {
        let gib = GIB as u64;
        assert_eq!(gpu_budget_bytes(16 * gib), 16 * gib / 3 * 2);
        assert_eq!(gpu_budget_bytes(36 * gib), 36 * gib / 3 * 2);
        assert_eq!(gpu_budget_bytes(128 * gib), 128 * gib / 4 * 3);
    }
    #[test]
    fn fit_classifies_against_the_budget() {
        let budget = Some(gpu_budget_bytes(36 * GIB as u64)); // 24 GB
        assert_eq!(fit(0.0, budget), "fits");
        assert_eq!(fit(5.2, budget), "fits");
        assert_eq!(fit(15.4, budget), "fits");
        assert_eq!(fit(21.0, budget), "tight");
        assert_eq!(fit(25.0, budget), "too_large");
        let eight = Some(gpu_budget_bytes(8 * GIB as u64)); // 5.3 GB
        assert_eq!(fit(2.5, eight), "fits");
        assert_eq!(fit(5.2, eight), "tight");
        assert_eq!(fit(15.4, eight), "too_large");
        assert_eq!(fit(15.4, None), "unknown");
    }
    #[test]
    fn only_model_files_with_plain_names_are_fetched() {
        for ok in [
            "config.json",
            "tokenizer.json",
            "model-00001-of-00003.safetensors",
            "model.safetensors.index.json",
        ] {
            assert!(wanted(ok) && safe_name(ok), "{ok}");
        }
        for no in [
            "README.md",
            ".gitattributes",
            "preprocessor_config.json",
            "video_preprocessor_config.json",
        ] {
            assert!(!wanted(no), "{no}");
        }
        for bad in [
            "../x.safetensors",
            "a/b.safetensors",
            "",
            "we ird.safetensors",
        ] {
            assert!(!safe_name(bad), "{bad}");
        }
    }
}
