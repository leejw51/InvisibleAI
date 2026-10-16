pub mod activity;
pub mod agent;
pub mod ai_activity;
pub mod ai_preferences;
pub mod ai_status;
pub mod artifacts;
pub mod assistance;
pub mod auth;
pub mod capture;
pub mod capture_choices;
pub mod chat;
pub mod cli_ai;
pub mod client_package;
pub mod code;
pub mod coding;
pub mod connections;
pub mod continuous;
pub mod credentials;
pub mod db;
pub mod documents;
pub mod invisible_sessions;
pub mod library;
pub mod local_ai;
pub mod local_models;
pub mod media;
pub mod mlx_manager;
pub mod nodes;
pub mod notes;
pub mod notifications;
pub mod preferences;
pub mod productivity;
pub mod providers;
pub mod remote;
pub mod schema;
pub mod semantic;
pub mod startup;
pub mod status;
pub mod storage;
pub mod toolchains;
pub mod transport;
pub mod usage;
pub mod users;
pub mod wallet;
pub mod wiki;

use anyhow::Result;
macro_rules! ensure { ($condition:expr, $($message:tt)*) => { if !$condition { return Err(anyhow::anyhow!($($message)*).into()); } }; }
// Declared after the `ensure!` macro so both modules can use it.
pub mod routes;
pub mod uploads;
pub use routes::{api_routes, app};
mod think;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use causewaybay_shared::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use think::*;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

#[derive(Default)]
struct OwnerLocks {
    erasure: tokio::sync::RwLock<()>,
    projects: Mutex<()>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: db::Database,
    pub auth: Arc<auth::Auth>,
    credentials: Arc<credentials::Credentials>,
    pub semantic: Arc<semantic::Semantic>,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    settings_write: Arc<Mutex<()>>,
    chat_submit: Arc<Mutex<()>>,
    owner_locks: Arc<Mutex<HashMap<String, Arc<OwnerLocks>>>>,
    continuous: Arc<Mutex<HashMap<String, continuous::Running>>>,
    pub local_runtime: local_ai::Runtime,
    pub notes: Arc<notes::Hub>,
    pub demo: bool,
    pub token: Option<String>,
    pub capture_token: Option<String>,
    pub started: std::time::Instant,
    pub addresses: Arc<std::sync::OnceLock<Vec<causewaybay_shared::BackendAddress>>>,
    jobs: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    slots: Arc<tokio::sync::Semaphore>,
    capture_sessions: Arc<Mutex<HashMap<String, String>>>,
    /// Whether `compile_code` can run here (macOS with a rustup toolchain).
    pub compiler: bool,
    /// Build and run tools for the coding agent's projects, detected once.
    pub toolchains: Arc<toolchains::Toolchains>,
    /// One agent at a time per project folder.
    coding_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

pub struct PrivateState(pub Arc<AppState>);
impl axum::extract::FromRequestParts<Arc<AppState>> for PrivateState {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &Arc<AppState>,
    ) -> Api<Self> {
        let actor = auth::authorize(
            state,
            &parts.headers,
            parts.method.as_str(),
            parts.uri.path(),
        )?;
        Ok(Self(
            state.for_owner(actor.address.as_deref().unwrap_or("local")),
        ))
    }
}
impl AppState {
    fn owner_locks(&self) -> Arc<OwnerLocks> {
        self.owner_locks
            .lock()
            .unwrap()
            .entry(self.db.owner().to_owned())
            .or_default()
            .clone()
    }

    pub fn for_owner(&self, owner: &str) -> Arc<Self> {
        let mut scoped = self.clone();
        scoped.db = self.db.for_owner(owner);
        Arc::new(scoped)
    }

    pub fn open(data_dir: PathBuf, demo: bool, token: Option<String>) -> Result<Arc<Self>> {
        Self::open_with_credentials(
            data_dir,
            demo,
            token,
            credentials::Credentials::from_environment(),
        )
    }
    pub fn open_configured(
        data_dir: PathBuf,
        config_dir: PathBuf,
        demo: bool,
        token: Option<String>,
    ) -> Result<Arc<Self>> {
        Self::open_at(
            data_dir,
            config_dir,
            demo,
            token,
            credentials::Credentials::from_environment(),
        )
    }
    pub fn open_with_credentials(
        data_dir: PathBuf,
        demo: bool,
        token: Option<String>,
        credentials: credentials::Credentials,
    ) -> Result<Arc<Self>> {
        Self::open_at(data_dir.clone(), data_dir, demo, token, credentials)
    }
    fn open_at(
        data_dir: PathBuf,
        config_dir: PathBuf,
        demo: bool,
        token: Option<String>,
        credentials: credentials::Credentials,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let capture_token = if demo {
            token.clone()
        } else {
            Some(causewaybay_shared::config::secret(
                &config_dir,
                "capture-token",
            )?)
        };
        let state = Arc::new(Self {
            auth: Arc::new(auth::Auth::new(std::env::var("CWBBOT_ADMIN").ok())?),
            db: db::Database::open(&data_dir.join("causewaybay.sqlite3"))?,
            credentials: Arc::new(credentials),
            local_runtime: local_ai::Runtime::from_environment()
                .with_models_dir(data_dir.join("models/mlx")),
            notes: Arc::new(notes::Hub::default()),
            semantic: semantic::Semantic::new(),
            data_dir,
            config_dir,
            settings_write: Arc::new(Mutex::new(())),
            chat_submit: Arc::new(Mutex::new(())),
            owner_locks: Arc::new(Mutex::new(HashMap::new())),
            continuous: Arc::new(Mutex::new(HashMap::new())),
            demo,
            token,
            capture_token,
            started: std::time::Instant::now(),
            addresses: Arc::new(std::sync::OnceLock::new()),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
            capture_sessions: Arc::new(Mutex::new(HashMap::new())),
            compiler: agent::compiler_available(),
            toolchains: Arc::new(toolchains::Toolchains::default()),
            coding_locks: Arc::new(Mutex::new(HashMap::new())),
        });
        preferences::restore(&state)?;
        nodes::restore(&state)?;
        users::restore(&state)?;
        if demo && state.db.count()? == 0 {
            seed(&state)?;
        }
        Ok(state)
    }
    pub fn local_ai(&self) -> Result<LocalAiSettings> {
        self.db
            .get_setting("local_ai")?
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .unwrap_or_else(|| Ok(LocalAiSettings::default()))
    }
    pub fn provider_configured(&self, id: &str) -> bool {
        let Ok(local) = self.local_ai() else {
            return false;
        };
        match id {
            "apple" => local.apple_enabled && self.local_runtime.apple_worker.is_file(),
            "mlx" => {
                local.mlx_enabled
                    && (local.mlx_engine == "service" || self.local_runtime.mlx_worker.is_file())
            }
            id if cli_ai::is_cli(id) => !local.local_only && cli_ai::available(id),
            _ => !local.local_only && self.credentials.configured(id),
        }
    }
    pub fn selected_provider(&self) -> Result<String> {
        if let Some(saved) = self.db.get_setting("provider")?
            && providers::PROVIDERS.iter().any(|r| r.0 == saved)
            && (!self.local_ai()?.local_only || local_ai::is_local(&saved))
            && (local_ai::is_local(&saved) || self.demo || self.provider_configured(&saved))
        {
            return Ok(saved);
        }
        if self.local_ai()?.local_only {
            return Ok(if self.local_ai()?.apple_enabled {
                "apple"
            } else {
                "mlx"
            }
            .into());
        }
        // Subscription tools are never chosen implicitly; the user picks their mode.
        Ok(providers::PROVIDERS
            .iter()
            .find(|r| !cli_ai::is_cli(r.0) && self.provider_configured(r.0))
            .unwrap_or(&providers::PROVIDERS[0])
            .0
            .into())
    }
    pub fn provider_config(&self) -> Result<providers::Config> {
        let provider = ai_preferences::read(self)?
            .map(|p| p.provider().to_owned())
            .unwrap_or(self.selected_provider()?);
        self.provider_config_for(&provider)
    }
    fn provider_config_for(&self, provider: &str) -> Result<providers::Config> {
        let row = providers::PROVIDERS
            .iter()
            .find(|r| r.0 == provider)
            .ok_or_else(|| anyhow::anyhow!("Unknown provider"))?;
        Ok(providers::Config {
            owner: self.db.owner().into(),
            model: ai_preferences::models(self)?
                .get(provider)
                .cloned()
                .or(self.db.get_setting(&format!("model.{provider}"))?)
                .unwrap_or_else(|| row.2.into()),
            provider: provider.into(),
            key: self.credentials.key(provider).into(),
            local: self.local_ai()?,
            runtime: self.local_runtime.clone(),
            progress: None,
            max_tokens: None,
        })
    }
    pub fn settings(&self) -> Result<Settings> {
        let mut providers = vec![];
        for (id, name, model, _) in providers::PROVIDERS {
            providers.push(Provider {
                id: (*id).into(),
                name: (*name).into(),
                model: self
                    .db
                    .get_setting(&format!("model.{id}"))?
                    .unwrap_or_else(|| (*model).into()),
                configured: self.provider_configured(id),
            });
        }
        Ok(Settings {
            input_mode: self
                .db
                .get_setting("input_mode")?
                .unwrap_or_else(|| "text".into()),
            provider: self.selected_provider()?,
            providers,
            capture_mode: self
                .db
                .get_setting("capture_mode")?
                .unwrap_or_else(|| "screen".into()),
            display: self
                .db
                .get_setting("display")?
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
            // Drafts belong to a browser wallet, never to shared server defaults.
            draft: String::new(),
            semantic_status: self.semantic.status.lock().unwrap().clone(),
            demo: self.demo,
            local_ai: self.local_ai()?,
            user_ai: ai_preferences::read(self)?,
            user_models: ai_preferences::models(self)?,
            user_capture: capture_choices::read(self)?,
            default_ai: ai_preferences::default_policy(self)?,
        })
    }
    pub fn start_indexer(self: &Arc<Self>, disabled: bool) {
        self.semantic.start(
            self.data_dir.join("models"),
            disabled
                || self.demo
                || self.local_ai().is_ok_and(|s| s.local_only)
                || std::env::var("SEMANTIC_SEARCH").is_ok_and(|s| s == "off"),
        );
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                if state.semantic.ready() {
                    let s = state.clone();
                    let _ = tokio::task::spawn_blocking(move || -> Result<()> {
                        for m in s.db.unindexed()? {
                            // Mean-pool sentence windows so the end of long answers stays searchable.
                            let text = format!("{}\n{}\n{}", m.title, m.input, m.answer);
                            let chars: Vec<char> = text.chars().take(30_000).collect();
                            let mut vectors = vec![];
                            for chunk in chars.chunks(900) {
                                if let Some(v) =
                                    s.semantic.embed(&chunk.iter().collect::<String>())?
                                {
                                    vectors.push(v);
                                }
                            }
                            if let Some(first) = vectors.first() {
                                let mut pooled = vec![0.; first.len()];
                                for v in &vectors {
                                    for (a, b) in pooled.iter_mut().zip(v) {
                                        *a += b / vectors.len() as f32;
                                    }
                                }
                                s.db.put_embedding_if_current(&m, &pooled)?;
                            }
                        }
                        Ok(())
                    })
                    .await;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }
}

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);
impl From<rusqlite::Error> for ApiError {
    fn from(_: rusqlite::Error) -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database operation failed".into(),
        )
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self(StatusCode::BAD_REQUEST, e.to_string())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
type Api<T> = std::result::Result<T, ApiError>;

fn constant_eq(a: &str, b: &str) -> bool {
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= (x ^ y) as usize;
    }
    diff == 0
}
async fn login(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Api<Response> {
    auth::same_origin(&headers)?;
    // Node tokens cannot be exchanged for browser/admin sessions in wallet mode.
    if s.auth.admin.is_none()
        && s.token
            .as_ref()
            .is_some_and(|t| constant_eq(v["token"].as_str().unwrap_or(""), t))
    {
        return auth::create_session(&s, &headers, None);
    }
    Err(ApiError(
        StatusCode::UNAUTHORIZED,
        "Use wallet login to open this workspace".into(),
    ))
}

async fn get_settings(PrivateState(s): PrivateState) -> Api<Json<Settings>> {
    Ok(Json(s.settings()?))
}
async fn update_settings(
    PrivateState(s): PrivateState,
    Json(v): Json<SettingsUpdate>,
) -> Api<Json<Settings>> {
    preferences::update(&s, v)?;
    Ok(Json(s.settings()?))
}

#[derive(Default, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}
pub async fn search(
    s: &Arc<AppState>,
    q: &str,
    kind: &str,
    offset: usize,
    limit: usize,
) -> Result<SearchResults> {
    let tags = db::extract_tags(q);
    let query = q
        .split_whitespace()
        .filter(|w| !w.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    let s2 = s.clone();
    let query2 = query.clone();
    let vec = if !query.is_empty() {
        tokio::task::spawn_blocking(move || s2.semantic.embed(&query2)).await??
    } else {
        None
    };
    let mut memos = s.db.all()?;
    memos.retain(|m| {
        (kind.is_empty()
            || kind == "all"
            || kind == m.kind
            || (kind == "pinned" && m.pinned)
            // Notes: what the user kept in their own words (Save anything, tidied notes).
            || (kind == "notes" && matches!(m.kind.as_str(), "memo" | "note")))
            && tags.iter().all(|t| m.tags.contains(t))
    });
    if !query.is_empty() {
        let ranked = db::fuse(
            s.db.lexical(&query)?,
            if let Some(v) = &vec {
                s.db.semantic(v)?
            } else {
                vec![]
            },
        );
        let ranks: HashMap<_, _> = ranked
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        memos.retain(|m| ranks.contains_key(&m.id));
        memos.sort_by_key(|m| ranks[&m.id]);
    }
    let total = memos.len();
    Ok(SearchResults {
        memos: memos
            .into_iter()
            .skip(offset)
            .take(limit.min(100))
            .collect(),
        semantic: vec.is_some(),
        total,
    })
}
async fn list_memos(
    PrivateState(s): PrivateState,
    Query(q): Query<SearchQuery>,
) -> Api<Json<SearchResults>> {
    ensure!(q.q.len() <= 500, "Search is too long");
    Ok(Json(
        search(&s, &q.q, &q.kind, q.offset, q.limit.unwrap_or(50)).await?,
    ))
}
async fn get_memo(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Memo>> {
    let mut memo =
        s.db.get(&id)?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "Memo not found".into()))?;
    // The memo page shows what its answer cost: wait, tokens, estimated cloud cost.
    memo.usage = chat::usage_for(&s.db.0.lock().unwrap(), &id)?;
    Ok(Json(memo))
}
/// Keep a thought: text, files, or both. No AI runs. Each file's readable content
/// (document text, picture or video-frame text, a local transcript) joins the memo
/// so the BM25 index, the embedding indexer, the Wiki, and later chat context all
/// see it. In a room the memo is also a chat message that carries the files.
async fn save_memo(PrivateState(s): PrivateState, Json(v): Json<Value>) -> Api<Json<Memo>> {
    let content = v["content"].as_str().unwrap_or("").to_string();
    ensure!(
        content.len() <= 100_000,
        "A memo must stay below 100,000 bytes"
    );
    let mut file_ids: Vec<String> = vec![];
    for id in v["file_ids"].as_array().into_iter().flatten() {
        if let Some(id) = id.as_str()
            && !file_ids.iter().any(|seen| seen == id)
        {
            file_ids.push(id.to_owned());
        }
    }
    ensure!(
        !content.trim().is_empty() || !file_ids.is_empty(),
        "Write something or add a file to keep"
    );
    ensure!(
        file_ids.len() <= MAX_CHAT_ATTACHMENTS,
        "Attach up to 8 photos or files per message"
    );
    let room = v["room_id"].as_str().map(str::to_owned);
    if let Some(id) = v["message_id"].as_str() {
        // A retried request with the same message GUID returns the first result.
        let room = room
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("A message requires a room"))?;
        if let Some(memo) = chat::kept_message(&s.db, room, id)? {
            return Ok(Json(memo));
        }
    }
    let title = v["title"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .map(str::to_owned);
    Ok(Json(
        keep(
            &s,
            &content,
            &file_ids,
            room.as_deref(),
            v["message_id"].as_str(),
            title.as_deref(),
            (v["clear_epoch"].as_i64(), v["data_epoch"].as_i64()),
        )
        .await?,
    ))
}
/// Keep text and files as one memo without a chat reply: a chat message when a
/// room is given, indexed, and organized into the Wiki (where its category and
/// tags and tidied text are set by the selected AI in the background). Used by "Save memo" and by a typed line the
/// chat files as a record rather than a question.
async fn keep(
    s: &Arc<AppState>,
    content: &str,
    file_ids: &[String],
    room: Option<&str>,
    message_id: Option<&str>,
    title: Option<&str>,
    (clear_epoch, data_epoch): (Option<i64>, Option<i64>),
) -> Result<Memo> {
    let content = content.to_string();
    let room = room.map(str::to_owned);
    for id in file_ids {
        s.db.require("files", id)?;
    }
    // Read every file before taking the submission lock: OCR, Quick Look frames,
    // document decoding, and local transcription all wait on the backend.
    let mut input = content.clone();
    let mut ocr_text = String::new();
    let mut transcript = String::new();
    let mut attachments = vec![];
    let budget = 120_000 / file_ids.len().max(1);
    let mut vision_tags: Vec<String> = db::extract_tags(&content);
    for id in file_ids {
        let file = s.db.file_info(id)?;
        // A video is read from disk frame by frame, never loaded whole.
        let bytes = if file.mime.starts_with("video/") {
            Vec::new()
        } else {
            s.db.file(id)?.1
        };
        // Apple Vision names what is in a picture (or a video's frame): those
        // names become the memo's hashtags and part of its text.
        let labels = media::vision_labels(s, &file).await;
        let text = media::keep_text(s, &file, bytes, &mut ocr_text, &mut transcript).await;
        let seen = if labels.is_empty() {
            String::new()
        } else {
            format!("\nSeen in the picture: {}", labels.join(", "))
        };
        // Hashtags written inside a kept document count like typed ones.
        for label in db::extract_tags(&text).into_iter().chain(labels) {
            if !vision_tags.contains(&label) && vision_tags.len() < wiki::MAX_USER_TAGS {
                vision_tags.push(label);
            }
        }
        input = format!(
            "{input}\n\nAttached file: {} (ID {})\n{}{seen}",
            file.name,
            file.id,
            media::excerpt(&text, budget)
        );
        attachments.push(file);
    }
    let input = input.trim().to_string();
    let title: String = title
        .map(str::to_owned)
        .or_else(|| {
            content
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            attachments
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .chars()
        .take(160)
        .collect();
    let media_only = content.trim().is_empty()
        && !attachments.is_empty()
        && attachments
            .iter()
            .all(|f| f.mime.starts_with("image/") || f.mime.starts_with("video/"));
    let _lock = s.chat_submit.lock().unwrap();
    chat::validate_epoch(&s.db, room.as_deref(), clear_epoch, data_epoch)?;
    // File decoding may have yielded while Delete all removed the originals.
    // Never recreate their extracted text as a new memo after erasure.
    for id in file_ids {
        s.db.require("files", id)?;
    }
    if let Some(room) = &room {
        chat::get_room(&s.db, room)?;
        let ids: Vec<&str> = file_ids.iter().map(String::as_str).collect();
        s.db.assign_files_to_room(&ids, room)?;
    }
    let m = s.db.create(&input, "memo", "local", "")?;
    // The kept files belong to this memo, so the Wiki shows them (a kept
    // picture is a photo node with its thumbnail) even without a chat message.
    if !file_ids.is_empty() {
        let placeholders = file_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&m.id];
        for id in file_ids {
            params.push(id);
        }
        s.db.0.lock().unwrap().execute(
            &format!("UPDATE files SET origin_memo=?1 WHERE origin_memo IS NULL AND id IN ({placeholders})"),
            rusqlite::params_from_iter(params),
        )?;
    }
    if let Some(room) = &room {
        s.db.assign_room("memos", &m.id, room)?;
        let visual = attachments
            .iter()
            .any(|f| f.mime.starts_with("image/") || f.mime.starts_with("video/"));
        let mut message = ChatMessage {
            roomid: room.clone(),
            messageid: message_id.map(str::to_owned).unwrap_or_else(db::id),
            role: "user".into(),
            text: content.clone(),
            memo_id: m.id.clone(),
            provider: "memo".into(),
            created_at: m.created_at.clone(),
            status: "complete".into(),
            attachments,
            ocr_text: ocr_text.trim().to_string(),
            transcript: transcript.trim().to_string(),
            // Pictures kept here are shown to a vision model in a later Image-mode question.
            input_mode: if visual {
                InputMode::Image
            } else {
                InputMode::Text
            },
            ..Default::default()
        };
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction()?;
        chat::write(&tx, &mut message)?;
        tx.commit()?;
    }
    s.db.event(&m.id, "delta", &content)?;
    s.db.finish(&m.id, "memo")?;
    s.db.update(&m.id, Some(&title), Some(&vision_tags), None)?;
    if media_only {
        // A picture or a clip kept without a word has nothing to tidy: it is
        // tagged right away (on-device, the model lane is not spent on the
        // backend's own notes about the file).
        wiki::analyze_later(s, &m.id);
    } else {
        // The chat keeps the words as written; the Wiki gets the tidied note, and
        // tags are drawn from that once it exists.
        tidy_later(s, &m.id);
    }
    Ok(s.db.get(&m.id)?.unwrap())
}
/// Tidy a kept memo in the background with the account's AI (the demo tidies
/// deterministically): the result becomes the memo's answer and the memo a
/// `note`, then the Wiki analysis runs. Failure leaves the raw memo and still
/// tags it.
fn tidy_later(s: &Arc<AppState>, memo: &str) {
    let s = s.clone();
    let memo = memo.to_string();
    tokio::spawn(async move {
        if let Err(error) = tidy(&s, &memo).await {
            tracing::warn!(%error, %memo, "Tidying a kept note failed; keeping it as written");
        }
        wiki::analyze_later(&s, &memo);
    });
}
fn tidy_config(s: &AppState) -> Result<providers::Config> {
    let config = s.provider_config()?;
    ai_preferences::allow(s, &config.provider)?;
    ensure!(
        s.demo || s.provider_configured(&config.provider),
        "No AI provider is available to tidy the note"
    );
    Ok(config)
}

async fn tidy(s: &Arc<AppState>, memo: &str) -> Result<()> {
    let Some(m) = s.db.get(memo)? else {
        return Ok(());
    };
    // `answer` holds the saved user text; `input` also contains extracted files
    // and backend scaffolding, which must not replace the user's note.
    // The kind changes to `note` once the background work has completed.
    if m.kind != "memo" || m.answer.trim().is_empty() {
        return Ok(());
    }
    let config = tidy_config(s)?;
    let tidied = if s.demo {
        demo_tidy(&m.answer)
    } else {
        let messages = vec![providers::Message::User(
            m.answer.chars().take(12_000).collect(),
        )];
        usage::stream(s, memo, &config, TIDY_SYSTEM, &messages, &[], |_| Ok(()))
            .await?
            .text
    };
    if tidied.trim().is_empty() {
        return Ok(());
    }
    {
        let conn = s.db.0.lock().unwrap();
        conn.execute(
            "UPDATE memos SET answer=?2,kind='note' WHERE id=?1 AND kind='memo'",
            rusqlite::params![memo, tidied],
        )?;
    }
    s.db.event(memo, "kind", "note")?;
    s.db.sync_wiki(memo)?;
    Ok(())
}
/// The demo's deterministic tidy: a title line and one bullet per sentence,
/// every word kept.
fn demo_tidy(text: &str) -> String {
    let text = text.replace(['<', '>'], "");
    let mut lines = vec![format!(
        "### {}",
        text.split(['.', '!', '\n']).next().unwrap_or("Note").trim()
    )];
    for sentence in text
        .split(['.', '!', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(format!("- {sentence}"));
    }
    lines.join("\n")
}
async fn update_memo(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> Api<Json<Memo>> {
    let tags = v["tags"].as_array().map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    let title = v["title"]
        .as_str()
        .map(|s| s.chars().take(160).collect::<String>());
    s.db.update(&id, title.as_deref(), None, v["pinned"].as_bool())?;
    if let Some(tags) = tags {
        // Editing tags curates the message: the list replaces inferred tags and
        // the Wiki and chat message follow in the same transaction.
        wiki::set_tags(&s, &id, &tags)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    if let Some(keep) = v["wiki"].as_bool() {
        // Keep in the Wiki, or take it out, overriding the analysis's judgement.
        wiki::set_kept(&s, &id, keep)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    get_memo(PrivateState(s), Path(id)).await
}
async fn delete_memo(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Value>> {
    let job = s.jobs.lock().unwrap().remove(&id);
    if let Some(job) = job {
        job.abort();
        let _ = job.await;
    }
    let _guard = s.chat_submit.lock().unwrap();
    s.db.delete(&id)?;
    Ok(Json(json!({"ok":true})))
}
async fn memo_image(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Response> {
    let image =
        s.db.image(&id)?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "Image not found".into()))?;
    let mime = capture::image_mime(&image).unwrap_or("application/octet-stream");
    Ok(([(header::CONTENT_TYPE, mime)], image).into_response())
}
async fn memo_thumbnail(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Response> {
    let image =
        s.db.image(&id)?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "Image not found".into()))?;
    jpeg_response(thumbnail_for(&s, &id, "capture.png", "image/png", image).await?)
}
async fn file_thumbnail(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Response> {
    let file = s.db.file_info(&id)?;
    if !file.mime.starts_with("image/") && !file.mime.starts_with("video/") {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "This file has no image preview".into(),
        ));
    }
    jpeg_response(file_thumbnail_jpeg(&s, &file).await?)
}
/// A stored file's thumbnail. A video's frame is rendered from the file on
/// disk, so even a 1 GB clip is never read into memory.
pub(crate) async fn file_thumbnail_jpeg(s: &AppState, file: &FileInfo) -> Result<Vec<u8>> {
    if file.mime.starts_with("video/") {
        let path = thumbnail_path(&s.db.1, &file.id)?;
        if let Ok(cached) = tokio::fs::read(&path).await
            && !cached.is_empty()
        {
            return Ok(cached);
        }
        let jpeg = capture::thumbnail(media::video_frame(s, file).await?).await?;
        cache_thumbnail(s, &file.id, &jpeg)?;
        return Ok(jpeg);
    }
    let (f, bytes) = s.db.file(&file.id)?;
    thumbnail_for(s, &f.id, &f.name, &f.mime, bytes).await
}
/// Where a thumbnail is kept once made: `thumbnails/<memo or file id>.jpg`.
fn thumbnail_path(root: &std::path::Path, key: &str) -> Result<std::path::PathBuf> {
    artifacts::resolve(root, &format!("thumbnails/{key}.jpg"))
}
/// Drop a cached thumbnail when its memo image or file goes.
pub(crate) fn forget_thumbnail(root: &std::path::Path, key: &str) {
    if let Ok(path) = thumbnail_path(root, key) {
        let _ = std::fs::remove_file(path);
    }
}
/// The JPEG thumbnail of a picture or a video, made once and cached on disk.
/// A video gives one frame; a picture the image decoder cannot read (HEIC, RAW)
/// is rendered by QuickLook instead, so iPhone photos preview too.
pub(crate) async fn thumbnail_for(
    s: &AppState,
    key: &str,
    name: &str,
    mime: &str,
    bytes: Vec<u8>,
) -> Result<Vec<u8>> {
    let path = thumbnail_path(&s.db.1, key)?;
    if let Ok(cached) = tokio::fs::read(&path).await
        && !cached.is_empty()
    {
        return Ok(cached);
    }
    let jpeg = if mime.starts_with("video/") {
        let frame = media::video_thumbnail(name, &bytes).await?;
        capture::thumbnail(frame).await?
    } else {
        match capture::thumbnail(bytes.clone()).await {
            Ok(jpeg) => jpeg,
            Err(_) => {
                let rendered = media::video_thumbnail(name, &bytes)
                    .await
                    .map_err(|_| anyhow::anyhow!("Cannot preview this image"))?;
                capture::thumbnail(rendered).await?
            }
        }
    };
    cache_thumbnail(s, key, &jpeg)?;
    Ok(jpeg)
}
/// Commit a derived image under the same database lock as erasure. Rendering
/// can finish after its source is deleted, but it must never recreate the cache.
fn cache_thumbnail(s: &AppState, key: &str, jpeg: &[u8]) -> Result<()> {
    let conn = s.db.0.lock().unwrap();
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM (SELECT id,owner_wallet FROM memos UNION ALL SELECT id,owner_wallet FROM files) AS source WHERE id=?1 AND (?2 IS NULL OR owner_wallet=?2) AND NOT EXISTS(SELECT 1 FROM user_purges WHERE owner_wallet=source.owner_wallet))",
        rusqlite::params![key, s.db.2],
        |r| r.get(0),
    )?;
    if !exists {
        // Caching is optional. Never recreate erased bytes or fail a rendered preview.
        return Ok(());
    }
    artifacts::write(&thumbnail_path(&s.db.1, key)?, jpeg)
}
/// Make a file's thumbnail ahead of time, so grids open without waiting.
fn warm_thumbnail_later(s: &Arc<AppState>, file: &FileInfo) {
    if !(file.mime.starts_with("image/") || file.mime.starts_with("video/")) {
        return;
    }
    let s = s.clone();
    let file = file.clone();
    tokio::spawn(async move {
        if let Err(error) = file_thumbnail_jpeg(&s, &file).await {
            tracing::debug!(%error, file = %file.id, "No thumbnail for this file");
        }
    });
}
async fn file_image(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Response> {
    let (_, image) = s.db.file(&id)?;
    let mime = capture::image_mime(&image).ok_or(ApiError(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "Not a supported image".into(),
    ))?;
    Ok(([(header::CONTENT_TYPE, mime)], image).into_response())
}
fn jpeg_response(jpeg: Vec<u8>) -> Api<Response> {
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg".to_string()),
            (header::CACHE_CONTROL, "private, max-age=86400".to_string()),
        ],
        jpeg,
    )
        .into_response())
}
async fn export_memo(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Response> {
    let m =
        s.db.get(&id)?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "Memo not found".into()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"causewaybay-memo.md\"",
            ),
        ],
        format!(
            "# {}\n\n{}\n\n## Answer\n\n{}\n\n{}",
            m.title,
            m.input,
            m.answer,
            m.tags
                .iter()
                .map(|t| format!("#{t}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    )
        .into_response())
}
#[derive(Deserialize, Default)]
struct Cursor {
    #[serde(default)]
    after: i64,
}
async fn events(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Query(q): Query<Cursor>,
    headers: HeaderMap,
) -> Api<impl IntoResponse> {
    if s.db.get(&id)?.is_none() {
        return Err(ApiError(StatusCode::NOT_FOUND, "Memo not found".into()));
    }
    let mut cursor = headers
        .get("last-event-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(q.after);
    let stream = async_stream::stream! {
        yield Ok::<_,Infallible>(Event::default().retry(Duration::from_secs(2)).comment("callbacks ready"));
        loop{
            if auth::authorize(&s,&headers,"GET","/api/memos").is_err(){break;}
            let events=match s.db.events(&id,cursor){Ok(v)=>v,Err(_)=>break};
            let full_batch=events.len()==256;
            for e in events{cursor=e.id;let terminal=e.kind=="done"||e.kind=="error";yield Ok::<_,Infallible>(Event::default().id(e.id.to_string()).json_data(&e).unwrap());if terminal{return;}}
            if !full_batch&&s.db.get(&id).ok().flatten().is_none_or(|m|m.status!="running"){break;}
            if !full_batch{tokio::time::sleep(Duration::from_millis(120)).await;}
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}
async fn cancel(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Value>> {
    if let Some(job) = s.jobs.lock().unwrap().remove(&id) {
        job.abort();
        s.db.fail(
            &id,
            "Stopped by you. Your partial answer and files were saved.",
        )?;
    }
    Ok(Json(json!({"ok":true})))
}
async fn files(PrivateState(s): PrivateState) -> Api<Json<Vec<FileInfo>>> {
    Ok(Json(s.db.files()?))
}
async fn upload(PrivateState(s): PrivateState, Json(v): Json<Value>) -> Api<Json<FileInfo>> {
    use base64::Engine;
    let name = media::upload_name(v["name"].as_str().unwrap_or("upload.txt"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(v["data_base64"].as_str().unwrap_or(""))
        .map_err(|_| anyhow::anyhow!("Invalid base64 file data"))?;
    ensure!(
        bytes.len() <= 25 * 1024 * 1024,
        "Upload must be below 25 MB"
    );
    let mime = media::mime(&name, &bytes);
    let _lock = s.chat_submit.lock().unwrap();
    chat::validate_epoch(
        &s.db,
        v["room_id"].as_str(),
        v["clear_epoch"].as_i64(),
        v["data_epoch"].as_i64(),
    )?;
    if let Some(room) = v["room_id"].as_str() {
        chat::get_room(&s.db, room)?;
    }
    let file = s.db.put_file(&name, mime, &bytes)?;
    if let Some(room) = v["room_id"].as_str() {
        s.db.assign_room("files", &file.id, room)?;
    }
    drop(_lock);
    after_upload(&s, &file, v["wiki"] == true && v["room_id"].is_null());
    Ok(Json(file))
}

/// Work that follows any stored upload: an early thumbnail and, for a picture
/// or clip added on the Images or Videos page, a memo that files it in the
/// Wiki as a photo or video.
pub(crate) fn after_upload(s: &Arc<AppState>, file: &FileInfo, wiki: bool) {
    warm_thumbnail_later(s, file);
    if wiki {
        let s2 = s.clone();
        let kept = file.clone();
        tokio::spawn(async move {
            if let Err(error) = keep(
                &s2,
                "",
                std::slice::from_ref(&kept.id),
                None,
                None,
                Some(&kept.name),
                (None, None),
            )
            .await
            {
                tracing::warn!(%error, file = %kept.id, "Could not file the upload in the Wiki");
            }
        });
    }
}

async fn download(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    request: Request,
) -> Api<Response> {
    let f = s.db.file_info(&id)?;
    let name: String = f
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let disposition = format!("attachment; filename=\"{name}\"");
    // A text file is served from SQLite; binary media streams from disk.
    if f.path.is_empty() {
        let (_, bytes) = s.db.file(&id)?;
        return library::serve_bytes(bytes, &f.mime, &disposition, None);
    }
    let (_, path) = s.db.file_location(&id)?;
    library::serve_from_disk(&path, request, &f.mime, &disposition, None).await
}
async fn file_text(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Value>> {
    let (f, b) = s.db.file(&id)?;
    ensure!(b.len() <= 500_000, "File is too large to preview as text");
    Ok(Json(
        json!({"file":f,"text":String::from_utf8(b).map_err(|_|anyhow::anyhow!("This file is binary"))?,"compilation":s.db.last_compilation(&id)?}),
    ))
}
async fn compile_file(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Value>> {
    let _permit = s.slots.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "The workspace is busy. Try again shortly.".into(),
        )
    })?;
    let (_, b) = s.db.file(&id)?;
    let source = String::from_utf8(b).map_err(|_| anyhow::anyhow!("Choose a Rust text file"))?;
    let result = agent::compile(&s.data_dir, &source, None).await?;
    s.db.save_compilation(&id, &result)?;
    Ok(Json(result))
}

#[cfg(test)]
mod context_tests;

fn seed(s: &Arc<AppState>) -> Result<()> {
    for (title, kind, tags, answer) in [
        (
            "A thoughtful reply to the team",
            "reply",
            "#work #communication",
            "Thanks for the thoughtful feedback. I’ll take another pass at the proposal and share an updated version tomorrow. Let’s keep the scope focused on what we can deliver well.",
        ),
        (
            "Making sense of Rust ownership",
            "code",
            "#rust #learning",
            "Ownership gives each value one owner. Borrow with `&T` when you only need to read, or `&mut T` when you need to change it.\n\n```rust\nfn greet(name: &str) {\n    println!(\"Hello, {name}!\");\n}\n```",
        ),
        (
            "Small ideas, worth keeping",
            "memo",
            "#ideas",
            "A place for the useful things you find along the way. Capture a screen, ask a question, or save a thought. Everything comes back to your library.",
        ),
    ] {
        let m =
            s.db.create(&format!("{title}\n{tags}"), kind, "demo", "fixture")?;
        s.db.update(&m.id, Some(title), None, None)?;
        s.db.event(&m.id, "delta", answer)?;
        s.db.finish(&m.id, kind)?;
    }
    Ok(())
}

#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod session_tests;

#[cfg(test)]
mod erasure_tests {
    use super::*;

    #[tokio::test]
    async fn tidy_uses_only_saved_user_text_and_honors_ai_policy() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?
            .for_owner("local");
        for (mode, expected) in [
            ("ondevice", "apple"),
            ("hybrid", "apple"),
            ("cloudai", "grok"),
        ] {
            s.db.set_setting(
                "user.ai.local",
                &json!({"mode":mode,"local_provider":"apple","cloud_provider":"grok"}).to_string(),
            )?;
            assert_eq!(tidy_config(&s)?.provider, expected);
        }
        s.db.set_setting(
            "local_ai",
            &serde_json::to_string(&LocalAiSettings {
                local_only: true,
                apple_enabled: true,
                ..Default::default()
            })?,
        )?;
        assert_eq!(tidy_config(&s)?.provider, "apple");
        let m = s.db.create("Meeting notes\nAttached file: secret.docx (ID private-id)\nSecret document excerpt\nSeen in the picture: private labels", "memo", "local", "")?;
        s.db.event(&m.id, "delta", "Meeting notes")?;
        tidy(&s, &m.id).await?;
        let m = s.db.get(&m.id)?.unwrap();
        assert_eq!(m.answer, "### Meeting notes\n- Meeting notes");
        let empty = s.db.create(
            "Attached file: secret.pdf (ID private-id)",
            "memo",
            "local",
            "",
        )?;
        tidy(&s, &empty.id).await?;
        assert!(s.db.get(&empty.id)?.unwrap().answer.is_empty());
        Ok(())
    }

    #[test]
    fn thumbnail_cache_uses_source_owner_and_skips_pending_purges() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
        let alice = s.for_owner("alice");
        let file = alice.db.put_file("private.png", "image/png", b"image")?;
        cache_thumbnail(&s, &file.id, b"thumbnail")?;
        let path = thumbnail_path(dir.path(), &file.id)?;
        assert!(path.exists());
        std::fs::remove_file(&path)?;
        cache_thumbnail(&s.for_owner("bob"), &file.id, b"wrong owner")?;
        assert!(!path.exists());
        s.db.0
            .lock()
            .unwrap()
            .execute("INSERT INTO user_purges(owner_wallet) VALUES('alice')", [])?;
        cache_thumbnail(&s, &file.id, b"late thumbnail")?;
        cache_thumbnail(&alice, &file.id, b"late thumbnail")?;
        assert!(!path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn completed_thumbnail_cannot_recreate_erased_content() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?
            .for_owner("local");
        let file =
            s.db.put_file("private.png", "image/png", b"synthetic image")?;
        cache_thumbnail(&s, &file.id, b"synthetic thumbnail")?;
        let path = thumbnail_path(dir.path(), &file.id)?;
        assert!(path.exists());
        let _ = chat::delete_all(PrivateState(s.clone()))
            .await
            .map_err(|e| anyhow::anyhow!(e.1))?;
        // Model the commit from a render that started before Delete all.
        cache_thumbnail(&s, &file.id, b"late thumbnail")?;
        assert!(!path.exists());
        Ok(())
    }
}
