//! IndexedDB holds JSON message records only; attachments remain on the backend.
use causewaybay_shared::ChatMessage;
use futures_channel::mpsc;
use futures_util::StreamExt;
use idb::{DatabaseEvent, Factory, ObjectStoreParams, TransactionMode};
use wasm_bindgen::JsValue;
use yew::Callback;
fn error(_: impl std::fmt::Debug) -> String {
    "Message cache is unavailable. Your messages are still saved on the backend.".into()
}
async fn open(owner: &str) -> Result<idb::Database, String> {
    let factory = Factory::new().map_err(error)?;
    let mut request = factory
        .open(&format!("causewaybay-messages-{owner}"), Some(1))
        .map_err(error)?;
    request.on_upgrade_needed(|e| {
        if let Ok(db) = e.database() {
            let _ = db.create_object_store("messages", ObjectStoreParams::new());
        }
    });
    request.await.map_err(error)
}
async fn put(db: &idb::Database, m: &ChatMessage) -> Result<(), String> {
    let tx = db
        .transaction(&["messages"], TransactionMode::ReadWrite)
        .map_err(error)?;
    let store = tx.object_store("messages").map_err(error)?;
    let cutoff = store
        .get(JsValue::from_str(&format!("cleared/{}", m.roomid)))
        .map_err(error)?
        .await
        .map_err(error)?
        .and_then(|v| v.as_string())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let all_cutoff = store
        .get(JsValue::from_str("cleared/all"))
        .map_err(error)?
        .await
        .map_err(error)?
        .and_then(|v| v.as_string())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    if m.messagesyncid <= cutoff.max(all_cutoff) {
        tx.await.map_err(error)?;
        return Ok(());
    }
    let key = JsValue::from_str(&m.messageid);
    let old = store
        .get(key.clone())
        .map_err(error)?
        .await
        .map_err(error)?
        .and_then(|v| v.as_string())
        .and_then(|v| serde_json::from_str::<ChatMessage>(&v).ok());
    if old.is_none_or(|old| old.messagesyncid < m.messagesyncid) {
        store
            .put(
                &JsValue::from_str(&serde_json::to_string(m).map_err(error)?),
                Some(&key),
            )
            .map_err(error)?
            .await
            .map_err(error)?;
    }
    tx.commit().map_err(error)?.await.map_err(error)?;
    Ok(())
}
pub enum Write {
    Message(Box<ChatMessage>),
    RemoveRoom(String),
    ClearRoom(String, i64),
    ClearAll(i64),
}
async fn remove_all(db: &idb::Database, before: i64) -> Result<(), String> {
    let tx = db
        .transaction(&["messages"], TransactionMode::ReadWrite)
        .map_err(error)?;
    let store = tx.object_store("messages").map_err(error)?;
    let key = JsValue::from_str("cleared/all");
    let previous = store
        .get(key.clone())
        .map_err(error)?
        .await
        .map_err(error)?
        .and_then(|v| v.as_string())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let before = before.max(previous);
    let values = store
        .get_all(None, None)
        .map_err(error)?
        .await
        .map_err(error)?;
    for value in values {
        if let Some(m) = value
            .as_string()
            .and_then(|v| serde_json::from_str::<ChatMessage>(&v).ok())
            && m.messagesyncid <= before
        {
            store
                .delete(JsValue::from_str(&m.messageid))
                .map_err(error)?
                .await
                .map_err(error)?;
        }
    }
    store
        .put(&JsValue::from_str(&before.to_string()), Some(&key))
        .map_err(error)?
        .await
        .map_err(error)?;
    tx.commit().map_err(error)?.await.map_err(error)?;
    Ok(())
}
async fn remove_room(db: &idb::Database, room: &str, before: i64) -> Result<(), String> {
    let tx = db
        .transaction(&["messages"], TransactionMode::ReadWrite)
        .map_err(error)?;
    let store = tx.object_store("messages").map_err(error)?;
    let key = JsValue::from_str(&format!("cleared/{room}"));
    let previous = store
        .get(key.clone())
        .map_err(error)?
        .await
        .map_err(error)?
        .and_then(|v| v.as_string())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let values = store
        .get_all(None, None)
        .map_err(error)?
        .await
        .map_err(error)?;
    // An absent room listing can be temporary while the server is clearing it.
    // Never persist i64::MAX as a floor: the empty room may accept new messages.
    let before = if before == i64::MAX {
        values
            .iter()
            .filter_map(|v| v.as_string())
            .filter_map(|s| serde_json::from_str::<ChatMessage>(&s).ok())
            .filter(|m| m.roomid == room)
            .map(|m| m.messagesyncid)
            .max()
            .unwrap_or(0)
    } else {
        before
    }
    .max(previous);
    store
        .put(&JsValue::from_str(&before.to_string()), Some(&key))
        .map_err(error)?
        .await
        .map_err(error)?;
    for value in values {
        if let Some(m) = value
            .as_string()
            .and_then(|s| serde_json::from_str::<ChatMessage>(&s).ok())
            && m.roomid == room
            && m.messagesyncid <= before
        {
            store
                .delete(JsValue::from_str(&m.messageid))
                .map_err(error)?
                .await
                .map_err(error)?;
        }
    }
    tx.commit().map_err(error)?.await.map_err(error)?;
    Ok(())
}
pub fn writer(owner: String, notify: Callback<String>) -> mpsc::UnboundedSender<Write> {
    let (tx, mut rx) = mpsc::unbounded::<Write>();
    wasm_bindgen_futures::spawn_local(async move {
        match open(&owner).await {
            Ok(db) => {
                while let Some(m) = rx.next().await {
                    let result = match m {
                        Write::Message(m) => put(&db, &m).await,
                        Write::RemoveRoom(room) => remove_room(&db, &room, i64::MAX).await,
                        Write::ClearRoom(room, cutoff) => remove_room(&db, &room, cutoff).await,
                        Write::ClearAll(cutoff) => remove_all(&db, cutoff).await,
                    };
                    if let Err(e) = result {
                        notify.emit(e);
                        break;
                    }
                }
                db.close();
            }
            Err(e) => notify.emit(e),
        }
    });
    tx
}
