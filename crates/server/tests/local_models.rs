//! On-device model catalog, GPU-memory fit, explicit downloads from a loopback
//! stand-in for Hugging Face, and the built-in Rust MLX worker path. No real
//! model is downloaded or run except in the ignored live test.
use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use causewaybay_server::{
    AppState, app,
    credentials::Credentials,
    local_models, preferences,
    providers::{self, Config, Message},
};
use causewaybay_shared::{LOCAL_MODELS, LocalAiSettings, LocalModels, SettingsUpdate};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

const GIB: u64 = 1024 * 1024 * 1024;
const REPO: &str = "mlx-community/Qwen3.5-4B-4bit";

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
async fn models(s: &Arc<AppState>) -> LocalModels {
    let (status, body) = call(s, "GET", "/api/local-ai/models", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}
async fn wait_for(
    s: &Arc<AppState>,
    what: &str,
    mut done: impl FnMut(&LocalModels) -> bool,
) -> LocalModels {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let m = models(s).await;
            if done(&m) {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}
#[cfg(unix)]
fn script(path: &Path, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/usr/bin/env python3\n{source}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// A stand-in for `invisible-mlx --serve`: one lane, requests answered in
/// order, the "model" kept loaded between them, cancel lines honoured between
/// tokens. It logs every spawn, request and cancel so tests can check that the
/// backend started it once and sent what it should.
struct FakeWorker {
    path: std::path::PathBuf,
    log: std::path::PathBuf,
}
impl FakeWorker {
    fn lines(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
    fn spawns(&self) -> usize {
        self.lines()
            .iter()
            .filter(|l| l["spawn"].is_number())
            .count()
    }
    fn requests(&self) -> Vec<Value> {
        self.lines()
            .into_iter()
            .filter(|l| l["operation"] == "generate")
            .collect()
    }
    fn cancels(&self) -> usize {
        self.lines()
            .iter()
            .filter(|l| l["cancel"].is_string())
            .count()
    }
}
#[cfg(unix)]
fn serve_worker(dir: &Path, name: &str) -> FakeWorker {
    let path = dir.join(name);
    let log = dir.join(format!("{name}.log"));
    script(
        &path,
        &format!(
            r#"import sys,json,os,time,threading,queue
LOG={log:?}
def record(v):
    with open(LOG,'a') as f: f.write(json.dumps(v)+"\n")
def emit(v):
    sys.stdout.write(json.dumps(v)+"\n"); sys.stdout.flush()
assert sys.argv[1:]==['--serve'], sys.argv
assert os.environ['HF_HUB_OFFLINE']=='1'
record({{'spawn':os.getpid()}})
q=queue.Queue()
def reader():
    for line in sys.stdin:
        if line.strip(): q.put(json.loads(line))
    q.put(None)
threading.Thread(target=reader,daemon=True).start()
loaded=None
cancelled=set()
pending=[]
def cancelled_now(rid):
    while True:
        try: item=q.get_nowait()
        except queue.Empty: break
        if item is None: os._exit(0)
        if 'cancel' in item: record(item); cancelled.add(item['cancel'])
        else: pending.append(item)
    return rid in cancelled
while True:
    r=pending.pop(0) if pending else q.get()
    if r is None: break
    if 'cancel' in r: record(r); cancelled.add(r['cancel']); continue
    rid=r.get('id')
    record(r)
    if r['operation']=='ping': emit({{'id':rid,'pong':True,'loaded':loaded}}); continue
    if r['operation']=='unload': loaded=None; emit({{'id':rid,'done':True,'unloaded':True}}); continue
    text=(r.get('messages') or [{{'content':r.get('prompt','')}}])[-1]['content']
    if text=='crash': os._exit(1)
    if not os.path.isfile(os.path.join(r['model_dir'],'config.json')):
        emit({{'id':rid,'error':'mlx_model_missing'}}); continue
    if loaded!=r['model_dir']:
        emit({{'id':rid,'stage':'loading','model':r['model_dir']}}); time.sleep(0.05); loaded=r['model_dir']
        emit({{'id':rid,'stage':'loaded','load_ms':50,'cached':False}})
    else:
        emit({{'id':rid,'stage':'loaded','load_ms':0,'cached':True}})
    if text.startswith('slow'):
        n=int(text.split()[1]); time.sleep(0.6)
        done=0
        for i in range(n):
            if cancelled_now(rid):
                emit({{'id':rid,'done':True,'cancelled':True,'input_tokens':5,'output_tokens':done}}); break
            emit({{'id':rid,'delta':'tok%d '%i}}); done+=1; time.sleep(0.1)
        else:
            emit({{'id':rid,'done':True,'cancelled':False,'input_tokens':5,'output_tokens':done}})
        continue
    emit({{'id':rid,'delta':'1'}}); emit({{'id':rid,'delta':'2'}})
    emit({{'id':rid,'done':True,'cancelled':False,'input_tokens':41,'output_tokens':3,'peak_memory_bytes':2500000000}})
"#
        ),
    );
    FakeWorker { path, log }
}

fn state(dir: &Path, memory: u64, hub: &str) -> Arc<AppState> {
    let mut s =
        AppState::open_with_credentials(dir.into(), false, None, Credentials::default()).unwrap();
    let runtime = &mut Arc::get_mut(&mut s).unwrap().local_runtime;
    runtime.hf_base = hub.into();
    runtime.hf_cache = dir.join("empty-hf-cache");
    runtime.unified_memory = Some(memory);
    runtime.mlx_worker = dir.join("no-worker");
    runtime.apple_worker = dir.join("no-apple-worker");
    s
}

/// A loopback Hugging Face: a tree listing with sizes and files served with
/// Range support, plus a gate that stalls the weights file so a test can watch
/// progress mid-flight and cancel. Every Range header received is recorded.
#[derive(Clone)]
struct Hub {
    files: Arc<Vec<(String, Vec<u8>)>>,
    ranges: Arc<Mutex<Vec<String>>>,
    gate: Arc<tokio::sync::Notify>,
    gated: Arc<std::sync::atomic::AtomicBool>,
}
async fn tree(State(h): State<Hub>, AxumPath((org, name)): AxumPath<(String, String)>) -> Response {
    if format!("{org}/{name}") != REPO {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut rows: Vec<Value> = h
        .files
        .iter()
        .map(|(p, b)| json!({"type":"file","path":p,"size":b.len()}))
        .collect();
    rows.push(json!({"type":"directory","path":"docs","size":0}));
    axum::Json(rows).into_response()
}
async fn resolve(
    State(h): State<Hub>,
    AxumPath((org, name, file)): AxumPath<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    if format!("{org}/{name}") != REPO {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some((_, bytes)) = h.files.iter().find(|(p, _)| *p == file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    if let Some(r) = &range {
        h.ranges.lock().unwrap().push(format!("{file} {r}"));
    }
    let start: usize = range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes=")?.strip_suffix('-')?.parse().ok())
        .unwrap_or(0);
    let body = bytes[start.min(bytes.len())..].to_vec();
    let status = if start > 0 {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    if file.ends_with(".safetensors") && h.gated.load(std::sync::atomic::Ordering::SeqCst) {
        // First half now, the rest once the test opens the gate.
        let (head, tail) = body.split_at(body.len() / 2);
        let (head, tail) = (head.to_vec(), tail.to_vec());
        let gate = h.gate.clone();
        let stream = async_stream::stream! {
            yield Ok::<_, std::io::Error>(axum::body::Bytes::from(head));
            gate.notified().await;
            yield Ok(axum::body::Bytes::from(tail));
        };
        return (status, Body::from_stream(stream)).into_response();
    }
    (status, body).into_response()
}
async fn hub(gated: bool) -> (String, Hub) {
    let weights: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let files = vec![
        (
            "config.json".to_string(),
            br#"{"model_type":"qwen3_5"}"#.to_vec(),
        ),
        (
            "tokenizer.json".to_string(),
            b"{\"version\":\"1.0\"}".to_vec(),
        ),
        ("tokenizer_config.json".to_string(), b"{}".to_vec()),
        (
            "model.safetensors.index.json".to_string(),
            br#"{"weight_map":{"a":"model.safetensors","b":"model.safetensors"}}"#.to_vec(),
        ),
        ("model.safetensors".to_string(), weights),
        ("README.md".to_string(), b"not needed".to_vec()),
        ("preprocessor_config.json".to_string(), b"vision".to_vec()),
    ];
    let h = Hub {
        files: Arc::new(files),
        ranges: Arc::new(Mutex::new(vec![])),
        gate: Arc::new(tokio::sync::Notify::new()),
        gated: Arc::new(std::sync::atomic::AtomicBool::new(gated)),
    };
    let router = Router::new()
        .route("/api/models/{org}/{name}/tree/main", get(tree))
        .route("/{org}/{name}/resolve/main/{file}", get(resolve))
        .with_state(h.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, h)
}

#[cfg(unix)]
#[tokio::test]
async fn catalog_reports_gpu_memory_fit_and_install_state() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = state(dir.path(), 8 * GIB, "http://127.0.0.1:9");
    let m = models(&s).await;
    assert_eq!(m.unified_memory_bytes, Some(8 * GIB));
    assert_eq!(m.gpu_budget_bytes, Some(8 * GIB / 3 * 2));
    assert!(!m.worker_available && !m.apple_available);
    assert_eq!(
        m.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        LOCAL_MODELS.iter().map(|m| m.id).collect::<Vec<_>>()
    );
    let by_id = |id: &str| m.models.iter().find(|m| m.id == id).unwrap();
    assert!(by_id("apple").installed && by_id("apple").fit == "fits");
    assert!(!by_id("qwen3.5-4b").installed && by_id("qwen3.5-4b").fit == "fits");
    assert_eq!(by_id("qwen3.5-9b").fit, "tight");
    assert_eq!(by_id("qwen3.8-27b").fit, "too_large");
    assert!(m.models.iter().all(|m| m.download.is_none()));

    let everyday = state(dir.path(), 16 * GIB, "http://127.0.0.1:9");
    let everyday_models = models(&everyday).await;
    assert_eq!(
        everyday_models
            .models
            .iter()
            .find(|m| m.id == "qwen3.5-9b")
            .unwrap()
            .fit,
        "fits"
    );
    assert_eq!(
        everyday_models
            .models
            .iter()
            .find(|m| m.id == "qwen3.8-27b")
            .unwrap()
            .fit,
        "too_large"
    );

    // A plain 36 GB Mac fits everything the catalog offers.
    let big = state(dir.path(), 36 * GIB, "http://127.0.0.1:9");
    assert!(models(&big).await.models.iter().all(|m| m.fit == "fits"));

    // The Hugging Face cache layout (snapshots of symlinks to blobs) counts as installed,
    // but only when the weights are really there.
    let cache = dir.path().join("empty-hf-cache");
    let base = cache.join("models--mlx-community--Qwen3.5-4B-4bit");
    let snap = base.join("snapshots/abc");
    std::fs::create_dir_all(&snap)?;
    std::fs::create_dir_all(base.join("blobs"))?;
    std::fs::create_dir_all(base.join("refs"))?;
    std::fs::write(base.join("refs/main"), "abc\n")?;
    for (name, blob, content) in [
        ("config.json", "c1", "{}"),
        ("tokenizer.json", "t1", "{}"),
        (
            "model.safetensors.index.json",
            "i1",
            r#"{"weight_map":{"w":"model.safetensors"}}"#,
        ),
    ] {
        std::fs::write(base.join("blobs").join(blob), content)?;
        std::os::unix::fs::symlink(format!("../../blobs/{blob}"), snap.join(name))?;
    }
    std::os::unix::fs::symlink("../../blobs/missing", snap.join("model.safetensors"))?;
    assert!(
        !models(&s).await.models[2].installed,
        "dangling weights are not installed"
    );
    std::fs::write(base.join("blobs/missing"), "weights")?;
    assert!(models(&s).await.models[2].installed);
    assert_eq!(local_models::resolve(&s.local_runtime, REPO).unwrap(), snap);
    Ok(())
}

#[tokio::test]
async fn download_refuses_apple_unknown_and_too_large_models_unless_forced() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (base, _hub) = hub(false).await;
    let s = state(dir.path(), 8 * GIB, &base);
    for (body, text) in [
        (json!({"id":"apple"}), "no download"),
        (json!({"id":"llama"}), "Unknown"),
        (json!({"id":"qwen3.8-27b"}), "needs 15.4 GB"),
    ] {
        let (status, err) = call(&s, "POST", "/api/admin/local-ai/models/fetch", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
        assert!(err["error"].as_str().unwrap().contains(text), "{err}");
    }
    // Forced: the download starts, and this hub has no such repo, so it fails cleanly.
    let (status, _) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.8-27b","force":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let m = wait_for(&s, "the forced download to fail", |m| {
        m.models[3].download.as_ref().is_some_and(|d| d.finished)
    })
    .await;
    let d = m.models[3].download.clone().unwrap();
    assert!(
        d.error.as_deref().unwrap_or("").contains("refused"),
        "{d:?}"
    );
    assert!(!m.models[3].installed);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn download_shows_progress_cancels_resumes_with_range_and_then_runs_the_worker()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (base, hub) = hub(true).await;
    let s = state(dir.path(), 36 * GIB, &base);
    let (status, started) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.5-4b"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["model"], REPO);
    // Starting it again while it runs is idempotent; starting another is refused.
    let (status, again) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.5-4b"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    let (status, other) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.5-9b"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(other["error"].as_str().unwrap().contains("Another model"));

    let total: u64 = hub
        .files
        .iter()
        .filter(|(p, _)| p != "README.md" && p != "preprocessor_config.json")
        .map(|(_, b)| b.len() as u64)
        .sum();
    let m = wait_for(&s, "half the weights", |m| {
        m.models[2]
            .download
            .as_ref()
            .is_some_and(|d| d.file == "model.safetensors" && d.done_bytes > d.total_bytes / 2)
    })
    .await;
    let d = m.models[2].download.clone().unwrap();
    assert!(d.active() && !m.models[2].installed);
    assert_eq!(
        d.total_bytes, total,
        "README and vision files are not counted"
    );
    assert_eq!(d.total_files, 5);
    assert_eq!(d.done_files, 4);
    assert!(d.done_bytes < d.total_bytes);
    assert!(d.fraction() > 0.5 && d.fraction() < 1.0);

    // Cancel mid-file: the partial weights stay for a resume, nothing is installed.
    let (status, cancelled) = call(
        &s,
        "DELETE",
        "/api/admin/local-ai/models/fetch",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["cancelled"], true);
    hub.gate.notify_waiters();
    hub.gate.notify_one();
    let m = wait_for(&s, "the cancel", |m| {
        m.models[2].download.as_ref().is_some_and(|d| d.finished)
    })
    .await;
    let d = m.models[2].download.clone().unwrap();
    assert!(d.cancelled && d.error.is_none() && !d.active(), "{d:?}");
    assert!(!m.models[2].installed);
    let local = local_models::local_dir(&s.local_runtime.models_dir, REPO);
    let part = local.join("model.safetensors.part");
    let kept = std::fs::metadata(&part)?.len();
    assert!(kept > 0 && kept < 300_000, "{kept}");
    assert!(local.join("config.json").is_file());
    assert!(!local.join("README.md").exists());
    assert_eq!(local_models::resolve(&s.local_runtime, REPO), None);

    // Resume: finished files are skipped, the weights continue from the kept bytes.
    hub.gated.store(false, std::sync::atomic::Ordering::SeqCst);
    let (status, _) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.5-4b"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let m = wait_for(&s, "the install", |m| m.models[2].installed).await;
    let d = m.models[2].download.clone().unwrap();
    assert!(d.finished && d.error.is_none() && !d.cancelled);
    assert_eq!((d.done_bytes, d.done_files), (total, 5));
    assert_eq!(d.fraction(), 1.0);
    assert_eq!(
        hub.ranges.lock().unwrap().as_slice(),
        [format!("model.safetensors bytes={kept}-")]
    );
    assert!(!part.exists());
    let weights = std::fs::read(local.join("model.safetensors"))?;
    assert_eq!(
        weights,
        hub.files
            .iter()
            .find(|(p, _)| p == "model.safetensors")
            .unwrap()
            .1
    );
    assert_eq!(
        local_models::resolve(&s.local_runtime, REPO).unwrap(),
        local
    );
    let (status, err) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.5-4b"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("already installed"));

    // The built-in engine hands the worker that directory, never a repo id, and
    // reads the token counts it reports.
    let worker = serve_worker(dir.path(), "mlx-worker");
    // The router holds clones of `s`, so the worker goes on a fresh state over the same data.
    let mut s2 = state(dir.path(), 36 * GIB, &base);
    Arc::get_mut(&mut s2).unwrap().local_runtime.mlx_worker = worker.path.clone();
    assert!(models(&s2).await.worker_available);
    let local_settings = LocalAiSettings {
        mlx_enabled: true,
        mlx_engine: "builtin".into(),
        ..Default::default()
    };
    preferences::update(
        &s2,
        SettingsUpdate {
            provider: Some("mlx".into()),
            model: Some(REPO.into()),
            local_ai: Some(local_settings.clone()),
            ..Default::default()
        },
    )?;
    assert!(s2.provider_configured("mlx"));
    let config = s2.provider_config()?;
    assert_eq!(
        (config.provider.as_str(), config.model.as_str()),
        ("mlx", REPO)
    );
    let messages = [
        Message::User("What is 2+2?".into()),
        Message::Assistant {
            text: "4".into(),
            calls: vec![],
        },
        Message::User("And 7+5?".into()),
    ];
    let mut deltas = vec![];
    let turn = providers::stream(&config, "Be brief.", &messages, &[], |v| {
        deltas.push(v.to_string());
        Ok(())
    })
    .await?;
    assert_eq!(deltas, ["1", "2"]);
    assert_eq!(turn.text, "12");
    assert_eq!((turn.input_tokens, turn.output_tokens), (Some(41), Some(3)));
    let seen = worker.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["model_dir"], local.to_str().unwrap());
    assert_eq!(seen[0]["system"], "Be brief.");
    assert_eq!(
        seen[0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["user", "assistant", "user"]
    );
    assert_eq!(seen[0]["messages"][2]["content"], "And 7+5?");
    assert_eq!(
        s2.local_runtime.mlx.status().loaded_model.as_deref(),
        Some("Qwen3.5 4B")
    );

    // A person's request for a catalog model that is not installed downloads it;
    // when that fails (this hub has no 9B) the request fails plainly, with no
    // cloud fallback. A background job without progress never starts one.
    let mut not_installed = config.clone();
    not_installed.model = "mlx-community/Qwen3.5-9B-4bit".into();
    let err = providers::stream(&not_installed, "x", &messages[..1], &[], |_| Ok(()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not installed"), "{err}");
    let (_, sink) = progress_log();
    not_installed.progress = Some(sink(0));
    let err = providers::stream(&not_installed, "x", &messages[..1], &[], |_| Ok(()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Downloading Qwen3.5 9B failed"), "{err}");
    assert!(
        !local_models::local_dir(
            &s2.local_runtime.models_dir,
            "mlx-community/Qwen3.5-9B-4bit"
        )
        .exists()
    );

    // Without the worker binary MLX local is not configured and says so.
    let mut no_worker = config.clone();
    no_worker.runtime.mlx_worker = dir.path().join("missing");
    let err = providers::stream(&no_worker, "x", &messages[..1], &[], |_| Ok(()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("make mlx-ai"), "{err}");

    // Images never reach the worker.
    let err = providers::stream(
        &config,
        "x",
        &[Message::Image {
            text: "t".into(),
            images: vec![("image/png".into(), "AAAA".into())],
        }],
        &[],
        |_| Ok(()),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("OCR text"), "{err}");
    Ok(())
}

/// An installed fake model directory (config + tokenizer + weights).
fn fake_installed(models_dir: &Path, repo: &str) -> std::path::PathBuf {
    let dir = local_models::local_dir(models_dir, repo);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, body) in [
        ("config.json", "{}"),
        ("tokenizer.json", "{}"),
        ("model.safetensors", "weights"),
    ] {
        std::fs::write(dir.join(name), body).unwrap();
    }
    dir
}
fn mlx_config(s: &Arc<AppState>, worker: &FakeWorker) -> Config {
    let mut runtime = s.local_runtime.clone();
    runtime.mlx_worker = worker.path.clone();
    Config {
        owner: "local".into(),
        provider: "mlx".into(),
        model: REPO.into(),
        local: LocalAiSettings {
            mlx_enabled: true,
            mlx_engine: "builtin".into(),
            ..Default::default()
        },
        runtime,
        key: String::new(),
        progress: None,
        max_tokens: None,
    }
}
/// Collects the progress a job reports, labelled by job.
type ProgressLog = Arc<Mutex<Vec<(usize, String, String, Option<f64>)>>>;
fn progress_log() -> (
    ProgressLog,
    impl Fn(usize) -> causewaybay_server::providers::ProgressSink,
) {
    let log = Arc::new(Mutex::new(vec![]));
    let make = {
        let log = log.clone();
        move |job: usize| -> causewaybay_server::providers::ProgressSink {
            let log = log.clone();
            Arc::new(move |p: causewaybay_shared::ModelProgress| {
                log.lock()
                    .unwrap()
                    .push((job, p.label, p.detail, p.fraction));
            })
        }
    };
    (log, make)
}

#[cfg(unix)]
#[tokio::test]
async fn manager_serves_users_one_at_a_time_and_keeps_the_model_loaded() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = state(dir.path(), 36 * GIB, "http://127.0.0.1:9");
    let worker = serve_worker(dir.path(), "serve-worker");
    fake_installed(&s.local_runtime.models_dir, REPO);
    let (log, sink) = progress_log();
    let mut jobs = tokio::task::JoinSet::new();
    for job in 0..3 {
        let mut config = mlx_config(&s, &worker);
        config.progress = Some(sink(job));
        jobs.spawn(async move {
            let started = std::time::Instant::now();
            let mut first = None;
            let turn =
                providers::stream(&config, "x", &[Message::User("slow 3".into())], &[], |_| {
                    first.get_or_insert(started.elapsed());
                    Ok(())
                })
                .await
                .unwrap();
            (job, turn.text, first.unwrap())
        });
        // Arrival order is the service order.
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let mut results = vec![];
    while let Some(r) = jobs.join_next().await {
        results.push(r?);
    }
    results.sort_by_key(|r| r.0);
    for (_, text, _) in &results {
        assert_eq!(text, "tok0 tok1 tok2 ");
    }
    // One worker process, one load; the second and third waited their turn.
    assert_eq!(worker.spawns(), 1);
    assert_eq!(worker.requests().len(), 3);
    let log = log.lock().unwrap().clone();
    let loads = log.iter().filter(|l| l.1.starts_with("Loading")).count();
    assert_eq!(loads, 1, "{log:?}");
    assert!(
        log.iter()
            .any(|l| l.0 == 0 && l.1 == "Qwen3.5 4B ready" && l.2.starts_with("loaded in")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l.0 == 1
            && l.1 == "Waiting for the on-device model"
            && l.2 == "1 request ahead"),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l.0 == 2
            && l.1 == "Waiting for the on-device model"
            && l.2 == "2 requests ahead"),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l.0 == 2 && l.2 == "already in memory"),
        "{log:?}"
    );
    assert!(
        results[1].2 > results[0].2 && results[2].2 > results[1].2,
        "{results:?}"
    );
    let status = s.local_runtime.mlx.status();
    assert_eq!(status.loaded_model.as_deref(), Some("Qwen3.5 4B"));
    assert!(!status.busy && status.waiting == 0);
    let m = models(&s).await;
    assert_eq!(m.loaded_model.as_deref(), Some("Qwen3.5 4B"));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn an_abandoned_request_is_cancelled_in_place_and_a_dead_worker_is_restarted()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = state(dir.path(), 36 * GIB, "http://127.0.0.1:9");
    let worker = serve_worker(dir.path(), "serve-worker");
    fake_installed(&s.local_runtime.models_dir, REPO);
    let config = mlx_config(&s, &worker);

    // The user stops a long answer: the manager cancels it in the worker and the
    // model stays loaded for the next request.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let long = tokio::spawn({
        let config = config.clone();
        async move {
            providers::stream(&config, "x", &[Message::User("slow 50".into())], &[], |t| {
                let _ = tx.send(t.to_string());
                Ok(())
            })
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await?
        .unwrap();
    long.abort();
    let started = std::time::Instant::now();
    let turn = providers::stream(&config, "x", &[Message::User("next".into())], &[], |_| {
        Ok(())
    })
    .await?;
    assert_eq!(turn.text, "12");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the lane freed promptly: {:?}",
        started.elapsed()
    );
    assert_eq!(worker.cancels(), 1);
    assert_eq!(worker.spawns(), 1, "cancel never restarts the worker");
    assert_eq!(
        s.local_runtime.mlx.status().loaded_model.as_deref(),
        Some("Qwen3.5 4B")
    );

    // The worker crashes: that request fails plainly, the next one restarts it.
    let err = providers::stream(&config, "x", &[Message::User("crash".into())], &[], |_| {
        Ok(())
    })
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("Local AI worker failed"), "{err}");
    assert_eq!(s.local_runtime.mlx.status().loaded_model, None);
    let (log, sink) = progress_log();
    let mut config = config.clone();
    config.progress = Some(sink(0));
    let turn = providers::stream(&config, "x", &[Message::User("after".into())], &[], |_| {
        Ok(())
    })
    .await?;
    assert_eq!(turn.text, "12");
    assert_eq!(worker.spawns(), 2);
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .any(|l| l.1.starts_with("Loading"))
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn a_users_request_downloads_the_chosen_model_and_shows_progress_in_the_chat()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (base, _hub) = hub(false).await;
    let mut s = state(dir.path(), 36 * GIB, &base);
    let worker = serve_worker(dir.path(), "serve-worker");
    Arc::get_mut(&mut s).unwrap().local_runtime.mlx_worker = worker.path.clone();
    preferences::update(
        &s,
        SettingsUpdate {
            provider: Some("mlx".into()),
            model: Some(REPO.into()),
            local_ai: Some(LocalAiSettings {
                mlx_enabled: true,
                mlx_engine: "builtin".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    assert!(!models(&s).await.models[2].installed);
    let (status, room) = call(&s, "POST", "/api/rooms", json!({"title":"On-device"})).await;
    assert_eq!(status, StatusCode::OK, "{room}");
    let room = room["roomid"].as_str().unwrap().to_owned();
    let message = uuid::Uuid::new_v4().to_string();
    let (status, memo) = call(
        &s,
        "POST",
        "/api/think",
        json!({"room_id":room,"message_id":message,"provider":"mlx","prompt":"slow 4","input_mode":"text"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    let id = memo["id"].as_str().unwrap().to_owned();

    // While the model loads, the chat's assistant message carries the progress.
    let seen = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let sync = causewaybay_server::chat::changes(&s.db, &room, 0).unwrap();
            if let Some(p) = sync
                .messages
                .iter()
                .find(|m| m.role == "assistant")
                .and_then(|m| m.progress.clone())
            {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("progress reached the chat message");
    assert!(
        seen.label.starts_with("Downloading")
            || seen.label.starts_with("Loading")
            || seen.label.ends_with("ready"),
        "{seen:?}"
    );

    let done = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let m = s.db.get(&id).unwrap().unwrap();
            if m.status != "running" {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert_eq!(done.status, "complete", "{:?}", done.error);
    assert_eq!(done.answer, "tok0 tok1 tok2 tok3 ");
    assert!(
        models(&s).await.models[2].installed,
        "the request installed the model"
    );
    let events = s.db.events(&id, 0)?;
    let progress: Vec<causewaybay_shared::ModelProgress> = events
        .iter()
        .filter(|e| e.kind == "progress")
        .map(|e| serde_json::from_str(&e.text).unwrap())
        .collect();
    assert!(
        progress
            .iter()
            .any(|p| p.label == "Downloading Qwen3.5 4B" && p.fraction.is_some()),
        "{progress:?}"
    );
    assert!(
        progress
            .iter()
            .any(|p| p.label == "Loading Qwen3.5 4B into memory" && p.fraction.is_none()),
        "{progress:?}"
    );
    assert!(
        progress.iter().any(|p| p.label == "Qwen3.5 4B ready"),
        "{progress:?}"
    );
    // Throttled: a 300 KB download does not write hundreds of rows.
    assert!(progress.len() < 40, "{}", progress.len());
    let sync = causewaybay_server::chat::changes(&s.db, &room, 0)?;
    let reply = sync
        .messages
        .iter()
        .find(|m| m.role == "assistant")
        .unwrap();
    assert_eq!(reply.status, "complete");
    assert_eq!(
        reply.progress, None,
        "progress clears once the answer arrives"
    );
    assert_eq!(reply.text, "tok0 tok1 tok2 tok3 ");
    assert_eq!(worker.spawns(), 1);
    Ok(())
}

#[tokio::test]
async fn the_models_stream_pushes_snapshots_as_a_download_moves() -> anyhow::Result<()> {
    use futures_util::StreamExt;
    let dir = tempfile::tempdir()?;
    let (base, hub) = hub(true).await;
    let s = state(dir.path(), 36 * GIB, &base);
    let response = app(s.clone(), "dist".into())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/local-ai/models/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()?
            .starts_with("text/event-stream")
    );
    let mut body = response.into_body().into_data_stream();
    async fn frame(
        body: &mut (impl futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
    ) -> LocalModels {
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(10), body.next())
                .await
                .expect("a frame in time")
                .unwrap()
                .unwrap();
            let text = String::from_utf8_lossy(&chunk).to_string();
            if let Some(data) = text.lines().find_map(|l| l.strip_prefix("data:")) {
                return serde_json::from_str::<LocalModels>(data.trim()).unwrap();
            }
        }
    }
    let first = frame(&mut body).await;
    assert!(!first.models[2].installed && first.models[2].download.is_none());
    let (status, _) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.5-4b"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut snapshots = vec![];
    let mut stamps = vec![];
    loop {
        let m = frame(&mut body).await;
        stamps.push(std::time::Instant::now());
        if let Some(d) = &m.models[2].download {
            snapshots.push(d.done_bytes);
            if d.file == "model.safetensors" && d.done_bytes > d.total_bytes / 2 {
                break;
            }
        }
    }
    assert!(snapshots.windows(2).all(|w| w[0] <= w[1]), "{snapshots:?}");
    // Snapshots are coalesced so the browser's eased bar finishes each move.
    let gaps: Vec<Duration> = stamps.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        gaps.iter().all(|g| *g >= Duration::from_millis(500)),
        "{gaps:?}"
    );
    hub.gated.store(false, std::sync::atomic::Ordering::SeqCst);
    hub.gate.notify_waiters();
    hub.gate.notify_one();
    loop {
        let m = frame(&mut body).await;
        if m.models[2].installed {
            break;
        }
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn an_idle_worker_is_stopped_and_background_jobs_never_start_downloads() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let (base, _hub) = hub(false).await;
    let mut s = state(dir.path(), 36 * GIB, &base);
    let worker = serve_worker(dir.path(), "serve-worker");
    {
        let runtime = &mut Arc::get_mut(&mut s).unwrap().local_runtime;
        runtime.mlx_worker = worker.path.clone();
        runtime.mlx = Arc::new(causewaybay_server::mlx_manager::Manager::new(
            Duration::from_millis(300),
        ));
    }
    // Without a progress sink (a Wiki tag job, a note check) nothing is fetched.
    let config = mlx_config(&s, &worker);
    let err = providers::stream(&config, "x", &[Message::User("hi".into())], &[], |_| Ok(()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not installed"), "{err}");
    assert!(
        s.local_runtime.downloads.current().is_none(),
        "no download started"
    );
    assert_eq!(worker.spawns(), 0);

    fake_installed(&s.local_runtime.models_dir, REPO);
    let (log, sink) = progress_log();
    let mut config = mlx_config(&s, &worker);
    config.progress = Some(sink(0));
    providers::stream(
        &config,
        "x",
        &[Message::User("one".into())],
        &[],
        |_| Ok(()),
    )
    .await?;
    assert_eq!(
        s.local_runtime.mlx.status().loaded_model.as_deref(),
        Some("Qwen3.5 4B")
    );
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        s.local_runtime.mlx.status().loaded_model,
        None,
        "idle worker stopped"
    );
    providers::stream(
        &config,
        "x",
        &[Message::User("two".into())],
        &[],
        |_| Ok(()),
    )
    .await?;
    assert_eq!(worker.spawns(), 2);
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|l| l.1.starts_with("Loading"))
            .count(),
        2
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn the_backend_checks_its_own_setup_and_reports_each_problem() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut s = state(dir.path(), 8 * GIB, "http://127.0.0.1:9");
    let worker = serve_worker(dir.path(), "serve-worker");
    async fn report(s: &Arc<AppState>) -> causewaybay_shared::SetupReport {
        let (status, body) = call(s, "GET", "/api/admin/setup/check", Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_value(body).unwrap()
    }
    let by = |r: &causewaybay_shared::SetupReport, id: &str| {
        r.checks
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} missing in {r:?}"))
    };
    // MLX chosen but switched off, no worker, nothing installed, and the 27B would not fit.
    preferences::update(
        &s,
        SettingsUpdate {
            provider: Some("mlx".into()),
            model: Some("mlx-community/Qwen3.8-27B-4bit".into()),
            default_ai: Some(causewaybay_shared::UserAiPreferences {
                mode: "ondevice".into(),
                local_provider: "mlx".into(),
                cloud_provider: "grok".into(),
            }),
            local_ai: Some(LocalAiSettings::default()),
            ..Default::default()
        },
    )?;
    let r = report(&s).await;
    assert!(!r.ok);
    assert_eq!(by(&r, "mode").detail, "On-device · mlx");
    assert!(!by(&r, "enabled").ok && by(&r, "enabled").detail.contains("Enable MLX local"));
    assert!(!by(&r, "worker").ok && by(&r, "worker").detail.contains("make mlx-ai"));
    assert!(!by(&r, "model").ok && by(&r, "model").label == "Model Qwen3.8 27B");
    assert!(!by(&r, "memory").ok && by(&r, "memory").detail.contains("too large"));
    assert!(!by(&r, "answer").ok && by(&r, "answer").detail.contains("skipped"));
    assert_eq!(r.problems(), 5);

    // Everything in place: the backend asks the model and reports its answer time.
    Arc::get_mut(&mut s).unwrap().local_runtime.mlx_worker = worker.path.clone();
    fake_installed(&s.local_runtime.models_dir, REPO);
    preferences::update(
        &s,
        SettingsUpdate {
            provider: Some("mlx".into()),
            model: Some(REPO.into()),
            local_ai: Some(LocalAiSettings {
                mlx_enabled: true,
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let r = report(&s).await;
    assert!(r.ok, "{r:?}");
    assert_eq!(by(&r, "model").detail, "installed");
    assert!(by(&r, "memory").detail.contains("fits"));
    assert!(by(&r, "answer").detail.starts_with("answered in"), "{r:?}");
    assert!(by(&r, "answer").detail.ends_with(": 12"), "{r:?}");
    assert_eq!(worker.spawns(), 1);

    // Apple local chosen without its worker.
    preferences::update(
        &s,
        SettingsUpdate {
            provider: Some("apple".into()),
            default_ai: Some(causewaybay_shared::UserAiPreferences {
                mode: "ondevice".into(),
                local_provider: "apple".into(),
                cloud_provider: "grok".into(),
            }),
            local_ai: Some(LocalAiSettings {
                apple_enabled: true,
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let r = report(&s).await;
    assert!(by(&r, "enabled").ok);
    assert!(!by(&r, "worker").ok && by(&r, "worker").detail.contains("make apple-ai"));
    Ok(())
}

#[tokio::test]
async fn service_engine_still_uses_the_loopback_url() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = state(dir.path(), 36 * GIB, "http://127.0.0.1:9");
    let config = Config {
        owner: "local".into(),
        provider: "mlx".into(),
        model: REPO.into(),
        local: LocalAiSettings {
            mlx_enabled: true,
            mlx_engine: "service".into(),
            mlx_url: "http://127.0.0.1:9/v1".into(),
            ..Default::default()
        },
        runtime: s.local_runtime.clone(),
        ..Default::default()
    };
    let err = providers::stream(&config, "x", &[Message::User("hi".into())], &[], |_| Ok(()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Could not connect"), "{err}");
    let (status, err) = call(
        &s,
        "PATCH",
        "/api/settings",
        json!({"local_ai":{"mlx_engine":"python"}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err["error"]
            .as_str()
            .unwrap()
            .contains("builtin or service")
    );
    Ok(())
}

#[tokio::test]
#[ignore = "Runs the real Rust MLX worker (make mlx-ai) with an installed Qwen3.5 4B; downloads nothing"]
async fn live_mlx_worker_answers_with_the_installed_qwen_4b() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s =
        AppState::open_with_credentials(dir.path().into(), false, None, Credentials::default())?;
    assert!(
        s.local_runtime.mlx_worker.is_file(),
        "run make mlx-ai first"
    );
    let m = models(&s).await;
    let qwen = m.models.iter().find(|m| m.id == "qwen3.5-4b").unwrap();
    assert!(
        qwen.installed,
        "install Qwen3.5 4B first (Backend settings or CausewaybayMaker)"
    );
    println!(
        "unified memory {:.0} GB, GPU budget {:.0} GB, fit {}",
        m.unified_memory_bytes.unwrap_or(0) as f64 / GIB as f64,
        m.gpu_budget_bytes.unwrap_or(0) as f64 / GIB as f64,
        qwen.fit
    );
    preferences::update(
        &s,
        SettingsUpdate {
            provider: Some("mlx".into()),
            model: Some(REPO.into()),
            local_ai: Some(LocalAiSettings {
                local_only: true,
                mlx_enabled: true,
                mlx_engine: "builtin".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let (status, memo) = call(&s, "POST", "/api/think", json!({"provider":"mlx","prompt":"What is seven plus five? Reply with the number only.","input_mode":"text"})).await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    let id = memo["id"].as_str().unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(120), async {
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
    println!("answer: {}", answer.answer);
    assert!(answer.answer.contains("12") || answer.answer.to_lowercase().contains("twelve"));
    assert_eq!(answer.provider, "mlx");
    let progress = |id: &str| -> Vec<causewaybay_shared::ModelProgress> {
        s.db.events(id, 0)
            .unwrap()
            .iter()
            .filter(|e| e.kind == "progress")
            .map(|e| serde_json::from_str(&e.text).unwrap())
            .collect()
    };
    let first = progress(id);
    println!("first request progress: {first:?}");
    assert!(
        first
            .iter()
            .any(|p| p.label == "Loading Qwen3.5 4B into memory")
    );
    // The single-lane manager keeps the model loaded: the next request skips the load.
    let (status, memo) = call(&s, "POST", "/api/think", json!({"provider":"mlx","prompt":"What is two plus two? Reply with the number only.","input_mode":"text"})).await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    let id2 = memo["id"].as_str().unwrap();
    let started = std::time::Instant::now();
    let second = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let m = s.db.get(id2).unwrap().unwrap();
            if m.status != "running" {
                break m;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    println!(
        "second answer: {} in {:?}",
        second.answer,
        started.elapsed()
    );
    assert_eq!(second.status, "complete", "{:?}", second.error);
    assert!(second.answer.contains('4'));
    let again = progress(id2);
    println!("second request progress: {again:?}");
    assert!(
        again.iter().any(|p| p.detail == "already in memory"),
        "{again:?}"
    );
    assert!(
        !again.iter().any(|p| p.label.starts_with("Loading")),
        "{again:?}"
    );
    assert_eq!(models(&s).await.loaded_model.as_deref(), Some("Qwen3.5 4B"));
    let (status, usage) = call(&s, "GET", "/api/usage", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let mlx = usage["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == "mlx")
        .unwrap();
    assert!(mlx["output_tokens"].as_u64().unwrap_or(0) > 0, "{mlx}");
    Ok(())
}

#[tokio::test]
#[ignore = "Downloads the real Qwen3.5 4B (2.5 GB) from Hugging Face into a temp dir, then answers through bin/invisible-mlx"]
async fn live_download_qwen_4b_from_hugging_face_and_answer() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut s =
        AppState::open_with_credentials(dir.path().into(), false, None, Credentials::default())?;
    let runtime = &mut Arc::get_mut(&mut s).unwrap().local_runtime;
    runtime.hf_cache = dir.path().join("empty-hf-cache");
    let worker = runtime.mlx_worker.clone();
    assert!(worker.is_file(), "run make mlx-ai first");
    assert!(!models(&s).await.models[2].installed);
    let started = std::time::Instant::now();
    let (status, body) = call(
        &s,
        "POST",
        "/api/admin/local-ai/models/fetch",
        json!({"id":"qwen3.5-4b"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut last = 0;
    let m = tokio::time::timeout(Duration::from_secs(3600), async {
        loop {
            let m = models(&s).await;
            let d = m.models[2].download.clone().unwrap();
            if d.done_bytes / 200_000_000 > last / 200_000_000 || d.finished {
                println!(
                    "{:>5.1}s {:>6.1}% {}/{} {} ({} B/s) {}",
                    started.elapsed().as_secs_f64(),
                    d.fraction() * 100.0,
                    d.done_files,
                    d.total_files,
                    d.file,
                    d.bytes_per_second,
                    d.error.clone().unwrap_or_default()
                );
            }
            last = d.done_bytes;
            if d.finished {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await?;
    let d = m.models[2].download.clone().unwrap();
    assert!(d.error.is_none() && !d.cancelled, "{d:?}");
    assert!(m.models[2].installed);
    let local = local_models::local_dir(&s.local_runtime.models_dir, REPO);
    assert_eq!(
        std::fs::metadata(local.join("model.safetensors"))?.len(),
        3_034_300_695
    );
    assert!(!local.join("README.md").exists() && !local.join("preprocessor_config.json").exists());
    preferences::update(
        &s,
        SettingsUpdate {
            provider: Some("mlx".into()),
            model: Some(REPO.into()),
            local_ai: Some(LocalAiSettings {
                mlx_enabled: true,
                mlx_engine: "builtin".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let config = s.provider_config()?;
    let turn = providers::stream(
        &config,
        "Answer briefly.",
        &[Message::User(
            "What is seven plus five? Reply with the number only.".into(),
        )],
        &[],
        |_| Ok(()),
    )
    .await?;
    println!("answer from the downloaded model: {}", turn.text.trim());
    assert!(turn.text.contains("12"));
    assert!(turn.output_tokens.unwrap_or(0) > 0);
    Ok(())
}
