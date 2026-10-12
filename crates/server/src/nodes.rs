//! Node aliases are durable JSONL configuration, mirrored in SQLite.
use crate::{Api, ApiError, AppState};
use anyhow::{Result, ensure};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use causewaybay_shared::{CaptureClient, config};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn restore(s: &AppState) -> Result<()> {
    if let Some(nodes) = config::read(&s.config_dir, "nodes")?.as_object() {
        for (id, name) in nodes {
            if let Some(name) = name.as_str() {
                s.db.set_setting(&format!("node.{id}"), name)?;
            }
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    name: String,
}
pub async fn rename(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(v): Json<Rename>,
) -> Api<Json<CaptureClient>> {
    let _guard = s.settings_write.lock().unwrap();
    let mut client =
        s.db.clients()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Unknown capture computer".into()))?;
    let name = v.name.trim();
    if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Use a computer name between 1 and 120 bytes without control characters".into(),
        ));
    }
    save(&s, &id, name)?;
    client.name = name.into();
    crate::notifications::client(&s, &id, "computer.renamed", json!({"name":name}))?;
    Ok(Json(client))
}
fn save(s: &AppState, id: &str, name: &str) -> Result<()> {
    ensure!(!id.is_empty() && id.len() <= 80, "Invalid client ID");
    let mut nodes = config::read(&s.config_dir, "nodes")?;
    if nodes.is_null() {
        nodes = json!({});
    }
    nodes[id] = Value::String(name.into());
    config::append(&s.config_dir, "nodes", nodes)?;
    s.db.set_setting(&format!("node.{id}"), name)?;
    s.db.0.lock().unwrap().execute(
        "UPDATE capture_clients SET name=?2 WHERE id=?1",
        rusqlite::params![id, name],
    )?;
    Ok(())
}
