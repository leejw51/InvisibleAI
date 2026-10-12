//! Durable arbitrary JSON notifications, scoped separately to wallets and capture devices.
use crate::{Api, AppState, PrivateState, db::Database};
use anyhow::Result;
use axum::{
    extract::Query,
    http::HeaderMap,
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, time::Duration};

pub fn wallet(db: &Database, room: Option<&str>, kind: &str, data: Value) -> Result<()> {
    record(db, Some(db.owner()), None, room, kind, data)
}
pub fn client(s: &AppState, id: &str, kind: &str, data: Value) -> Result<()> {
    record(&s.db, None, Some(id), None, kind, data)
}
fn record(
    db: &Database,
    owner: Option<&str>,
    client: Option<&str>,
    room: Option<&str>,
    kind: &str,
    data: Value,
) -> Result<()> {
    db.0.lock().unwrap().execute(
        "INSERT INTO notifications(owner_wallet,client_id,roomid,kind,data) VALUES(?1,?2,?3,?4,?5)",
        params![owner, client, room, kind, data.to_string()],
    )?;
    Ok(())
}
pub fn changes(db: &Database, client: Option<&str>, after: i64) -> Result<Vec<Value>> {
    let conn = db.0.lock().unwrap();
    let mut stmt=conn.prepare("SELECT id,kind,data FROM notifications WHERE id>?1 AND ((?2 IS NOT NULL AND client_id=?2) OR (?2 IS NULL AND client_id IS NULL AND owner_wallet=?3)) ORDER BY id LIMIT 128")?;
    let rows = stmt
        .query_map(params![after, client, db.owner()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(|(id,kind,data)|Ok(json!({"type":"notification","id":id,"kind":kind,"data":serde_json::from_str::<Value>(&data)?}))).collect()
}
/// The newest notification id at this moment, 0 when none was ever recorded.
pub fn latest(db: &Database) -> Result<i64> {
    Ok(db.0.lock().unwrap().query_row(
        "SELECT COALESCE(MAX(id),0) FROM notifications",
        [],
        |r| r.get(0),
    )?)
}
/// Mark a notification recorded at or before `live_from` as a replay.
pub fn flag_replay(mut event: Value, live_from: i64) -> Value {
    if event["id"].as_i64().unwrap_or(0) <= live_from {
        event["replay"] = json!(true);
    }
    event
}
#[derive(Default, Deserialize)]
pub struct Cursor {
    #[serde(default)]
    pub after: i64,
}
pub async fn events(
    PrivateState(s): PrivateState,
    Query(q): Query<Cursor>,
    headers: HeaderMap,
) -> Api<impl axum::response::IntoResponse> {
    let mut after = headers
        .get("last-event-id")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(q.after)
        .max(0);
    // A connection without a cursor is a fresh page: everything already recorded
    // is history it is catching up on, which caches still apply but the views it
    // has just loaded must not reset on. A reconnect with a cursor is resuming,
    // and what it missed meanwhile is live news.
    let has_cursor = headers.contains_key("last-event-id") || q.after > 0;
    let live_from = if has_cursor { 0 } else { latest(&s.db)? };
    let stream = async_stream::stream! {
        yield Ok::<_,Infallible>(Event::default().retry(Duration::from_secs(2)).comment("notifications ready"));
        loop{
            if crate::auth::authorize(&s,&headers,"GET","/api/notifications").is_err(){break;}
            match changes(&s.db,None,after){Ok(events)=>for event in events{after=event["id"].as_i64().unwrap();let event=flag_replay(event,live_from);yield Ok(Event::default().id(after.to_string()).json_data(event).unwrap());},Err(_)=>break}
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_flagged_as_replay_and_new_events_are_not() {
        let old = flag_replay(json!({"id":7,"kind":"room.deleted"}), 7);
        assert_eq!(old["replay"], json!(true));
        let fresh = flag_replay(json!({"id":8,"kind":"room.deleted"}), 7);
        assert!(fresh.get("replay").is_none());
    }
}
