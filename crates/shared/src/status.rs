use crate::{Dashboard, Provider};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceStats {
    pub rooms: u64,
    pub messages: u64,
    pub memos: u64,
    pub pinned: u64,
    pub files: u64,
    pub images: u64,
    pub file_bytes: u64,
    pub screenshot_bytes: u64,
    pub running: u64,
    pub failed: u64,
    pub queued_captures: u64,
    pub rust_total: u64,
    pub rust_online: u64,
    pub web_total: u64,
    pub web_online: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UserStats {
    pub total: u64,
    pub admins: u64,
    pub approved: u64,
    pub pending: u64,
    pub blocked: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SystemStatus {
    pub stats: WorkspaceStats,
    pub users: UserStats,
    pub connections: Dashboard,
    pub providers: Vec<Provider>,
    pub semantic_status: String,
    pub demo: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ClientPackage {
    pub available: bool,
    pub reason: String,
    pub platform: String,
    pub backend_url: Option<String>,
    pub owner_wallet: Option<String>,
    pub token_required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceStatus {
    pub stats: WorkspaceStats,
    pub connections: Dashboard,
    pub system: Option<SystemStatus>,
    pub package: ClientPackage,
    pub updated_at: String,
}
