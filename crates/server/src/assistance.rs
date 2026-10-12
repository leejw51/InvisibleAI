use crate::{Api, AppState, PrivateState, db::Database};
use anyhow::{Result, ensure};
use axum::Json;
use causewaybay_shared::AssistancePreferences;
use rusqlite::{Connection, OptionalExtension, params};

pub fn read_locked(conn: &Connection, owner: &str) -> Result<AssistancePreferences> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [format!("assistance.{owner}")],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value
        .map(|v| serde_json::from_str(&v))
        .transpose()?
        .unwrap_or_default())
}
pub fn read(db: &Database) -> Result<AssistancePreferences> {
    read_locked(&db.0.lock().unwrap(), db.owner())
}
pub async fn get(PrivateState(s): PrivateState) -> Api<Json<AssistancePreferences>> {
    Ok(Json(read(&s.db)?))
}
pub fn save(
    s: &std::sync::Arc<AppState>,
    patch: serde_json::Value,
) -> Result<AssistancePreferences> {
    let (old, value) = {
        let mut conn = s.db.0.lock().unwrap();
        s.db.writable(&conn)?;
        let old = read_locked(&conn, s.db.owner())?;
        let mut merged = serde_json::to_value(&old)?;
        let object = patch
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Settings must be an object"))?;
        for (k, v) in object {
            merged[k] = v.clone();
        }
        let value: AssistancePreferences = serde_json::from_value(merged)?;
        ensure!(
            matches!(value.wiki_group_by.as_str(), "topic" | "day"),
            "Choose topic or day grouping"
        );
        ensure!(
            (500..=10_000).contains(&value.textedit_settle_ms),
            "Typing pause must be 500–10000 milliseconds"
        );
        ensure!(
            matches!(value.textedit_style.as_str(), "concise" | "detailed"),
            "Choose concise or detailed answers"
        );
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("assistance.{}", s.db.owner()), serde_json::to_string(&value)?])?;
        crate::wiki::sync_locked(&tx, s.db.owner(), false)?;
        tx.commit()?;
        (old, value)
    };
    if old.textedit_enabled != value.textedit_enabled
        || old.textedit_settle_ms != value.textedit_settle_ms
        || old.textedit_style != value.textedit_style
    {
        crate::notes::settings_changed(s);
    }
    Ok(value)
}
pub async fn update(
    PrivateState(s): PrivateState,
    Json(patch): Json<serde_json::Value>,
) -> Api<Json<AssistancePreferences>> {
    Ok(Json(save(&s, patch)?))
}
