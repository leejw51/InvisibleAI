//! On-device Qwen worker for the Invisible AI backend. JSON on stdin, JSON lines
//! on stdout, like the Apple Foundation Models worker. One-shot by default;
//! `--serve` keeps running as a single-lane manager: one request at a time,
//! the model kept loaded between them, every event tagged with the request id.
//! It only ever loads a model directory it is handed; it never downloads anything.
//! The model and the Metal kernel are derived from mlx-lm (MIT, Apple Inc.);
//! see THIRD_PARTY_NOTICES.

mod kernel;
mod model;

use std::collections::VecDeque;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use mlx_rs::ops::indexing::argmax_axis;
use mlx_rs::ops::{partition_axis, select, split_at_indices};
use mlx_rs::{Array, random, transforms};
use serde::Deserialize;
use serde_json::{Value, json};
use tokenizers::Tokenizer;

use model::{LayerCache, Model};

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct Request {
    /// Serve mode tags every event with this; one-shot requests carry none.
    #[serde(default)]
    id: Option<String>,
    operation: String,
    /// `{"cancel": "<id>"}` stops the request with that id.
    #[serde(default)]
    cancel: Option<String>,
    /// Absolute directory holding config.json, tokenizer.json and *.safetensors.
    model_dir: Option<PathBuf>,
    system: Option<String>,
    #[serde(default)]
    messages: Vec<ChatMessage>,
    /// Single-turn shorthand when `messages` is empty.
    prompt: Option<String>,
    max_tokens: Option<usize>,
    temperature: Option<f32>,
}

/// Only these codes cross the process boundary; the backend maps them to text.
enum WorkerError {
    InvalidRequest,
    ModelMissing,
    Failed,
}
impl WorkerError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::ModelMissing => "mlx_model_missing",
            Self::Failed => "worker_failed",
        }
    }
}

fn emit(event: Value) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(event.to_string().as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

/// Qwen chat template (ChatML) with thinking switched off, as in CausewaybayMaker.
pub fn build_prompt(system: &str, messages: &[(String, String)]) -> String {
    let mut s = String::new();
    if !system.trim().is_empty() {
        s.push_str(&format!(
            "<|im_start|>system\n{}<|im_end|>\n",
            system.trim()
        ));
    }
    for (role, content) in messages {
        let role = if role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        s.push_str(&format!("<|im_start|>{role}\n{content}<|im_end|>\n"));
    }
    s.push_str("<|im_start|>assistant\n<think>\n\n</think>\n\n");
    s
}

struct Sampler {
    temp: f32,
    top_k: i32,
}

impl Sampler {
    fn sample(&self, logits: &Array) -> Result<Array> {
        if self.temp <= 0.0 {
            return Ok(argmax_axis(logits, -1, None)?);
        }
        let mut logits = logits.multiply(Array::from_f32(1.0 / self.temp))?;
        let vocab = logits.dim(-1);
        if self.top_k > 0 && self.top_k < vocab {
            let part = partition_axis(&logits, vocab - self.top_k, -1)?;
            let kth = split_at_indices(&part, &[vocab - self.top_k, vocab - self.top_k + 1], -1)?
                .swap_remove(1);
            let ninf = Array::from_f32(f32::NEG_INFINITY).as_dtype(logits.dtype())?;
            logits = select(&logits.lt(&kth)?, &ninf, &logits)?;
        }
        Ok(random::categorical(&logits, -1, None, None)?)
    }
}

struct Engine {
    dir: PathBuf,
    model: Model,
    tok: Tokenizer,
    eos: Vec<u32>,
    sampler: Sampler,
}

impl Engine {
    fn load(dir: &Path, temp: f32) -> Result<Self> {
        let model = Model::load(dir, true)?;
        let tok = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(anyhow::Error::msg)?;
        let eos = ["<|im_end|>", "<|endoftext|>"]
            .iter()
            .filter_map(|t| tok.token_to_id(t))
            .collect();
        Ok(Self {
            dir: dir.to_path_buf(),
            model,
            tok,
            eos,
            sampler: Sampler { temp, top_k: 20 },
        })
    }

    /// Prefill, then pipelined decode: the next step is queued before the current
    /// token is read back, so the GPU never waits on the CPU.
    fn generate(
        &self,
        prompt_ids: &[u32],
        cache: &mut [LayerCache],
        max_tokens: usize,
        mut stop: impl FnMut() -> bool,
        mut on_token: impl FnMut(u32),
    ) -> Result<(usize, bool)> {
        let ids: Vec<i32> = prompt_ids.iter().map(|&t| t as i32).collect();
        let input = Array::from_slice(&ids, &[1, ids.len() as i32]);
        let logits = self.model.forward(&input, cache)?;
        let mut y = self.sampler.sample(&logits)?;
        transforms::async_eval([&y])?;
        let mut n = 0;
        let mut cancelled = false;
        while n < max_tokens {
            if stop() {
                cancelled = true;
                break;
            }
            let next = if n + 1 < max_tokens {
                let logits = self.model.forward(&y.reshape(&[1, 1])?, cache)?;
                let nx = self.sampler.sample(&logits)?;
                transforms::async_eval([&nx])?;
                Some(nx)
            } else {
                None
            };
            let token = y.item_cast::<u32>();
            n += 1;
            if self.eos.contains(&token) {
                break;
            }
            on_token(token);
            match next {
                Some(nx) => y = nx,
                None => break,
            }
        }
        Ok((n, cancelled))
    }
}

/// Events for one request, tagged with its id in serve mode.
struct Tagged<'a>(Option<&'a str>);
impl Tagged<'_> {
    fn emit(&self, mut event: Value) {
        if let Some(id) = self.0 {
            event["id"] = json!(id);
        }
        emit(event);
    }
}

/// Run one generate request, reusing `engine` when it already holds the same
/// model directory. The previous model is dropped before another one loads so
/// two never share unified memory.
fn run(
    request: Request,
    engine: &mut Option<Engine>,
    mut stop: impl FnMut() -> bool,
) -> Result<(), WorkerError> {
    let out = Tagged(request.id.as_deref());
    if request.operation != "generate" {
        return Err(WorkerError::InvalidRequest);
    }
    let dir = request.model_dir.ok_or(WorkerError::InvalidRequest)?;
    if !dir.is_absolute()
        || !dir.join("config.json").is_file()
        || !dir.join("tokenizer.json").is_file()
    {
        return Err(WorkerError::ModelMissing);
    }
    let mut messages: Vec<(String, String)> = request
        .messages
        .into_iter()
        .map(|m| (m.role, m.content))
        .collect();
    if messages.is_empty() {
        let prompt = request.prompt.unwrap_or_default();
        if prompt.trim().is_empty() {
            return Err(WorkerError::InvalidRequest);
        }
        messages.push(("user".into(), prompt));
    }
    let max_tokens = request.max_tokens.unwrap_or(2048).clamp(1, 8192);
    let prompt = build_prompt(request.system.as_deref().unwrap_or(""), &messages);

    let cached = engine.as_ref().is_some_and(|e| e.dir == dir);
    let t0 = Instant::now();
    if !cached {
        *engine = None;
        out.emit(json!({"stage":"loading","model":dir}));
        let loaded = Engine::load(&dir, request.temperature.unwrap_or(0.7))
            .map_err(|_| WorkerError::Failed)?;
        *engine = Some(loaded);
    } else if let Some(temp) = request.temperature
        && let Some(e) = engine.as_mut()
    {
        e.sampler.temp = temp;
    }
    let load_ms = t0.elapsed().as_millis() as u64;
    out.emit(json!({"stage":"loaded","load_ms":load_ms,"cached":cached}));
    let engine = engine.as_ref().ok_or(WorkerError::Failed)?;
    let ids = engine
        .tok
        .encode(prompt.as_str(), false)
        .map_err(|_| WorkerError::Failed)?
        .get_ids()
        .to_vec();

    let mut cache = engine.model.make_cache();
    let mut outv: Vec<u32> = Vec::new();
    let mut printed = 0;
    let t1 = Instant::now();
    let (produced, cancelled) = engine
        .generate(&ids, &mut cache, max_tokens, &mut stop, |t| {
            outv.push(t);
            if let Ok(text) = engine.tok.decode(&outv, false) {
                // Hold back partial UTF-8 sequences until they complete.
                if !text.ends_with('\u{FFFD}') && text.len() > printed {
                    out.emit(json!({"delta": &text[printed..]}));
                    printed = text.len();
                }
            }
        })
        .map_err(|_| WorkerError::Failed)?;
    let gen_ms = t1.elapsed().as_millis() as u64;
    let peak_bytes = mlx_rs::memory::peak_memory().unwrap_or(0);
    out.emit(json!({
        "done": true,
        "cancelled": cancelled,
        "input_tokens": ids.len(),
        "output_tokens": produced,
        "load_ms": load_ms,
        "generation_ms": gen_ms,
        "peak_memory_bytes": peak_bytes,
    }));
    Ok(())
}

/// Serve mode: a single lane. Requests are read on a thread into a channel;
/// the main thread serves them one at a time and, between tokens, looks for a
/// `{"cancel": id}` line so an abandoned answer stops without unloading the model.
fn serve() {
    let (tx, rx) = mpsc::channel::<Result<Request, ()>>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.lock().read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if line.trim().is_empty() {
                continue;
            }
            let parsed = if line.len() > 1_000_000 {
                Err(())
            } else {
                serde_json::from_str::<Request>(&line).map_err(|_| ())
            };
            if tx.send(parsed).is_err() {
                break;
            }
        }
    });
    let mut engine: Option<Engine> = None;
    let mut cancelled: Vec<String> = Vec::new();
    // Requests that arrive while one is being answered wait here, in order.
    let mut pending: VecDeque<Result<Request, ()>> = VecDeque::new();
    loop {
        let next = match pending.pop_front() {
            Some(next) => next,
            None => match rx.recv() {
                Ok(next) => next,
                Err(_) => break,
            },
        };
        let request = match next {
            Ok(r) => r,
            Err(()) => {
                emit(json!({"error": WorkerError::InvalidRequest.code()}));
                continue;
            }
        };
        if let Some(id) = request.cancel {
            cancelled.push(id);
            continue;
        }
        let id = request.id.clone();
        match request.operation.as_str() {
            "ping" => {
                Tagged(id.as_deref())
                    .emit(json!({"pong":true,"loaded":engine.as_ref().map(|e|e.dir.clone())}));
                continue;
            }
            "unload" => {
                engine = None;
                Tagged(id.as_deref()).emit(json!({"done":true,"unloaded":true}));
                continue;
            }
            _ => {}
        }
        if id.as_ref().is_some_and(|i| cancelled.contains(i)) {
            cancelled.retain(|c| Some(c) != id.as_ref());
            Tagged(id.as_deref())
                .emit(json!({"done":true,"cancelled":true,"input_tokens":0,"output_tokens":0}));
            continue;
        }
        let stop = {
            let id = id.clone();
            let rx = &rx;
            let cancelled = &mut cancelled;
            let pending = &mut pending;
            move || {
                // Between tokens, look for a cancel line; anything else queues up.
                while let Ok(next) = rx.try_recv() {
                    match next {
                        Ok(r) if r.cancel.is_some() => cancelled.push(r.cancel.unwrap()),
                        other => pending.push_back(other),
                    }
                }
                let hit = id.as_ref().is_some_and(|i| cancelled.contains(i));
                if hit {
                    cancelled.retain(|c| Some(c) != id.as_ref());
                }
                hit
            }
        };
        if let Err(e) = run(request, &mut engine, stop) {
            Tagged(id.as_deref()).emit(json!({"error": e.code()}));
        }
    }
}

fn read_request() -> Result<Request> {
    let mut data = Vec::new();
    std::io::stdin()
        .lock()
        .take(1_000_001)
        .read_to_end(&mut data)
        .context("stdin")?;
    if data.len() > 1_000_000 {
        bail!("request too large");
    }
    Ok(serde_json::from_slice(&data)?)
}

fn main() {
    // Release bundles carry the compiled kernels next to this executable.
    if let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("mlx.metallib")))
        .filter(|p| p.is_file())
    {
        if let Err(error) = mlx_rs::metal::set_metallib_path(path.to_string_lossy()) {
            eprintln!("Could not load bundled Metal kernels: {error}");
            std::process::exit(1);
        }
    }
    if std::env::args().any(|a| a == "--self-test") {
        let result = (|| -> Result<()> {
            let a = Array::from_slice(&[1i32, 2], &[2]);
            let doubled = a.add(&a)?;
            doubled.eval()?;
            anyhow::ensure!(
                doubled.as_slice::<i32>() == [2, 4],
                "Metal calculation failed"
            );
            Ok(())
        })();
        emit(json!({"ok": result.is_ok()}));
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if std::env::args().any(|a| a == "--serve") {
        serve();
        return;
    }
    let Ok(request) = read_request() else {
        emit(json!({"error": WorkerError::InvalidRequest.code()}));
        std::process::exit(1);
    };
    let mut engine = None;
    if let Err(e) = run(request, &mut engine, || false) {
        // Never echo prompts, paths or framework diagnostics: the backend relays stdout.
        emit(json!({"error": e.code()}));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::build_prompt;

    #[test]
    fn prompt_follows_chatml_with_thinking_off() {
        let p = build_prompt(
            "Be brief.",
            &[
                ("user".into(), "Hi".into()),
                ("assistant".into(), "Hello".into()),
                ("user".into(), "2+2?".into()),
            ],
        );
        assert_eq!(
            p,
            "<|im_start|>system\nBe brief.<|im_end|>\n<|im_start|>user\nHi<|im_end|>\n<|im_start|>assistant\nHello<|im_end|>\n<|im_start|>user\n2+2?<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
        );
        assert!(
            build_prompt("  ", &[("user".into(), "x".into())]).starts_with("<|im_start|>user\n")
        );
    }
}
