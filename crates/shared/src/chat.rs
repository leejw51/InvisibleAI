use crate::{FileInfo, InputMode, ThinkRequest};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Room {
    pub roomid: String,
    pub title: String,
    pub created_at: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub updated_at: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceLink {
    pub title: String,
    pub url: String,
}
/// A skill the AI ran while answering: its name and whether the run worked.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillUse {
    pub name: String,
    pub success: bool,
}
/// Measured usage behind one AI reply, summed over its provider calls.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageUsage {
    pub elapsed_ms: u64,
    pub first_token_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Estimated from the dated list-price table; never a provider bill.
    pub estimated_usd: Option<f64>,
    pub calls: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub roomid: String,
    pub messageid: String,
    pub messagesyncid: i64,
    pub sequence: i64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub edited_at: Option<String>,
    pub role: String,
    pub text: String,
    pub ocr_text: String,
    pub transcript: String,
    pub image_path: Option<String>,
    pub attachments: Vec<FileInfo>,
    #[serde(default)]
    pub sources: Vec<SourceLink>,
    /// The user's skills the AI ran for this reply, in order of first use.
    #[serde(default)]
    pub skills: Vec<SkillUse>,
    pub input_mode: InputMode,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub stage: String,
    pub error: Option<String>,
    pub memo_id: String,
    pub client_name: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub usage: Option<MessageUsage>,
    /// Live progress while the reply is being prepared: an on-device model
    /// downloading, waiting its turn, or loading. Cleared once the model answers.
    #[serde(default)]
    pub progress: Option<crate::ModelProgress>,
    /// Wiki tags of the memo behind this message, shared by the post and its
    /// generated response: the user's own hashtags and edits, then those the
    /// on-device analysis of both texts added. Empty until analyzed.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The memo's kind (`question`, `note`, `memo`, `capture`, …), so the chat
    /// can label a tidied record differently from an answer.
    #[serde(default)]
    pub memo_kind: String,
    /// Whether the memo behind this message is in the Wiki: `pending` (not yet
    /// judged), `kept`, or `skipped` (judged not worth remembering; the user
    /// can keep it anyway).
    #[serde(default)]
    pub wiki_state: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MessageSync {
    pub messages: Vec<ChatMessage>,
    pub next: i64,
    pub has_more: bool,
    #[serde(default)]
    pub cleared_through: i64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub url: String,
    pub source: String,
    pub created_at: String,
    /// The message the picture belongs to (its Wiki node), when it has one.
    #[serde(default)]
    pub memo_id: String,
    /// That message's Wiki tags, editable like any message's.
    #[serde(default)]
    pub tags: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AiInfo {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub configured: bool,
    pub demo: bool,
    pub audio_provider: Option<String>,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuousRequest {
    pub client_ids: Vec<String>,
    #[serde(default)]
    pub all: bool,
    pub interval_seconds: u64,
    pub request: ThinkRequest,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContinuousStatus {
    pub roomid: String,
    pub active: bool,
    pub interval_seconds: u64,
    pub client_ids: Vec<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub capturing: bool,
    #[serde(default)]
    pub capture_queued: bool,
    #[serde(default)]
    pub next_capture_at: Option<String>,
    #[serde(default)]
    pub unchanged_count: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessedFile {
    pub file: FileInfo,
    pub kind: String,
    pub text: String,
    pub truncated: bool,
    pub processor: String,
}
