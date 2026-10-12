use serde::{Deserialize, Serialize};
pub mod chat;
pub mod invisible;
pub mod quiz;
pub mod solved;
pub mod status;
pub use chat::*;
pub use invisible::*;
pub use status::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBatchRequest {
    #[serde(default)]
    pub client_ids: Vec<String>,
    #[serde(default)]
    pub all: bool,
    pub request: ThinkRequest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureBatchItem {
    pub client: CaptureClient,
    pub memo: Option<Memo>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Memo {
    pub id: String,
    pub title: String,
    pub input: String,
    pub answer: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub created_at: String,
    pub pinned: bool,
    pub has_image: bool,
    pub error: Option<String>,
    /// Measured model calls behind this memo: wait, tokens, and estimated cloud cost.
    #[serde(default)]
    pub usage: Option<crate::MessageUsage>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub model: String,
    pub configured: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub input_mode: String,
    pub provider: String,
    pub providers: Vec<Provider>,
    pub capture_mode: String,
    pub display: u32,
    pub draft: String,
    pub semantic_status: String,
    pub demo: bool,
    #[serde(default)]
    pub local_ai: LocalAiSettings,
    #[serde(default)]
    pub user_ai: Option<UserAiPreferences>,
    #[serde(default)]
    pub user_models: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub default_ai: Option<UserAiPreferences>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UserAiPreferences {
    /// ondevice, hybrid, cloudai, codex, or claude_code. Hybrid escalates to cloud AI
    /// when the local model asks for help; `AssistancePreferences::hybrid_confirm`
    /// makes it ask first. codex and claude_code run those CLIs on the backend.
    pub mode: String,
    pub local_provider: String,
    pub cloud_provider: String,
}
/// What a Wiki node is, first among its tags: a question with its answer, a
/// tidied note, source code, a photo or a video. The chat keeps the raw
/// material; the Wiki keeps these organized forms.
pub const TYPES: [&str; 5] = ["q&a", "note", "code", "photo", "video"];
pub fn is_type(tag: &str) -> bool {
    TYPES.contains(&tag)
}
/// What a note is about, second among its tags (q&a entries carry no category).
pub const CATEGORIES: [&str; 5] = ["personal", "information", "idea", "task", "other"];
pub fn is_category(tag: &str) -> bool {
    CATEGORIES.contains(&tag)
}
/// A type or a category: the fixed, non-editable part of a message's tags.
pub fn is_fixed_tag(tag: &str) -> bool {
    is_type(tag) || is_category(tag)
}
/// Providers that run the user's own command-line tools on the backend Mac,
/// billed to those subscriptions rather than an API key.
pub fn cli_provider(id: &str) -> bool {
    matches!(id, "codex" | "claude_code")
}
/// Providers that take text only: local models and the CLI tools get OCR text,
/// never a screenshot image.
pub fn text_only_provider(id: &str) -> bool {
    matches!(id, "apple" | "mlx") || cli_provider(id)
}
impl UserAiPreferences {
    pub fn provider(&self) -> &str {
        match self.mode.as_str() {
            "cloudai" => &self.cloud_provider,
            "codex" => "codex",
            "claude_code" => "claude_code",
            _ => &self.local_provider,
        }
    }
    /// The policy used when neither the wallet nor the admin saved one. The backend
    /// and the browser call this same function so they never disagree about it.
    /// Hybrid is the default whenever a local and a cloud provider are both ready:
    /// answers start on the backend Mac and cloud help needs a one-time approval.
    pub fn computed_default(
        local_only: bool,
        selected: &str,
        providers: &[(String, bool)],
    ) -> Self {
        let ready = |id: &str| providers.iter().any(|(p, ok)| p == id && *ok);
        let first = |local: bool| {
            providers
                .iter()
                .find(|(p, ok)| *ok && (ai_location(p) == "local") == local)
                .map(|(p, _)| p.clone())
        };
        let selected_local = ai_location(selected) == "local";
        let local_provider = if selected_local && ready(selected) {
            Some(selected.to_owned())
        } else {
            first(true)
        };
        let cloud_provider = if !selected_local && ready(selected) {
            Some(selected.to_owned())
        } else {
            first(false)
        };
        // Hybrid is the default whenever a local model is ready: it polls and answers
        // on-device and asks a cloud provider only when necessary.
        let mode = if local_only {
            "ondevice"
        } else if local_provider.is_some() || selected_local {
            "hybrid"
        } else {
            "cloudai"
        };
        Self {
            mode: mode.into(),
            local_provider: local_provider.unwrap_or_else(|| {
                if selected_local {
                    selected.into()
                } else {
                    "apple".into()
                }
            }),
            cloud_provider: cloud_provider.unwrap_or_else(|| {
                if selected_local {
                    "grok".into()
                } else {
                    selected.into()
                }
            }),
        }
    }
}

/// Personal assistance settings; stored by the backend for the signed-in wallet.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AssistancePreferences {
    pub wiki_enabled: bool,
    pub wiki_auto_update: bool,
    pub wiki_group_by: String,
    pub wiki_include_answers: bool,
    pub textedit_enabled: bool,
    pub textedit_settle_ms: u64,
    pub textedit_style: String,
    /// Hybrid: ask before each cloud request instead of escalating automatically.
    #[serde(default)]
    pub hybrid_confirm: bool,
    /// Cloud AI, Codex and Claude Code: let the local model decide first whether a
    /// changed note actually asks for help, so unchanged or idle notes cost nothing.
    #[serde(default = "enabled")]
    pub local_gate: bool,
}
fn enabled() -> bool {
    true
}
impl Default for AssistancePreferences {
    fn default() -> Self {
        Self {
            // The Wiki is automatic: every completed capture, note answer, terminal
            // answer and remote request is organized without any setup.
            wiki_enabled: true,
            wiki_auto_update: true,
            wiki_group_by: "topic".into(),
            wiki_include_answers: true,
            textedit_enabled: true,
            textedit_settle_ms: 1200,
            textedit_style: "concise".into(),
            hybrid_confirm: false,
            local_gate: true,
        }
    }
}

/// Admin-owned configuration. Executable paths and credentials are never client inputs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LocalAiSettings {
    pub local_only: bool,
    pub apple_enabled: bool,
    pub mlx_enabled: bool,
    /// `builtin`: the backend's own Rust MLX worker runs an installed model
    /// (see `LOCAL_MODELS`). `service`: connect to an OpenAI-compatible MLX
    /// service already running on the backend Mac at `mlx_url`.
    #[serde(default = "mlx_engine_default")]
    pub mlx_engine: String,
    pub mlx_url: String,
    pub speech_provider: String,
    pub speech_locale: String,
    pub mlx_speech_model: String,
    /// Polling interval for Cloud AI mode, where each change may cost a cloud call.
    pub auto_interval_seconds: u64,
    /// Polling interval for modes that poll with the on-device model (On-device,
    /// Hybrid, Codex, Claude Code); polling costs nothing, so it is short.
    #[serde(default = "local_interval_default")]
    pub local_interval_seconds: u64,
    /// Floor for every poll in Cloud AI mode (autonomous run and capture polling),
    /// because each poll there can cost a cloud call. Default one minute.
    #[serde(default = "cloud_min_interval_default")]
    pub cloud_min_interval_seconds: u64,
}
fn local_interval_default() -> u64 {
    5
}
fn mlx_engine_default() -> String {
    "builtin".into()
}
fn cloud_min_interval_default() -> u64 {
    60
}
impl Default for LocalAiSettings {
    fn default() -> Self {
        Self {
            local_only: false,
            apple_enabled: false,
            mlx_enabled: false,
            mlx_engine: "builtin".into(),
            mlx_url: "http://127.0.0.1:8080/v1".into(),
            speech_provider: "auto".into(),
            speech_locale: "en-US".into(),
            mlx_speech_model: String::new(),
            auto_interval_seconds: 60,
            local_interval_seconds: 5,
            cloud_min_interval_seconds: 60,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsUpdate {
    pub default_ai: Option<UserAiPreferences>,
    pub local_ai: Option<LocalAiSettings>,
    pub input_mode: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub capture_mode: Option<String>,
    pub display: Option<u32>,
    pub draft: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkRequest {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub capture_mode: Option<String>,
    pub display: Option<u32>,
    pub client_id: Option<String>,
    #[serde(default)]
    pub input_mode: InputMode,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub capture: bool,
    #[serde(default)]
    pub agent: bool,
    /// Answer even if the text reads like a record: set by "Ask AI instead".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ask: bool,
    /// Set by the backend when a line is filed as a record: the AI tidies it
    /// into a clean note instead of answering it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub note: bool,
    /// Ask: answer from the whole library (BM25 and semantic retrieval feed the
    /// AI its context) rather than in a room. Saved as a memo of kind `ask`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub library: bool,
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_ids: Vec<String>,
    pub parent_id: Option<String>,
    pub room_id: Option<String>,
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_epoch: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_epoch: Option<i64>,
}

/// Per-message attachment limit shared by the browser and backend.
pub const MAX_CHAT_ATTACHMENTS: usize = 8;
impl ThinkRequest {
    /// Keep legacy single-file requests valid, preserving the user's attachment order.
    pub fn attachment_ids(&self) -> Vec<&str> {
        let mut ids = vec![];
        for id in self.file_id.iter().chain(self.file_ids.iter()) {
            if !ids.contains(&id.as_str()) {
                ids.push(id.as_str());
            }
        }
        ids
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    #[default]
    Text,
    Image,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub id: i64,
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FileInfo {
    #[serde(default)]
    pub path: String,
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SearchResults {
    pub memos: Vec<Memo>,
    pub semantic: bool,
    pub total: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CaptureClient {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub online: bool,
    pub last_seen: i64,
    #[serde(default)]
    pub owner_wallet: Option<String>,
    #[serde(default)]
    pub can_capture: bool,
    /// Account permission and live CLI capability, independent of AI policy.
    #[serde(default)]
    pub textedit: TextEditAvailability,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextEditAvailability {
    #[default]
    Unavailable,
    NeedsEnrollment,
    Offline,
    UpdateRequired,
    Ready,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BackendAddress {
    pub label: String,
    pub web_url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Dashboard {
    #[serde(default)]
    pub addresses: Vec<BackendAddress>,
    pub rust_clients: Vec<CaptureClient>,
    pub web_clients: Vec<CaptureClient>,
    pub rust_connected: usize,
    pub web_connected: usize,
    pub uptime_seconds: u64,
    pub server_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureCommand {
    #[serde(rename = "type")]
    pub kind: String,
    pub request_id: String,
    pub mode: String,
    pub display: u32,
}

#[cfg(not(target_arch = "wasm32"))]
pub mod config {
    use serde_json::Value;
    use std::{
        io::{self, Read, Seek, SeekFrom, Write},
        path::{Path, PathBuf},
    };
    /// Private credentials never go into the exported JSONL preferences.
    pub fn secret(dir: &Path, name: &str) -> io::Result<String> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = dir.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                let mut bytes = [0u8; 32];
                std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
                let value = hex::encode(bytes);
                file.write_all(value.as_bytes())?;
                file.sync_all()?;
                Ok(value)
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                    return Err(io::Error::other(
                        "Credential file must not be a symbolic link",
                    ));
                }
                let value = std::fs::read_to_string(path)?;
                if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(io::Error::other("Invalid private credential file"));
                }
                Ok(value)
            }
            Err(e) => Err(e),
        }
    }
    pub fn directory() -> io::Result<PathBuf> {
        if let Some(dir) = std::env::var_os("CAUSEWAYBAY_CONFIG_DIR").filter(|v| !v.is_empty()) {
            return Ok(dir.into());
        }
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(|dir| PathBuf::from(dir).join(".causewaybaybot"))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "HOME is unavailable; cannot locate ~/.causewaybaybot",
                )
            })
    }
    pub fn client_directory() -> PathBuf {
        std::env::var_os("CAUSEWAYBAY_CLIENT_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| ".".into())
                    .join(".causewaybaybotclient")
            })
    }
    /// Last complete JSONL snapshot wins. An interrupted final append is ignored.
    pub fn read(dir: &Path, name: &str) -> io::Result<Value> {
        match std::fs::read_to_string(dir.join(format!("{name}.jsonl"))) {
            Ok(text) => Ok(text
                .lines()
                .rev()
                .find_map(|line| serde_json::from_str::<Value>(line).ok())
                .and_then(|v| v.get("config").cloned())
                .unwrap_or(Value::Null)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Value::Null),
            Err(e) => Err(e),
        }
    }
    pub fn append(dir: &Path, name: &str, config: Value) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = dir.join(format!("{name}.jsonl"));
        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        // Isolate an interrupted append without inserting blank JSONL records.
        if file.metadata()?.len() > 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                writeln!(file)?;
            }
        }
        let record = serde_json::json!({"version":1,"updated_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),"config":config});
        writeln!(file, "{record}")?;
        file.sync_data()
    }
}
pub mod identity;

#[cfg(test)]
mod policy_tests {
    use super::UserAiPreferences;
    fn providers(ready: &[&str]) -> Vec<(String, bool)> {
        ["anthropic", "openai", "grok", "gemini", "apple", "mlx"]
            .iter()
            .map(|p| (p.to_string(), ready.contains(p)))
            .collect()
    }
    #[test]
    fn hybrid_is_the_default_whenever_a_local_model_is_ready() {
        let both = UserAiPreferences::computed_default(false, "grok", &providers(&["mlx", "grok"]));
        assert_eq!(
            (
                both.mode.as_str(),
                both.local_provider.as_str(),
                both.cloud_provider.as_str()
            ),
            ("hybrid", "mlx", "grok")
        );
        let preferred = UserAiPreferences::computed_default(
            false,
            "apple",
            &providers(&["mlx", "apple", "openai", "grok"]),
        );
        assert_eq!(
            (
                preferred.mode.as_str(),
                preferred.local_provider.as_str(),
                preferred.cloud_provider.as_str()
            ),
            ("hybrid", "apple", "openai")
        );
        let restricted =
            UserAiPreferences::computed_default(true, "grok", &providers(&["mlx", "grok"]));
        assert_eq!(
            (restricted.mode.as_str(), restricted.local_provider.as_str()),
            ("ondevice", "mlx")
        );
        let local_only_ready =
            UserAiPreferences::computed_default(false, "apple", &providers(&["apple"]));
        // A ready local model means Hybrid even before any cloud key exists: the
        // worker answers locally and escalation falls back on-device until one is set.
        assert_eq!(
            (
                local_only_ready.mode.as_str(),
                local_only_ready.cloud_provider.as_str()
            ),
            ("hybrid", "grok")
        );
        let cloud_only = UserAiPreferences::computed_default(false, "grok", &providers(&["grok"]));
        assert_eq!(
            (
                cloud_only.mode.as_str(),
                cloud_only.local_provider.as_str(),
                cloud_only.cloud_provider.as_str()
            ),
            ("cloudai", "apple", "grok")
        );
        let nothing = UserAiPreferences::computed_default(false, "anthropic", &providers(&[]));
        assert_eq!(
            (nothing.mode.as_str(), nothing.cloud_provider.as_str()),
            ("cloudai", "anthropic")
        );
    }
}

/// One on-device language model offered in Backend settings. Apple's system
/// model is part of macOS; the Qwen entries are 4-bit MLX builds that the
/// backend downloads from Hugging Face on request and runs with its own Rust
/// MLX worker. Sizes and peak memory are measured on an M5 Max.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalModelSpec {
    pub id: &'static str,
    pub provider: &'static str,
    pub model: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    pub size_gb: f64,
    pub memory_gb: f64,
}
pub const LOCAL_MODELS: [LocalModelSpec; 4] = [
    LocalModelSpec {
        id: "apple",
        provider: "apple",
        model: "system",
        name: "Apple LLM",
        note: "Apple Intelligence on-device model, no download",
        size_gb: 0.0,
        memory_gb: 0.0,
    },
    LocalModelSpec {
        id: "qwen3.5-9b",
        provider: "mlx",
        model: "mlx-community/Qwen3.5-9B-4bit",
        name: "Qwen3.5 9B",
        note: "default",
        size_gb: 6.0,
        memory_gb: 5.2,
    },
    LocalModelSpec {
        id: "qwen3.5-4b",
        provider: "mlx",
        model: "mlx-community/Qwen3.5-4B-4bit",
        name: "Qwen3.5 4B",
        note: "fastest, least memory",
        size_gb: 2.5,
        memory_gb: 2.5,
    },
    LocalModelSpec {
        id: "qwen3.8-27b",
        provider: "mlx",
        model: "mlx-community/Qwen3.8-27B-4bit",
        name: "Qwen3.8 27B",
        note: "strongest, slowest",
        size_gb: 16.0,
        memory_gb: 15.4,
    },
];
pub fn local_model_spec(id: &str) -> Option<&'static LocalModelSpec> {
    LOCAL_MODELS.iter().find(|m| m.id == id)
}
pub fn local_model_by_repo(provider: &str, model: &str) -> Option<&'static LocalModelSpec> {
    LOCAL_MODELS
        .iter()
        .find(|m| m.provider == provider && m.model == model)
}

/// One step of preparing an answer, shown to the user as it happens.
/// `fraction` is set for measurable work (a download), absent while waiting or loading.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelProgress {
    pub label: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub fraction: Option<f64>,
}
/// Progress of the one model download the backend runs at a time.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelDownload {
    pub id: String,
    pub model: String,
    pub file: String,
    pub done_files: usize,
    pub total_files: usize,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
    pub finished: bool,
    pub cancelled: bool,
    #[serde(default)]
    pub error: Option<String>,
}
impl ModelDownload {
    pub fn fraction(&self) -> f64 {
        if self.finished && self.error.is_none() {
            1.0
        } else if self.total_bytes == 0 {
            0.0
        } else {
            (self.done_bytes as f64 / self.total_bytes as f64).clamp(0.0, 1.0)
        }
    }
    pub fn active(&self) -> bool {
        !self.finished && !self.cancelled && self.error.is_none()
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LocalModel {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub name: String,
    pub note: String,
    pub size_gb: f64,
    pub memory_gb: f64,
    pub installed: bool,
    /// `fits`, `tight`, `too_large`, or `unknown` when memory could not be read.
    pub fit: String,
    #[serde(default)]
    pub download: Option<ModelDownload>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LocalModels {
    /// Apple Silicon unified memory, shared by the CPU and GPU.
    pub unified_memory_bytes: Option<u64>,
    /// The part of it a model may use: what Metal recommends as a working set.
    pub gpu_budget_bytes: Option<u64>,
    pub worker_available: bool,
    pub apple_available: bool,
    /// The MLX model the backend's single-lane worker currently holds in memory.
    #[serde(default)]
    pub loaded_model: Option<String>,
    /// Whether that worker is answering right now, and how many requests wait behind it.
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub waiting: usize,
    pub models: Vec<LocalModel>,
}

/// One line of the backend setup check the admin sees after saving settings:
/// what was checked, whether it passed, and what to do if not.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SetupCheck {
    pub id: String,
    pub label: String,
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SetupReport {
    pub ok: bool,
    pub checks: Vec<SetupCheck>,
    pub checked_at: String,
}
impl SetupReport {
    pub fn problems(&self) -> usize {
        self.checks.iter().filter(|c| !c.ok).count()
    }
}
