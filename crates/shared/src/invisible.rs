//! The Rust backend and Rust/WASM web UI share the same session contract.
//! These are data structures only; scheduling and inference belong to the backend.
use serde::{Deserialize, Serialize};

/// Safe, actionable failures shared by the native client and session worker.
/// Never carry script output or private note contents in an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteError {
    NoteChanged,
    DocumentChanged,
    NoDocument,
    PermissionDenied,
    Expired,
    TimedOut,
    OperationFailed,
}

impl std::fmt::Display for NoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoteChanged => "The note changed while AI was answering. Your edits were kept.",
            Self::DocumentChanged => "The selected TextEdit document changed. Select the note you want and start Auto again.",
            Self::NoDocument => "Open a note in TextEdit on the capture Mac, then start Auto again.",
            Self::PermissionDenied => "Allow the app that started the capture CLI to control TextEdit in System Settings → Privacy & Security → Automation on the capture Mac, then start Auto again.",
            Self::Expired => "This note response expired. Start Auto again to retry.",
            Self::TimedOut => "TextEdit did not respond in time. Close any open dialog in TextEdit on the capture Mac, then start Auto again.",
            Self::OperationFailed => "TextEdit could not read or update this note. Check that the selected document is editable and close any open dialog on the capture Mac, then start Auto again.",
        })
    }
}

impl std::error::Error for NoteError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudApproval {
    pub id: String,
    pub reason: String,
    pub provider: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvisibleStatus {
    pub active: bool,
    pub textedit_enabled: bool,
    pub client_id: String,
    pub document: String,
    pub stage: String,
    pub error: Option<String>,
    pub approval: Option<CloudApproval>,
    /// Provider id and model the running worker will use for its next answer.
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// Seconds between note checks for this session; 0 means the admin default.
    #[serde(default)]
    pub interval_seconds: u64,
    /// Which Mac app the run watches: auto (follow the front app), textedit, notes,
    /// or none (remote prompts only).
    #[serde(default)]
    pub app: String,
    /// The app actually being read right now (textedit or notes), resolved by the CLI.
    #[serde(default)]
    pub resolved_app: String,
    /// Read the note as displayed: a screenshot of the capture Mac accompanies each
    /// new note or request (OCR locally for local models, the image for cloud AI).
    #[serde(default = "yes")]
    pub read_screen: bool,
    /// Remote prompts from the web UI waiting for the worker.
    #[serde(default)]
    pub queued_prompts: u64,
    /// The newest remote prompt and what the backend did with it, for the session card.
    #[serde(default)]
    pub last_prompt: String,
    #[serde(default)]
    pub last_reply: String,
    /// Fixed-form summary of the Mac action taken, e.g. the URL opened in Safari.
    #[serde(default)]
    pub last_action: String,
}

fn yes() -> bool {
    true
}
impl Default for InvisibleStatus {
    /// Screen reading is on for new sessions; everything else starts empty.
    fn default() -> Self {
        Self {
            active: false,
            textedit_enabled: false,
            client_id: String::new(),
            document: String::new(),
            stage: String::new(),
            error: None,
            approval: None,
            provider: String::new(),
            model: String::new(),
            interval_seconds: 0,
            app: String::new(),
            resolved_app: String::new(),
            read_screen: true,
            queued_prompts: 0,
            last_prompt: String::new(),
            last_reply: String::new(),
            last_action: String::new(),
        }
    }
}

/// Watched note applications on the capture Mac. `auto` follows whichever of
/// TextEdit or Notes is in front; `none` keeps only remote prompts.
pub const NOTE_APPS: [&str; 5] = ["auto", "textedit", "notes", "terminal", "none"];

/// Normalize a stored or requested app id; empty or unknown means automatic detection.
pub fn note_app(app: &str) -> &str {
    if NOTE_APPS.contains(&app) {
        app
    } else {
        "auto"
    }
}

/// The concrete app behind a snapshot: the CLI's `app`, else the document identity.
pub fn concrete_app(app: Option<&str>, document: &str) -> &'static str {
    match app {
        Some("notes") => "notes",
        Some("textedit") => "textedit",
        Some("terminal") => "terminal",
        _ if document.starts_with("notes:") => "notes",
        _ if document.starts_with("terminal:") || document.starts_with("iterm:") => "terminal",
        _ => "textedit",
    }
}

/// The part of a terminal the local model reads: the newest lines after the last
/// answer, without the final line, which is the prompt still being typed on.
pub fn terminal_tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let entered = &lines[..lines.len().saturating_sub(1)];
    let tail = &entered[entered.len().saturating_sub(40)..];
    let joined = tail.join("\n");
    if joined.trim().is_empty() {
        String::new()
    } else {
        joined
    }
}

/// Demo-mode stand-in for the local model's judgement of a terminal: the newest
/// entered line that starts, after the shell prompt, with `ai-`, `ai:`, `ai,`,
/// `ai ,`, plain `ai ` or `invisible ai` (any case). Real runs ask the local
/// model instead of matching patterns.
pub fn terminal_request(text: &str) -> Option<String> {
    const TRIGGERS: [&str; 14] = [
        "invisible ai, ",
        "invisible ai: ",
        "invisible ai- ",
        "invisible ai ",
        "invisible ai,",
        "ai- ",
        "ai-",
        "ai: ",
        "ai:",
        "ai , ",
        "ai ,",
        "ai, ",
        "ai,",
        "ai ",
    ];
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate().rev() {
        if i + 1 >= lines.len() {
            continue;
        }
        let trimmed = line.trim_start();
        let lower = trimmed.to_lowercase();
        if lower.len() != trimmed.len() {
            continue;
        }
        for trigger in TRIGGERS {
            let Some(pos) = lower.find(trigger) else {
                continue;
            };
            // Only at the start of the typed input: line start or after the prompt.
            let before = &trimmed[..pos];
            if !(before.is_empty() || before.ends_with(' ')) {
                continue;
            }
            let request = trimmed[pos + trigger.len()..].trim();
            if !request.is_empty() {
                return Some(request.to_string());
            }
        }
    }
    None
}

/// Display name of a watched app, used in fixed stage strings.
pub fn note_app_name(app: &str) -> &'static str {
    match note_app(app) {
        "notes" => "Notes",
        "terminal" => "Terminal",
        "none" => "Remote prompts",
        "auto" => "TextEdit, Notes or Terminal",
        _ => "TextEdit",
    }
}

/// A prompt typed on another device (iPhone or iPad) for a running session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemotePrompt {
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateInvisibleSession {
    /// 2–300 seconds, or 0 to return to the admin default.
    pub interval_seconds: Option<u64>,
    /// auto, textedit, notes, or none; only while the session is stopped.
    pub app: Option<String>,
    /// Whether a screenshot accompanies each new note or request; applies live.
    pub read_screen: Option<bool>,
}

/// Whether a provider runs on the backend Mac ("local") or at a cloud vendor ("cloud").
pub fn ai_location(provider: &str) -> &'static str {
    if matches!(provider, "apple" | "mlx") {
        "local"
    } else {
        "cloud"
    }
}

/// One step of background work. Stage labels are fixed strings; never note or prompt text.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub at: String,
    /// textedit, capture, or chat
    pub source: String,
    pub stage: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// local, cloud, or empty when no AI was involved in this step
    #[serde(default)]
    pub location: String,
}

/// One measured inference, taken from the wallet-scoped usage ledger.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InferenceRecord {
    pub memo_id: String,
    pub created_at: String,
    pub provider: String,
    pub model: String,
    pub location: String,
    /// running, complete, error, or interrupted
    pub status: String,
    pub elapsed_ms: u64,
    pub first_token_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Estimated from the dated list-price table; never a provider bill.
    pub estimated_usd: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActivityTotals {
    pub requests: u64,
    pub elapsed_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Sum over cloud requests with a known price and reported tokens.
    pub estimated_usd: Option<f64>,
    /// Cloud requests whose tokens or price are unknown and therefore excluded.
    pub unpriced_cloud_requests: u64,
}

/// Live inference the backend is waiting on right now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveInference {
    pub memo_id: String,
    pub provider: String,
    pub model: String,
    pub location: String,
    /// Measured on the backend at read time; clients add their own delta since the read.
    pub waited_ms: u64,
}

/// Wallet-wide processing counts for one time window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageBucket {
    pub requests: u64,
    pub running: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub estimated_usd: Option<f64>,
    /// Provider id and request count, most used first.
    pub providers: Vec<(String, u64)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessingStats {
    pub ocr_runs: u64,
    pub skipped_unchanged: u64,
    pub local: UsageBucket,
    pub cloud: UsageBucket,
    /// Changed notes the local model looked at first (expensive modes only).
    #[serde(default)]
    pub local_checks: u64,
    /// ...of which it found no request, so nothing else ran.
    #[serde(default)]
    pub no_request: u64,
    /// ...of which it answered itself, at no cloud cost.
    #[serde(default)]
    pub answered_locally: u64,
    /// ...of which it handed to cloud AI, Codex or Claude Code.
    #[serde(default)]
    pub escalated: u64,
}

/// Dashboard numbers for the Cloud page. Everything is measured locally.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActivitySummary {
    /// The last 24 hours.
    pub day: ProcessingStats,
    pub total: ProcessingStats,
    /// RFC 3339 time of the newest measured inference, or empty.
    pub last_at: String,
    pub price_date: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionActivity {
    pub thinking: Option<ActiveInference>,
    /// Newest first.
    pub entries: Vec<ActivityEntry>,
    /// Newest first.
    pub inferences: Vec<InferenceRecord>,
    pub totals: ActivityTotals,
    /// ISO date of the list prices behind every estimate, or empty when none apply.
    #[serde(default)]
    pub price_date: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvisibleSession {
    pub id: String,
    pub title: String,
    pub client_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: InvisibleStatus,
    #[serde(default)]
    pub capture: crate::ContinuousStatus,
    #[serde(default)]
    pub capture_count: u64,
    #[serde(default)]
    pub memo_count: u64,
    #[serde(default)]
    pub recent_captures: Vec<String>,
    #[serde(default)]
    pub activity: SessionActivity,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInvisibleSession {
    #[serde(default)]
    pub client_id: String,
    pub title: Option<String>,
    /// Omit to have the backend create the source room along with the session.
    pub room_id: Option<String>,
    /// Start TextEdit replies immediately on the bound client.
    #[serde(default)]
    pub start: bool,
    /// auto (default), textedit, notes, or none.
    pub app: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvisibleDecision {
    pub id: String,
    pub approve: bool,
}
