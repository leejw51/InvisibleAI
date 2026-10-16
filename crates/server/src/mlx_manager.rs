//! One single-lane manager for the Rust MLX worker. The backend starts
//! `invisible-mlx --serve` once, keeps the model loaded between requests, and
//! answers users one at a time in arrival order; the GPU never runs two
//! generations at once. Each job hears its queue position and the model load
//! as they happen, so the chat can show them. An abandoned job is cancelled in
//! the worker without unloading the model; a dead worker is restarted on the
//! next request; an idle worker is stopped to give unified memory back.
use anyhow::{Context, Result, bail, ensure};
use causewaybay_shared::ModelProgress;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, watch},
};

/// Loading the 27B and answering a long request both fit comfortably here.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// After this long without a request the worker is stopped and memory freed.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(20 * 60);
const MAX_OUTPUT: usize = 2_000_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The model the worker currently holds in memory.
    pub loaded_model: Option<String>,
    pub busy: bool,
    pub waiting: usize,
}

struct Job {
    id: String,
    model: String,
    request: Value,
    events: mpsc::UnboundedSender<Value>,
}

pub struct Manager {
    inbox: Mutex<Option<mpsc::UnboundedSender<Job>>>,
    status: watch::Sender<Status>,
    waiting: Arc<AtomicUsize>,
    idle: Duration,
    /// The live worker's process id, so a backend shutdown (SIGTERM from
    /// `make stop`, Ctrl-C) takes the model process down with it instead of
    /// leaving gigabytes of weights resident in an orphan.
    worker_pid: Arc<Mutex<Option<u32>>>,
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
impl Default for Manager {
    fn default() -> Self {
        Self::new(IDLE_UNLOAD)
    }
}
impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MlxManager({:?})", self.status())
    }
}
impl Manager {
    pub fn new(idle: Duration) -> Self {
        Self {
            inbox: Mutex::new(None),
            status: watch::channel(Status::default()).0,
            waiting: Arc::new(AtomicUsize::new(0)),
            idle,
            worker_pid: Arc::new(Mutex::new(None)),
        }
    }
    /// Stop the worker now (its whole process group) and forget the lane; the
    /// next request starts a fresh one. Safe to call when nothing runs.
    pub fn shutdown(&self) {
        if let Some(pid) = self.worker_pid.lock().unwrap().take() {
            kill_group(pid);
        }
        *self.inbox.lock().unwrap() = None;
        self.status.send_modify(|s| {
            s.loaded_model = None;
            s.busy = false;
        });
    }
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }
    pub fn watch(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }
    /// The actor starts with the first request and lives as long as the backend.
    fn inbox(&self, worker: &Path) -> mpsc::UnboundedSender<Job> {
        let mut inbox = self.inbox.lock().unwrap();
        if let Some(tx) = inbox.as_ref().filter(|tx| !tx.is_closed()) {
            return tx.clone();
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(actor(
            worker.to_path_buf(),
            rx,
            self.status.clone(),
            self.waiting.clone(),
            self.idle,
            self.worker_pid.clone(),
        ));
        *inbox = Some(tx.clone());
        tx
    }

    /// Queue one generate request and stream its text. `model` is the name
    /// shown to the user; `request` is the worker's JSON (an installed
    /// `model_dir`, never a repo id).
    pub async fn generate(
        &self,
        worker: &Path,
        model: &str,
        mut request: Value,
        progress: Option<&crate::providers::ProgressSink>,
        mut on_text: impl FnMut(&str) -> Result<()>,
    ) -> Result<crate::local_ai::WorkerOutput> {
        ensure!(
            request.to_string().len() <= 1_000_000,
            "Local request is too large"
        );
        let id = crate::db::id();
        request["id"] = json!(id);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let report = |p: ModelProgress| {
            if let Some(sink) = progress {
                sink(p);
            }
        };
        let ahead =
            self.waiting.fetch_add(1, Ordering::SeqCst) + usize::from(self.status.borrow().busy);
        self.status
            .send_modify(|s| s.waiting = self.waiting.load(Ordering::SeqCst));
        if ahead > 0 {
            report(ModelProgress {
                label: "Waiting for the on-device model".into(),
                detail: if ahead == 1 {
                    "1 request ahead".into()
                } else {
                    format!("{ahead} requests ahead")
                },
                fraction: None,
            });
        }
        if self
            .inbox(worker)
            .send(Job {
                id,
                model: model.to_owned(),
                request,
                events: tx,
            })
            .is_err()
        {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            bail!("The on-device worker manager stopped; retry");
        }
        // Dropping `rx` (timeout, or the user stopping the answer) tells the
        // actor to cancel this request in the worker.
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            let mut text = String::new();
            let mut done = Value::Null;
            let mut total = 0usize;
            while let Some(event) = rx.recv().await {
                total += event.to_string().len();
                ensure!(
                    total <= MAX_OUTPUT,
                    "Local worker exceeded its output limit"
                );
                if let Some(stage) = event["stage"].as_str() {
                    match stage {
                        "loading" => report(ModelProgress {
                            label: format!("Loading {model} into memory"),
                            detail: String::new(),
                            fraction: None,
                        }),
                        "loaded" => report(ModelProgress {
                            label: format!("{model} ready"),
                            detail: if event["cached"] == true {
                                "already in memory".into()
                            } else {
                                format!(
                                    "loaded in {:.1} s",
                                    event["load_ms"].as_u64().unwrap_or(0) as f64 / 1000.0
                                )
                            },
                            fraction: None,
                        }),
                        _ => {}
                    }
                } else if let Some(code) = event["error"].as_str() {
                    bail!("{}", crate::local_ai::worker_message(code));
                } else if let Some(delta) = event["delta"].as_str() {
                    text.push_str(delta);
                    on_text(delta)?;
                } else if event["done"] == true {
                    done = event;
                    break;
                }
            }
            ensure!(
                !done.is_null(),
                "Local worker stopped before completing its response"
            );
            ensure!(
                done["cancelled"] != true,
                "The on-device model stopped this answer"
            );
            ensure!(!text.trim().is_empty(), "Local worker returned no text");
            Ok(crate::local_ai::WorkerOutput { text, done })
        })
        .await
        .context("Local AI timed out; its request was cancelled")?
    }
}

struct Lane {
    _child: Child,
    pid: Option<u32>,
    stdin: ChildStdin,
    lines: tokio::io::Lines<BufReader<ChildStdout>>,
}
impl Drop for Lane {
    fn drop(&mut self) {
        // kill_on_drop reaches the worker; its group catches anything it forked.
        if let Some(pid) = self.pid {
            kill_group(pid);
        }
    }
}
/// Kill a worker's whole process group; the worker leads its own group.
fn kill_group(pid: u32) {
    let _ = std::process::Command::new("/bin/kill")
        .args(["-9", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = std::process::Command::new("/bin/kill")
        .args(["-9", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
fn spawn(worker: &Path) -> Result<Lane> {
    let mut child = tokio::process::Command::new(worker)
        .arg("--serve")
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .env("HF_HUB_DISABLE_TELEMETRY", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .context("The MLX worker could not start; run `make mlx-ai` on the backend")?;
    let stdin = child.stdin.take().context("Worker input is unavailable")?;
    let stdout = child
        .stdout
        .take()
        .context("Worker output is unavailable")?;
    Ok(Lane {
        pid: child.id(),
        _child: child,
        stdin,
        lines: BufReader::new(stdout).lines(),
    })
}

enum Outcome {
    /// The worker answered (or reported a request-level error) and is still healthy.
    Served { loaded: bool },
    /// The worker died, hung, or spoke nonsense: it is dropped and restarted next time.
    Broken(String),
}
async fn serve(lane: &mut Lane, job: &Job) -> Outcome {
    if job.events.is_closed() {
        // The user gave up while queued: no load, no prefill.
        return Outcome::Served { loaded: false };
    }
    let mut line = job.request.to_string();
    line.push('\n');
    if lane.stdin.write_all(line.as_bytes()).await.is_err() {
        return Outcome::Broken("worker input closed".into());
    }
    let mut cancel_sent = false;
    let mut loaded = false;
    loop {
        let next = tokio::time::timeout(REQUEST_TIMEOUT, lane.lines.next_line()).await;
        let line = match next {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => return Outcome::Broken("worker exited".into()),
            Ok(Err(_)) => return Outcome::Broken("worker output failed".into()),
            Err(_) => return Outcome::Broken("worker timed out".into()),
        };
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            return Outcome::Broken("invalid worker event".into());
        };
        if event["id"] != job.id {
            // Stale output from a request that was cancelled earlier.
            continue;
        }
        if event["stage"] == "loaded" {
            loaded = true;
        }
        let finished = event["done"] == true || event["error"].is_string();
        if job.events.send(event).is_err() && !cancel_sent && !finished {
            cancel_sent = true;
            let cancel = format!("{}\n", json!({"cancel": job.id}));
            if lane.stdin.write_all(cancel.as_bytes()).await.is_err() {
                return Outcome::Broken("worker input closed".into());
            }
        }
        if finished {
            return Outcome::Served { loaded };
        }
    }
}

async fn actor(
    worker: PathBuf,
    mut rx: mpsc::UnboundedReceiver<Job>,
    status: watch::Sender<Status>,
    waiting: Arc<AtomicUsize>,
    idle: Duration,
    worker_pid: Arc<Mutex<Option<u32>>>,
) {
    let mut lane: Option<Lane> = None;
    let mut last_used = Instant::now();
    loop {
        let job = tokio::select! {
            job = rx.recv() => match job { Some(job) => job, None => break },
            _ = tokio::time::sleep(idle.saturating_sub(last_used.elapsed())), if lane.is_some() => {
                lane = None;
                *worker_pid.lock().unwrap() = None;
                status.send_modify(|s| s.loaded_model = None);
                continue;
            }
        };
        waiting.fetch_sub(1, Ordering::SeqCst);
        status.send_modify(|s| {
            s.busy = true;
            s.waiting = waiting.load(Ordering::SeqCst);
        });
        if lane.is_none() {
            match spawn(&worker) {
                Ok(l) => {
                    *worker_pid.lock().unwrap() = l.pid;
                    lane = Some(l);
                }
                Err(e) => {
                    let _ = job
                        .events
                        .send(json!({"error":"spawn_failed","message":e.to_string()}));
                    status.send_modify(|s| s.busy = false);
                    continue;
                }
            }
        }
        let outcome = serve(lane.as_mut().unwrap(), &job).await;
        last_used = Instant::now();
        match outcome {
            Outcome::Served { loaded } => {
                if loaded {
                    status.send_modify(|s| s.loaded_model = Some(job.model.clone()));
                }
            }
            Outcome::Broken(reason) => {
                tracing::warn!(reason, "MLX worker lane reset");
                lane = None;
                *worker_pid.lock().unwrap() = None;
                let _ = job.events.send(json!({"error":"worker_failed"}));
                status.send_modify(|s| s.loaded_model = None);
            }
        }
        status.send_modify(|s| {
            s.busy = false;
            s.waiting = waiting.load(Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_kills_the_worker_process_group() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir()?;
        let script = dir.path().join("worker.sh");
        // A worker that forks a helper and waits, like a model process would.
        std::fs::write(&script, "#!/bin/sh\nsleep 300 &\nsleep 300\n")?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        let mut lane = spawn(&script)?;
        let pid = lane.pid.expect("pid");
        let group_members = |pid: u32| {
            std::process::Command::new("/usr/bin/pgrep")
                .args(["-g", &pid.to_string()])
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .split_whitespace()
                        .count()
                })
                .unwrap_or(0)
        };
        // Under a parallel build the shell may take a while to fork its helper.
        for _ in 0..100 {
            if group_members(pid) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            group_members(pid) >= 2,
            "the worker and its helper share a group"
        );
        let manager = Manager::default();
        *manager.worker_pid.lock().unwrap() = Some(pid);
        manager.shutdown();
        // The leader exits by signal and is reaped here; the helper is gone too.
        let status = tokio::time::timeout(Duration::from_secs(5), lane._child.wait()).await??;
        assert!(!status.success());
        for _ in 0..50 {
            if group_members(pid) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            group_members(pid),
            0,
            "nothing of the worker's group survives"
        );
        assert!(manager.worker_pid.lock().unwrap().is_none());
        lane.pid = None;
        manager.shutdown();
        Ok(())
    }
}
