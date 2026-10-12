//! Persistent room messages and a server-assigned int64 change cursor.
use crate::PrivateState;
use crate::{Api, ApiError, AppState, db};
use anyhow::{Result, ensure};
use axum::{
    Json,
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use causewaybay_shared::*;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

fn guid(id: &str) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(id).is_ok(), "Expected a GUID");
    Ok(())
}
fn next_sync_id(tx: &Connection) -> Result<i64> {
    // Microseconds since Unix epoch, advanced under the write transaction even if
    // the wall clock moves backwards or several revisions share a timestamp.
    let last: i64 = tx.query_row(
        "SELECT MAX((SELECT value FROM sync_clock WHERE id=1),COALESCE(MAX(messagesyncid),0)) FROM message_changes",
        [],
        |r| r.get(0),
    )?;
    let next = chrono::Utc::now().timestamp_micros().max(
        last.checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Sync cursor exhausted"))?,
    );
    tx.execute("UPDATE sync_clock SET value=?1 WHERE id=1", [next])?;
    Ok(next)
}
pub fn write(tx: &Connection, m: &mut ChatMessage) -> Result<()> {
    m.messagesyncid = next_sync_id(tx)?;
    tx.execute(
        "DELETE FROM message_changes WHERE messageid=?1",
        [&m.messageid],
    )?;
    tx.execute(
        "INSERT INTO message_changes(messagesyncid,roomid,messageid,document) VALUES(?1,?2,?3,'')",
        params![m.messagesyncid, m.roomid, m.messageid],
    )?;
    if m.sequence == 0 {
        m.sequence = m.messagesyncid;
    }
    let document = serde_json::to_string(m)?;
    tx.execute("INSERT INTO chat_messages(messageid,roomid,memo_id,role,document) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(messageid) DO UPDATE SET document=excluded.document",params![m.messageid,m.roomid,m.memo_id,m.role,document])?;
    tx.execute(
        "UPDATE message_changes SET document=?2 WHERE messagesyncid=?1",
        params![m.messagesyncid, document],
    )?;
    Ok(())
}
pub fn refresh(tx: &Transaction<'_>, memo: &str, kind: &str, text: &str) -> Result<()> {
    let docs = tx
        .prepare("SELECT document FROM chat_messages WHERE memo_id=?1")?
        .query_map([memo], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if docs.is_empty() {
        return Ok(());
    }
    let (answer, status, error, image, memo_kind): (
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    ) = tx.query_row(
        "SELECT answer,status,error,image_path,kind FROM memos WHERE id=?1",
        [memo],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    for doc in docs {
        let mut m: ChatMessage = serde_json::from_str(&doc)?;
        if m.deleted || m.edited_at.is_some() {
            continue;
        }
        let old = m.clone();
        m.error = error.clone();
        m.memo_kind = memo_kind.clone();
        if m.role == "assistant" {
            m.text = answer.clone();
            m.status = status.clone();
            if matches!(kind, "kind" | "error" | "done") {
                m.usage = message_usage(tx, memo)?;
            }
            // A skill the AI ran shows on the reply: which one, and whether it worked.
            if kind == "tool_result"
                && let Ok(event) = serde_json::from_str::<serde_json::Value>(text)
                && event["name"] == "run_skill"
                && let Some(output) = event["output"].as_str()
            {
                let result = serde_json::from_str::<serde_json::Value>(output).unwrap_or_default();
                let name = result["skill"].as_str().unwrap_or("").to_owned();
                let success = result["success"] == true;
                if !name.is_empty() {
                    match m.skills.iter_mut().find(|k| k.name == name) {
                        Some(k) => k.success = success,
                        None => m
                            .skills
                            .push(causewaybay_shared::SkillUse { name, success }),
                    }
                    m.skills.truncate(20);
                }
            }
            if kind == "tool_result"
                && let Ok(event) = serde_json::from_str::<serde_json::Value>(text)
                && event["name"] == "web_search"
                && let Some(output) = event["output"].as_str()
                && let Ok(result) = serde_json::from_str::<serde_json::Value>(output)
                && let Some(sources) = result["sources"].as_array()
            {
                for source in sources {
                    if let Ok(source) = serde_json::from_value::<SourceLink>(source.clone())
                        && source.url.starts_with("https://")
                        && !m.sources.contains(&source)
                    {
                        m.sources.push(source);
                    }
                }
                m.sources.truncate(30);
            }
        } else {
            m.image_path = image.clone();
            if kind == "ocr_text" || kind == "ocr" && m.ocr_text.is_empty() {
                m.ocr_text = text.into();
            }
            if kind == "transcript" {
                m.transcript = text.into();
            }
            if image.is_some() || !m.attachments.is_empty() || status != "running" {
                m.status = "complete".into();
            }
        }
        if kind == "stage" {
            m.stage = text.into();
        }
        if kind == "done" {
            m.stage = "Complete".into();
        }
        // Preparation progress lives until the next stage or the first words.
        if kind == "progress" {
            m.progress = serde_json::from_str(text).ok();
        } else if matches!(kind, "stage" | "delta" | "done" | "error") {
            m.progress = None;
        }
        if m != old {
            write(tx, &mut m)?;
        }
    }
    Ok(())
}
/// Sum every measured provider call behind a memo; None until one exists.
fn message_usage(tx: &Transaction<'_>, memo: &str) -> Result<Option<MessageUsage>> {
    usage_for(tx, memo)
}
/// Wait, tokens, and estimated cloud cost of every model call made for a memo.
pub fn usage_for(conn: &rusqlite::Connection, memo: &str) -> Result<Option<MessageUsage>> {
    let rows = conn
        .prepare("SELECT provider,model,elapsed_ms,first_token_ms,input_tokens,output_tokens FROM ai_usage WHERE memo_id=?1 ORDER BY created_at")?
        .query_map([memo], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?.max(0) as u64,
                r.get::<_, Option<i64>>(3)?.map(|v| v.max(0) as u64),
                r.get::<_, Option<i64>>(4)?.map(|v| v.max(0) as u64),
                r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut usage = MessageUsage::default();
    let mut tokens_known = false;
    for (provider, model, elapsed, first, input, output) in rows {
        usage.calls += 1;
        usage.elapsed_ms += elapsed;
        if usage.first_token_ms.is_none() {
            usage.first_token_ms = first;
        }
        if let (Some(i), Some(o)) = (input, output) {
            tokens_known = true;
            *usage.input_tokens.get_or_insert(0) += i;
            *usage.output_tokens.get_or_insert(0) += o;
        }
        if let Some(usd) = crate::activity::estimate_usd(&provider, &model, input, output) {
            *usage.estimated_usd.get_or_insert(0.0) += usd;
        }
    }
    if !tokens_known {
        usage.input_tokens = None;
        usage.output_tokens = None;
    }
    Ok(Some(usage))
}
pub fn attach(
    db: &db::Database,
    room: &str,
    messageid: &str,
    memo: &Memo,
    req: &ThinkRequest,
) -> Result<()> {
    let attachments = req
        .attachment_ids()
        .into_iter()
        .map(|id| db.file_info(id))
        .collect::<Result<Vec<_>>>()?;
    let client_name = req.client_id.as_ref().and_then(|id| {
        db.clients()
            .ok()?
            .into_iter()
            .find(|c| c.id == *id)
            .map(|c| c.name)
    });
    let mut user = ChatMessage {
        roomid: room.into(),
        messageid: messageid.into(),
        role: "user".into(),
        text: req.prompt.clone(),
        memo_id: memo.id.clone(),
        attachments,
        input_mode: req.input_mode,
        provider: memo.provider.clone(),
        model: memo.model.clone(),
        created_at: memo.created_at.clone(),
        status: if req.capture { "capturing" } else { "complete" }.into(),
        client_name,
        memo_kind: memo.kind.clone(),
        ..Default::default()
    };
    let mut assistant = ChatMessage {
        roomid: room.into(),
        messageid: db::id(),
        role: "assistant".into(),
        memo_id: memo.id.clone(),
        input_mode: req.input_mode,
        provider: memo.provider.clone(),
        model: memo.model.clone(),
        created_at: memo.created_at.clone(),
        status: "running".into(),
        memo_kind: memo.kind.clone(),
        ..Default::default()
    };
    let mut conn = db.0.lock().unwrap();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO chat_submissions(messageid,request) VALUES(?1,?2)",
        params![messageid, submission_digest(&serde_json::to_string(req)?)],
    )?;
    write(&tx, &mut user)?;
    write(&tx, &mut assistant)?;
    tx.commit()?;
    Ok(())
}
pub(crate) fn submission_digest(request: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(request.as_bytes())))
}
pub fn existing(
    db: &db::Database,
    room: &str,
    messageid: &str,
    req: &ThinkRequest,
) -> Result<Option<Memo>> {
    guid(room)?;
    guid(messageid)?;
    get_room(db, room)?;
    let conn = db.0.lock().unwrap();
    ensure!(
        conn.query_row("SELECT count(*) FROM rooms WHERE roomid=?1", [room], |r| {
            r.get::<_, i64>(0)
        })? == 1,
        "Room not found"
    );
    let doc: Option<String> = conn
        .query_row(
            "SELECT document FROM chat_messages WHERE messageid=?1",
            [messageid],
            |r| r.get(0),
        )
        .optional()?;
    let original: Option<String> = conn
        .query_row(
            "SELECT request FROM chat_submissions WHERE messageid=?1",
            [messageid],
            |r| r.get(0),
        )
        .optional()?;
    drop(conn);
    if let Some(original) = original {
        ensure!(
            original == submission_digest(&serde_json::to_string(req)?),
            "Message ID is already used for a different request"
        );
    }
    if let Some(doc) = doc {
        let m: ChatMessage = serde_json::from_str(&doc)?;
        ensure!(
            !m.deleted
                && m.edited_at.is_none()
                && m.roomid == room
                && m.role == "user"
                && m.text == req.prompt
                && m.input_mode == req.input_mode
                && m.attachments
                    .iter()
                    .map(|f| f.id.as_str())
                    .collect::<Vec<_>>()
                    == req.attachment_ids(),
            "Message ID is already used for a different message"
        );
        return Ok(Some(db.get(&m.memo_id)?.ok_or_else(|| {
            anyhow::anyhow!("This message was already submitted and its memo was deleted")
        })?));
    }
    Ok(None)
}
/// The memo behind a kept message when this GUID was already used, so a retried
/// keep cannot duplicate the file or the Wiki source.
pub fn kept_message(db: &db::Database, room: &str, messageid: &str) -> Result<Option<Memo>> {
    guid(room)?;
    guid(messageid)?;
    get_room(db, room)?;
    let doc: Option<String> =
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT document FROM chat_messages WHERE messageid=?1",
                [messageid],
                |r| r.get(0),
            )
            .optional()?;
    let Some(doc) = doc else {
        return Ok(None);
    };
    let m: ChatMessage = serde_json::from_str(&doc)?;
    ensure!(
        m.roomid == room && m.provider == "memo",
        "Message ID is already used for a different message"
    );
    Ok(Some(db.get(&m.memo_id)?.ok_or_else(|| {
        anyhow::anyhow!("This message was already kept and its memo was deleted")
    })?))
}
fn history_documents(db: &db::Database, room: &str, before: &str) -> Result<Vec<ChatMessage>> {
    get_room(db, room)?;
    let conn = db.0.lock().unwrap();
    let mut stmt=conn.prepare("SELECT document FROM chat_messages WHERE roomid=?1 AND json_extract(document,'$.sequence') < (SELECT MIN(json_extract(document,'$.sequence')) FROM chat_messages WHERE memo_id=?2) ORDER BY json_extract(document,'$.sequence') DESC LIMIT 24")?;
    let docs = stmt
        .query_map(params![room, before], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    docs.into_iter()
        .map(|doc| Ok(serde_json::from_str(&doc)?))
        .collect()
}

/// Older TextEdit sessions saved room-owned memos without chat messages. Import
/// those once into the shared history, preserving their original chronology.
pub fn restore_history(database: &db::Database) -> Result<()> {
    let mut conn = database.0.lock().unwrap();
    let tx = conn.transaction()?;
    let rows = tx.prepare("SELECT m.roomid,m.id,m.input,m.answer,m.provider,m.model,m.status,m.created_at,m.kind,m.image_path,m.error FROM memos m JOIN rooms r ON r.roomid=m.roomid WHERE r.deleted=0 AND (m.input!='' OR m.answer!='') AND NOT EXISTS(SELECT 1 FROM chat_messages c WHERE c.memo_id=m.id) ORDER BY m.created_at,m.rowid")?
        .query_map([], |r| Ok((ChatMessage {
            roomid: r.get(0)?, memo_id: r.get(1)?, text: r.get(2)?, provider: r.get(4)?,
            model: r.get(5)?, status: "complete".into(), created_at: r.get(7)?,
            image_path: r.get(9)?, ..Default::default()
        }, r.get::<_, String>(3)?, r.get::<_, String>(6)?, r.get::<_, String>(8)?, r.get::<_, Option<String>>(10)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (mut user, answer, status, kind, error) in rows {
        user.messageid = db::id();
        user.role = "user".into();
        user.sequence = chrono::DateTime::parse_from_rfc3339(&user.created_at)?.timestamp_micros();
        let mut assistant = ChatMessage {
            roomid: user.roomid.clone(),
            memo_id: user.memo_id.clone(),
            messageid: db::id(),
            role: "assistant".into(),
            text: answer,
            provider: user.provider.clone(),
            model: user.model.clone(),
            status,
            error,
            created_at: user.created_at.clone(),
            sequence: user.sequence + 1,
            ..Default::default()
        };
        if kind == "memo" {
            user.provider = "memo".into();
        }
        write(&tx, &mut user)?;
        if kind != "memo" {
            write(&tx, &mut assistant)?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Rebuild from current message revisions, never from stale submissions or memo input.
/// The most recent photos take priority within the image budget; Text mode sends no images.
pub async fn history(
    db: &db::Database,
    room: &str,
    before: &str,
    mode: InputMode,
    mut image_budget: usize,
) -> Result<Vec<crate::providers::Message>> {
    use crate::{capture, media, providers::Message};
    let docs = history_documents(db, room, before)?;
    let mut out = vec![];
    for m in docs {
        if m.deleted || m.status != "complete" {
            continue;
        }
        let mut text = m.text.chars().take(8000).collect::<String>();
        // A long typed comment must not crowd out all OCR or audio context.
        for (label, content) in [
            ("Extracted image text", &m.ocr_text),
            ("Audio transcript", &m.transcript),
        ] {
            if !content.is_empty() {
                text.push_str(&format!(
                    "\n\n{label}:\n{}",
                    content.chars().take(4000).collect::<String>()
                ));
            }
        }
        if m.role != "user" {
            out.push(Message::Assistant {
                text,
                calls: vec![],
            });
            continue;
        }
        let mut images = vec![];
        let allow_images = mode == InputMode::Image && m.input_mode == InputMode::Image;
        // Old single-photo messages also have a memo image; do not send it twice.
        let screenshot = if allow_images && m.image_path.is_some() {
            db.image(&m.memo_id)?
        } else {
            None
        };
        let mut screenshot_is_attachment = false;
        for file in &m.attachments {
            let (info, bytes) = db.file(&file.id)?;
            if capture::image_mime(&bytes).is_some() {
                screenshot_is_attachment |= screenshot.as_ref() == Some(&bytes);
                if allow_images && image_budget > 0 {
                    images.push(capture::cloud_image(bytes).await?);
                    image_budget -= 1;
                    text.push_str(&format!("\nPhoto {}: {}", images.len(), info.name));
                } else {
                    text.push_str(&format!(
                        "\n[Earlier photo {} is not included visually in this request.]",
                        info.name
                    ));
                }
            } else if !info.mime.starts_with("audio/") {
                let (_, content, _) = media::context_text(&info.name, &info.mime, bytes).await?;
                // Budget per file as well as per message; comments retain first priority.
                text.push_str(&format!(
                    "\nAttached file: {}\n{}",
                    info.name,
                    content.chars().take(4000).collect::<String>()
                ));
            }
        }
        if let Some(bytes) = screenshot
            && !screenshot_is_attachment
        {
            if image_budget > 0 {
                images.push(capture::cloud_image(bytes).await?);
                image_budget -= 1;
                text.push_str(&format!("\nPhoto {}: captured screen", images.len()));
            } else {
                text.push_str("\n[Earlier screen image omitted from this request's image budget.]");
            }
        }
        text = text.chars().take(16000).collect();
        out.push(if images.is_empty() {
            Message::User(text)
        } else {
            Message::Image { text, images }
        });
    }
    out.reverse();
    Ok(out)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewRoom {
    pub roomid: Option<String>,
    pub title: Option<String>,
}
pub async fn create(PrivateState(s): PrivateState, Json(v): Json<NewRoom>) -> Api<Json<Room>> {
    let _lock = s.chat_submit.lock().unwrap();
    Ok(Json(create_room(&s.db, v)?))
}

/// Shared creation path for chat and the Cloud session view; caller holds chat_submit.
pub fn create_room(database: &db::Database, v: NewRoom) -> Result<Room> {
    let roomid = v.roomid.unwrap_or_else(db::id);
    guid(&roomid)?;
    let title = title(v.title.as_deref().unwrap_or("New conversation"))?;
    {
        let mut conn = database.0.lock().unwrap();
        database.writable(&conn)?;
        let tx = conn.transaction()?;
        let created = tx.execute("INSERT INTO rooms(roomid,title,created_at,updated_at,owner_wallet) VALUES(?1,?2,?3,?3,?4) ON CONFLICT(roomid) DO NOTHING",params![roomid,title,db::now(),database.owner()])?;
        if created > 0 {
            tx.execute("INSERT INTO notifications(owner_wallet,roomid,kind,data) VALUES(?1,?2,'room.created',?3)",params![database.owner(),roomid,serde_json::json!({"roomid":roomid}).to_string()])?;
        }
        tx.commit()?;
    }
    get_room(database, &roomid)
}
fn title(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.is_empty() && value.chars().count() <= 100 && !value.chars().any(char::is_control),
        "Use a room name of 1–100 characters"
    );
    Ok(value.into())
}
pub fn get_room(db: &db::Database, id: &str) -> Result<Room> {
    db.require("rooms", id)?;
    let room = db.0.lock().unwrap().query_row(
        "SELECT roomid,title,created_at,deleted,updated_at FROM rooms WHERE roomid=?1",
        [id],
        room_row,
    )?;
    ensure!(!room.deleted, "Room not found");
    Ok(room)
}
fn room_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Room> {
    Ok(Room {
        roomid: r.get(0)?,
        title: r.get(1)?,
        created_at: r.get(2)?,
        deleted: r.get(3)?,
        updated_at: r.get(4)?,
    })
}
pub async fn rooms(PrivateState(s): PrivateState) -> Api<Json<Vec<Room>>> {
    let conn = s.db.0.lock().unwrap();
    let mut stmt = conn.prepare("SELECT roomid,title,created_at,deleted,updated_at FROM rooms WHERE (?1 IS NULL OR owner_wallet=?1) AND deleted=0 ORDER BY updated_at DESC,created_at DESC")?;
    Ok(Json(
        stmt.query_map([&s.db.2], room_row)?
            .collect::<rusqlite::Result<_>>()?,
    ))
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThoughtRequest {
    #[serde(default)]
    pub fresh: bool,
    pub room_id: Option<String>,
}

/// Read the wallet's current thought without creating or resetting a room.
pub async fn current_thought(PrivateState(s): PrivateState) -> Api<Json<Option<Room>>> {
    let room =
        s.db.get_setting(&format!("thought.{}", s.db.owner()))?
            .and_then(|id| get_room(&s.db, &id).ok());
    Ok(Json(room))
}

/// Resume on reload; an explicit New thought always gets an independent room.
pub async fn thought(
    PrivateState(s): PrivateState,
    Json(request): Json<ThoughtRequest>,
) -> Api<Json<Room>> {
    let _lock = s.chat_submit.lock().unwrap();
    s.db.writable(&s.db.0.lock().unwrap())?;
    let key = format!("thought.{}", s.db.owner());
    if let Some(id) = request.room_id {
        let room = get_room(&s.db, &id)?;
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,id])?;
        tx.execute("INSERT INTO notifications(owner_wallet,roomid,kind,data) VALUES(?1,?2,'room.thought',?3)", params![s.db.owner(),id,serde_json::json!({"roomid":id}).to_string()])?;
        tx.commit()?;
        return Ok(Json(room));
    }
    if !request.fresh
        && let Some(id) = s.db.get_setting(&key)?
        && let Ok(room) = get_room(&s.db, &id)
    {
        return Ok(Json(room));
    }
    let id = db::id();
    let stamp = db::now();
    let mut conn = s.db.0.lock().unwrap();
    let tx = conn.transaction()?;
    tx.execute("INSERT INTO rooms(roomid,title,created_at,updated_at,owner_wallet) VALUES(?1,'New thought',?2,?2,?3)",params![id,stamp,s.db.owner()])?;
    tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,id])?;
    tx.execute(
        "INSERT INTO notifications(owner_wallet,roomid,kind,data) VALUES(?1,?2,'room.thought',?3)",
        params![
            s.db.owner(),
            id,
            serde_json::json!({"roomid":id}).to_string()
        ],
    )?;
    tx.commit()?;
    drop(conn);
    Ok(Json(get_room(&s.db, &id)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    title: String,
}
pub async fn rename(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
    Json(v): Json<Rename>,
) -> Api<Json<Room>> {
    let _lock = s.chat_submit.lock().unwrap();
    get_room(&s.db, &room)?;
    let name = title(&v.title)?;
    s.db.0.lock().unwrap().execute(
        "UPDATE rooms SET title=?2,updated_at=?3 WHERE roomid=?1",
        params![room, name, db::now()],
    )?;
    crate::notifications::wallet(
        &s.db,
        Some(&room),
        "room.renamed",
        serde_json::json!({"roomid":room}),
    )?;
    Ok(Json(get_room(&s.db, &room)?))
}
pub async fn delete_room(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
) -> Api<Json<serde_json::Value>> {
    purge_room(&s, &room, true).await
}
pub async fn clear_room(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
) -> Api<Json<serde_json::Value>> {
    purge_room(&s, &room, false).await
}

/// Stage a screenshot as a room-owned attachment, without creating a chat turn
/// or invoking AI. The temporary memo ties the device request to room erasure.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    client_id: Option<String>,
    display: Option<u32>,
    clear_epoch: Option<i64>,
    data_epoch: Option<i64>,
}
pub async fn snapshot(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<Snapshot>,
) -> Api<Json<FileInfo>> {
    let actor = crate::auth::authorize(&s, &headers, "POST", "/api/rooms")?;
    let memo = {
        let _lock = s.chat_submit.lock().unwrap();
        validate_epoch(&s.db, Some(&room), req.clear_epoch, req.data_epoch)?;
        get_room(&s.db, &room)?;
        if let Some(bound) = crate::invisible_sessions::bound_client(&s.db, &room)? {
            if req.client_id.as_ref().is_some_and(|id| id != &bound) {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "This session is bound to another capture computer".into(),
                ));
            }
            req.client_id = Some(bound);
        }
        if req.client_id.is_none() && !s.demo {
            req.client_id = s
                .db
                .clients()?
                .into_iter()
                .find(|c| c.online && crate::transport::can_capture(c, actor.address.as_deref()))
                .map(|c| c.id);
        }
        if crate::continuous::capture_in_flight(&s.db, &room)?
            || s.continuous
                .lock()
                .unwrap()
                .get(&room)
                .is_some_and(|r| r.status.active)
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "Use Capture while polling is active".into(),
            ));
        }
        if let Some(client) = &req.client_id {
            let allowed = s.db.clients()?.iter().any(|c| {
                c.id == *client
                    && c.online
                    && crate::transport::can_capture(c, actor.address.as_deref())
            });
            if !allowed {
                return Err(ApiError(
                    StatusCode::BAD_REQUEST,
                    "Choose a connected capture computer".into(),
                ));
            }
            crate::invisible_sessions::register(&s.db, &room, client)?;
        }
        let memo = s.db.create("", "capture", "local", "")?;
        s.db.assign_room("memos", &memo.id, &room)?;
        memo
    };
    let result: Result<Vec<u8>> = if s.demo && req.client_id.is_none() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(640, 360, image::Rgb([230, 220, 245]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map(|_| bytes.into_inner())
            .map_err(Into::into)
    } else {
        crate::transport::request_capture(
            &s,
            &memo.id,
            req.client_id.as_deref(),
            "screen",
            req.display.unwrap_or(1),
            actor.address.as_deref(),
        )
        .await
    };
    let _lock = s.chat_submit.lock().unwrap();
    // A clear can finish while capture is in flight. Checking the original memo
    // as well as the room prevents a late screenshot from recreating erased data.
    let exists = s.db.get(&memo.id)?.is_some();
    let result = result.and_then(|bytes| {
        ensure!(exists, "This capture was deleted");
        get_room(&s.db, &room)?;
        let file =
            s.db.put_file(&format!("capture-{}.png", memo.id), "image/png", &bytes)?;
        s.db.assign_room("files", &file.id, &room)?;
        Ok(file)
    });
    if exists {
        s.db.delete(&memo.id)?;
    }
    Ok(Json(result?))
}

pub async fn delete_all(PrivateState(s): PrivateState) -> Api<Json<serde_json::Value>> {
    let locks = s.owner_locks();
    let _erasure = locks.erasure.write().await;
    let (rooms, handles) = {
        let _lock = s.chat_submit.lock().unwrap();
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction()?;
        // Retrying after an interrupted filesystem operation resumes the purge.
        tx.execute(
            "INSERT OR IGNORE INTO user_purges(owner_wallet) VALUES(?1)",
            [s.db.owner()],
        )?;
        let orphan_room = db::id();
        tx.execute("INSERT INTO rooms(roomid,title,created_at,updated_at,owner_wallet,deleted) VALUES(?1,'','','',?2,3)", params![orphan_room,s.db.owner()])?;
        for table in ["memos", "files"] {
            tx.execute(
                &format!("UPDATE {table} SET roomid=?1 WHERE owner_wallet=?2 AND roomid IS NULL"),
                params![orphan_room, s.db.owner()],
            )?;
        }
        tx.execute(
            "UPDATE rooms SET deleted=3 WHERE owner_wallet=?1",
            [s.db.owner()],
        )?;
        let rooms = tx
            .prepare("SELECT roomid FROM rooms WHERE owner_wallet=?1")?
            .query_map([s.db.owner()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let ids = tx
            .prepare("SELECT id FROM memos WHERE owner_wallet=?1")?
            .query_map([s.db.owner()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tx.commit()?;
        drop(conn);
        let mut continuous = s.continuous.lock().unwrap();
        for room in &rooms {
            if let Some(running) = continuous.remove(room) {
                running.task.abort();
            }
        }
        let mut jobs = s.jobs.lock().unwrap();
        let handles = ids
            .into_iter()
            .filter_map(|id| jobs.remove(&id))
            .collect::<Vec<_>>();
        (rooms, handles)
    };
    for handle in &handles {
        handle.abort();
    }
    for handle in handles {
        let _ = handle.await;
    }
    let state = s.clone();
    let cutoff = tokio::task::spawn_blocking(move || -> Result<i64> {
        // The durable marker rejects new writes. Drain writes that started before
        // it without holding the chat-submit or SQLite lock during filesystem work.
        let locks = state.owner_locks();
        let _projects = locks.projects.lock().unwrap();
        for room in rooms {
            crate::notes::cancel_room(&state, &room);
            finish_purge(&state.db, &room, true)?;
        }
        finish_user_purge(&state.db)
    })
    .await
    .map_err(anyhow::Error::from)??;
    Ok(Json(
        serde_json::json!({"deleted":true,"messagesyncid":cutoff}),
    ))
}

/// A file read or queued request may finish after another session clears data.
/// Check the client's observed erasure generation while holding chat_submit,
/// before creating any bytes or records. Missing fields support legacy callers.
/// The erasure generation a client must have observed to write: the sync
/// cursor of this account's last Delete all, or 0.
pub fn data_epoch(conn: &Connection, owner: &str) -> Result<i64> {
    Ok(conn.query_row("SELECT COALESCE(MAX(json_extract(data,'$.messagesyncid')),0) FROM notifications WHERE owner_wallet=?1 AND kind='data.deleted'",[owner],|r|r.get(0))?)
}
pub fn validate_epoch(
    database: &db::Database,
    room: Option<&str>,
    clear_epoch: Option<i64>,
    data_epoch: Option<i64>,
) -> Result<()> {
    let conn = database.0.lock().unwrap();
    if let Some(expected) = data_epoch {
        let current = self::data_epoch(&conn, database.owner())?;
        ensure!(
            expected == current,
            "Your workspace was erased. Refresh before sending new content."
        );
    }
    if let (Some(room), Some(expected)) = (room, clear_epoch) {
        let current: i64 = conn.query_row(
            "SELECT cleared_through FROM rooms WHERE roomid=?1 AND owner_wallet=?2",
            params![room, database.owner()],
            |r| r.get(0),
        )?;
        ensure!(
            expected == current,
            "This chatroom was cleared. Refresh before sending new content."
        );
    }
    Ok(())
}

pub fn finish_user_purge(database: &db::Database) -> Result<i64> {
    // The same finalizer runs after a normal deletion and after a restart.
    // Never clear the durable purge marker before projects are erased too.
    crate::coding::purge_owner(database)?;
    let mut conn = database.0.lock().unwrap();
    let tx = conn.transaction()?;
    let cutoff = next_sync_id(&tx)?;
    // Nothing derived outlives the data it was derived from: the Wiki's tags,
    // documents and code notes go, and so do the unfinished uploads.
    crate::wiki::forget_owner(&tx, database.owner())?;
    let parts = tx
        .prepare("SELECT temp_name FROM upload_sessions WHERE owner_wallet=?1")?
        .query_map([database.owner()], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for name in parts {
        if name.ends_with(".part")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            && !name.contains("..")
        {
            erase(&database.1.join("uploads").join(name))?;
        }
    }
    tx.execute(
        "DELETE FROM upload_sessions WHERE owner_wallet=?1",
        [database.owner()],
    )?;
    tx.execute(
        "DELETE FROM settings WHERE key=?1",
        [format!("thought.{}", database.owner())],
    )?;
    // Only identifiers and erasure cursors survive. Device erasure notifications
    // are retained separately until the capture clients have seen them.
    tx.execute(
        "DELETE FROM notifications WHERE owner_wallet=?1",
        [database.owner()],
    )?;
    tx.execute(
        "INSERT INTO notifications(owner_wallet,kind,data) VALUES(?1,'data.deleted',?2)",
        params![
            database.owner(),
            serde_json::json!({"messagesyncid":cutoff}).to_string()
        ],
    )?;
    tx.execute(
        "DELETE FROM user_purges WHERE owner_wallet=?1",
        [database.owner()],
    )?;
    tx.commit()?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(cutoff)
}

async fn purge_room(s: &Arc<AppState>, room: &str, delete: bool) -> Api<Json<serde_json::Value>> {
    let locks = s.owner_locks();
    let _erasure = locks.erasure.write().await;
    let handles = {
        let _lock = s.chat_submit.lock().unwrap();
        s.db.require("rooms", room)?;
        let status: i64 = s.db.0.lock().unwrap().query_row(
            "SELECT deleted FROM rooms WHERE roomid=?1",
            [room],
            |r| r.get(0),
        )?;
        if status != 0 {
            return Err(ApiError(
                if status == 1 {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::CONFLICT
                },
                "Chatroom was deleted or is being cleared".into(),
            ));
        }
        // A clearing marker prevents submissions/uploads while cancelled jobs unwind.
        s.db.0.lock().unwrap().execute(
            "UPDATE rooms SET deleted=?2 WHERE roomid=?1",
            params![room, if delete { 3 } else { 2 }],
        )?;
        if let Some(r) = s.continuous.lock().unwrap().remove(room) {
            r.task.abort();
        }
        let ids =
            s.db.0
                .lock()
                .unwrap()
                .prepare("SELECT id FROM memos WHERE roomid=?1")?
                .query_map([room], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut jobs = s.jobs.lock().unwrap();
        ids.into_iter()
            .filter_map(|id| jobs.remove(&id))
            .collect::<Vec<_>>()
    };
    for handle in &handles {
        handle.abort();
    }
    for handle in handles {
        let _ = handle.await;
    }
    let _lock = s.chat_submit.lock().unwrap();
    crate::notes::cancel_room(s, room);
    let cutoff = finish_purge(&s.db, room, delete)?;
    Ok(Json(
        serde_json::json!({"deleted":delete,"cleared":true,"messagesyncid":cutoff}),
    ))
}
pub fn finish_purge(database: &db::Database, room: &str, delete: bool) -> Result<i64> {
    let mut conn = database.0.lock().unwrap();
    let tx = conn.transaction()?;
    let state: i64 = tx.query_row("SELECT deleted FROM rooms WHERE roomid=?1", [room], |r| {
        r.get(0)
    })?;
    let delete = delete || matches!(state, 1 | 3);
    // Progress history belongs to the cleared conversation; nothing may recreate it.
    crate::activity::purge(&tx, room)?;
    if delete {
        tx.execute("DELETE FROM invisible_sessions WHERE id=?1", [room])?;
    } else {
        tx.execute("UPDATE invisible_sessions SET status=json_set(status,'$.active',json('false'),'$.approval',NULL,'$.error',NULL,'$.document','','$.stage','Ready to start'),updated_at=?2 WHERE id=?1", params![room,db::now()])?;
    }
    let memos = tx
        .prepare("SELECT id,image_path FROM memos WHERE roomid=?1")?
        .query_map([room], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let files = tx
        .prepare("SELECT id,path FROM files WHERE roomid=?1")?
        .query_map([room], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let ids = tx
        .prepare("SELECT messageid FROM chat_messages WHERE roomid=?1")?
        .query_map([room], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Delete original bytes before committing erasure of their durable references.
    for (id, image) in &memos {
        erase(&crate::artifacts::memo_path(
            &database.1,
            "thumbnails",
            id,
            "jpg",
        )?)?;
        if let Some(path) = image {
            erase(&crate::artifacts::resolve(&database.1, path)?)?;
        }
    }
    for (id, path) in &files {
        // A text file has no bytes on disk: it goes with its row.
        if !path.is_empty() {
            erase(&crate::artifacts::resolve(&database.1, path)?)?;
        }
        // A cached frame or preview of the file is the file's content too.
        erase(&crate::artifacts::resolve(
            &database.1,
            &format!("thumbnails/{id}.jpg"),
        )?)?;
    }
    let captures=tx.prepare("SELECT c.id,c.client_id FROM capture_requests c JOIN memos m ON m.id=c.memo_id WHERE m.roomid=?1")?.query_map([room],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (request, client) in captures {
        tx.execute(
            "INSERT INTO notifications(client_id,kind,data) VALUES(?1,'capture.remove',?2)",
            params![
                client,
                serde_json::json!({"request_id":request}).to_string()
            ],
        )?;
    }
    tx.execute("DELETE FROM chat_submissions WHERE messageid IN (SELECT messageid FROM chat_messages WHERE roomid=?1)",[room])?;
    tx.execute("UPDATE sync_clock SET value=MAX(value,(SELECT COALESCE(MAX(messagesyncid),0) FROM message_changes)) WHERE id=1",[])?;
    tx.execute("DELETE FROM notifications WHERE roomid=?1", [room])?;
    tx.execute("DELETE FROM message_changes WHERE roomid=?1", [room])?;
    tx.execute("DELETE FROM chat_messages WHERE roomid=?1", [room])?;
    tx.execute("DELETE FROM memos WHERE roomid=?1", [room])?;
    crate::wiki::sync_locked(&tx, database.owner(), false)?;
    tx.execute(
        "DELETE FROM compilations WHERE roomid=?1 OR file_id IN (SELECT id FROM files WHERE roomid=?1)",
        [room],
    )?;
    tx.execute("DELETE FROM files WHERE roomid=?1", [room])?;
    for id in ids {
        let mut marker = ChatMessage {
            roomid: room.into(),
            messageid: id,
            deleted: true,
            ..Default::default()
        };
        write(&tx, &mut marker)?;
    }
    tx.execute("UPDATE rooms SET deleted=?2,title=CASE WHEN ?2=1 THEN '' ELSE title END,created_at=CASE WHEN ?2=1 THEN '' ELSE created_at END,updated_at=?3 WHERE roomid=?1",params![room,delete,db::now()])?;
    let cutoff = next_sync_id(&tx)?;
    tx.execute(
        "UPDATE rooms SET cleared_through=?2 WHERE roomid=?1",
        params![room, cutoff],
    )?;
    let data = serde_json::json!({"roomid":room,"messagesyncid":cutoff});
    tx.execute(
        "INSERT INTO notifications(owner_wallet,roomid,kind,data) VALUES(?1,?2,?3,?4)",
        params![
            database.owner(),
            room,
            if delete {
                "room.deleted"
            } else {
                "room.cleared"
            },
            data.to_string()
        ],
    )?;
    tx.commit()?;
    // FTS5 merges drop obsolete tokens; checkpoint lets SQLite recycle erased WAL pages.
    conn.execute("INSERT INTO memo_fts(memo_fts) VALUES('optimize')", [])?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(cutoff)
}
fn erase(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub text: String,
    pub messagesyncid: i64,
}
pub async fn edit_message(
    PrivateState(s): PrivateState,
    Path((room, id)): Path<(String, String)>,
    Json(v): Json<Edit>,
) -> Api<Json<ChatMessage>> {
    if v.text.trim().is_empty() || v.text.len() > 100_000 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Write a message between 1 and 100,000 bytes".into(),
        ));
    }
    mutate(&s, &room, &id, Some(v.text), v.messagesyncid).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteMessage {
    pub messagesyncid: i64,
}
pub async fn delete_message(
    PrivateState(s): PrivateState,
    Path((room, id)): Path<(String, String)>,
    Json(v): Json<DeleteMessage>,
) -> Api<Json<ChatMessage>> {
    mutate(&s, &room, &id, None, v.messagesyncid).await
}
async fn mutate(
    s: &Arc<AppState>,
    room: &str,
    id: &str,
    text: Option<String>,
    expected: i64,
) -> Api<Json<ChatMessage>> {
    // Await cancellation before changing searchable memo content. A provider
    // callback already executing on another thread must not append after editing.
    let job = {
        let _lock = s.chat_submit.lock().unwrap();
        get_room(&s.db, room)?;
        let doc: Option<String> =
            s.db.0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT document FROM chat_messages WHERE roomid=?1 AND messageid=?2",
                    params![room, id],
                    |r| r.get(0),
                )
                .optional()?;
        let m: ChatMessage = serde_json::from_str(
            &doc.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Message not found".into()))?,
        )
        .map_err(anyhow::Error::from)?;
        if m.messagesyncid != expected {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "This message changed on another device. Review the latest version and retry."
                    .into(),
            ));
        }
        let job = s.jobs.lock().unwrap().remove(&m.memo_id);
        if let Some(job) = &job {
            job.abort();
        }
        job
    };
    if let Some(job) = job {
        let _ = job.await;
    }
    let _lock = s.chat_submit.lock().unwrap();
    get_room(&s.db, room)?;
    let mut conn = s.db.0.lock().unwrap();
    let tx = conn.transaction()?;
    let doc: Option<String> = tx
        .query_row(
            "SELECT document FROM chat_messages WHERE roomid=?1 AND messageid=?2",
            params![room, id],
            |r| r.get(0),
        )
        .optional()?;
    let mut m: ChatMessage = serde_json::from_str(
        &doc.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Message not found".into()))?,
    )
    .map_err(anyhow::Error::from)?;
    if m.messagesyncid != expected {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "This message changed on another device. Review the latest version and retry.".into(),
        ));
    }
    if m.deleted {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Message was removed".into(),
        ));
    }
    tx.execute(
        "UPDATE memos SET status='complete',error=NULL WHERE id=?1",
        [&m.memo_id],
    )?;
    refresh(&tx, &m.memo_id, "done", "")?;
    m.deleted = text.is_none();
    let delete_turn = m.deleted && m.role == "user";
    m.text = text.unwrap_or_default();
    m.edited_at = Some(db::now());
    m.status = "complete".into();
    m.stage = String::new();
    m.error = None;
    m.sources.clear();
    if m.deleted {
        if m.role == "user" {
            let image: Option<String> = tx.query_row(
                "SELECT image_path FROM memos WHERE id=?1",
                [&m.memo_id],
                |r| r.get(0),
            )?;
            if let Some(path) = image {
                erase(&crate::artifacts::resolve(&s.db.1, &path)?)?;
            }
            tx.execute(
                "UPDATE memos SET image=NULL,image_path=NULL WHERE id=?1",
                [&m.memo_id],
            )?;
            tx.execute("INSERT INTO notifications(client_id,kind,data) SELECT client_id,'capture.remove',json_object('request_id',id) FROM capture_requests WHERE memo_id=?1",[&m.memo_id])?;
            tx.execute(
                "DELETE FROM capture_requests WHERE memo_id=?1",
                [&m.memo_id],
            )?;
            for file in &m.attachments {
                // Preserve attachments that are still explicitly used by another
                // live message; unreferenced message-owned bytes are erased.
                let used: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM chat_messages c,json_each(c.document,'$.attachments') a WHERE c.messageid!=?1 AND json_extract(c.document,'$.deleted')=0 AND json_extract(a.value,'$.id')=?2)",params![id,file.id],|r|r.get(0))?;
                if !used {
                    let path: Option<String> = tx
                        .query_row(
                            "SELECT path FROM files WHERE id=?1 AND owner_wallet=?2",
                            params![file.id, s.db.owner()],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if let Some(path) = path {
                        if !path.is_empty() {
                            erase(&crate::artifacts::resolve(&s.db.1, &path)?)?;
                        }
                        tx.execute("DELETE FROM compilations WHERE file_id=?1", [&file.id])?;
                        tx.execute("DELETE FROM files WHERE id=?1", [&file.id])?;
                        crate::forget_thumbnail(&s.db.1, &file.id);
                    }
                }
            }
        }
        m.ocr_text.clear();
        m.transcript.clear();
        m.attachments.clear();
        m.image_path = None;
    }
    let column = if m.role == "user" { "input" } else { "answer" };
    tx.execute(
        &format!("UPDATE memos SET {column}=?2 WHERE id=?1"),
        params![m.memo_id, m.text],
    )?;
    if m.role == "user" {
        // Tags the user curated in the Wiki or the chat stay; hashtags written
        // into the new text join them. Otherwise the hashtags are the tags.
        let tags = crate::wiki::tags_after_edit(&tx, &m.memo_id, &m.text)?;
        tx.execute(
            "UPDATE memos SET title=?2,tags=?3 WHERE id=?1",
            params![
                m.memo_id,
                if m.deleted {
                    "Removed message".into()
                } else {
                    m.text.chars().take(80).collect::<String>()
                },
                serde_json::to_string(&tags).map_err(anyhow::Error::from)?
            ],
        )?;
    }
    // Erase stale job replay text; the durable chat revision is authoritative now.
    tx.execute("DELETE FROM events WHERE memo_id=?1", [&m.memo_id])?;
    let memo = m.memo_id.clone();
    if m.deleted {
        let peers = if delete_turn {
            tx.prepare("SELECT document FROM chat_messages WHERE memo_id=?1 AND messageid!=?2")?
                .query_map(params![memo, m.messageid], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            vec![]
        };
        // A response can repeat the prompt verbatim. Erase the entire generated
        // turn when its prompt is removed, and notify caches even outside this room.
        for doc in peers {
            let old: ChatMessage = serde_json::from_str(&doc).map_err(anyhow::Error::from)?;
            if !old.deleted {
                let mut removed = ChatMessage {
                    roomid: old.roomid,
                    messageid: old.messageid,
                    sequence: old.sequence,
                    deleted: true,
                    ..Default::default()
                };
                write(&tx, &mut removed)?;
                tx.execute("INSERT INTO notifications(owner_wallet,roomid,kind,data) VALUES(?1,?2,'message.deleted',?3)",params![s.db.owner(),room,serde_json::json!({"message":removed}).to_string()])?;
            }
        }
        let generated = tx
            .prepare("SELECT id,path FROM files WHERE origin_memo=?1 AND owner_wallet=?2")?
            .query_map(params![memo, s.db.owner()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (file, path) in generated {
            let reused:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM chat_messages c,json_each(c.document,'$.attachments') a WHERE c.memo_id!=?1 AND json_extract(c.document,'$.deleted')=0 AND json_extract(a.value,'$.id')=?2)",params![memo,file],|r|r.get(0))?;
            if !reused {
                if !path.is_empty() {
                    erase(&crate::artifacts::resolve(&s.db.1, &path)?)?;
                }
                tx.execute("DELETE FROM compilations WHERE file_id=?1", [&file])?;
                tx.execute("DELETE FROM files WHERE id=?1", [&file])?;
                crate::forget_thumbnail(&s.db.1, &file);
            }
        }
        let notes = tx
            .prepare("SELECT id FROM memos WHERE origin_memo=?1 AND owner_wallet=?2")?
            .query_map(params![memo, s.db.owner()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for note in notes {
            tx.execute("DELETE FROM memos WHERE id=?1", [&note])?;
        }
        tx.execute(
            "DELETE FROM chat_submissions WHERE messageid=?1",
            [&m.messageid],
        )?;
        m = ChatMessage {
            roomid: m.roomid,
            messageid: m.messageid,
            sequence: m.sequence,
            deleted: true,
            ..Default::default()
        };
    }
    write(&tx, &mut m)?;
    if m.deleted {
        tx.execute("INSERT INTO notifications(owner_wallet,roomid,kind,data) VALUES(?1,?2,'message.deleted',?3)",params![s.db.owner(),room,serde_json::json!({"message":m}).to_string()])?;
    }
    let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM chat_messages WHERE memo_id=?1 AND json_extract(document,'$.deleted')=0)",[&memo],|r|r.get(0))?;
    if !live {
        tx.execute("DELETE FROM memos WHERE id=?1", [&memo])?;
    }
    crate::wiki::sync_locked(&tx, s.db.owner(), false)?;
    tx.commit()?;
    conn.execute("INSERT INTO memo_fts(memo_fts) VALUES('optimize')", [])?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(conn);
    Ok(Json(m))
}
/// Memo/room deletion is a revision too, so offline clients cannot resurrect it.
pub fn tombstone_memo(tx: &Transaction<'_>, memo: &str) -> Result<()> {
    let docs = tx
        .prepare("SELECT document FROM chat_messages WHERE memo_id=?1")?
        .query_map([memo], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for doc in docs {
        let old: ChatMessage = serde_json::from_str(&doc)?;
        if old.deleted {
            continue;
        }
        let mut m = ChatMessage {
            roomid: old.roomid,
            messageid: old.messageid,
            sequence: old.sequence,
            deleted: true,
            ..Default::default()
        };
        write(tx, &mut m)?;
    }
    Ok(())
}

#[derive(Deserialize, Default)]
pub struct Cursor {
    #[serde(default)]
    pub after: i64,
}
pub fn changes(db: &db::Database, room: &str, after: i64) -> Result<MessageSync> {
    ensure!(after >= 0, "Sync cursor must be nonnegative");
    db.require("rooms", room)?;
    let conn = db.0.lock().unwrap();
    let cleared_through: i64 = conn.query_row(
        "SELECT cleared_through FROM rooms WHERE roomid=?1",
        [room],
        |r| r.get(0),
    )?;
    let mut stmt=conn.prepare("SELECT document FROM message_changes WHERE roomid=?1 AND messagesyncid>?2 ORDER BY messagesyncid LIMIT 128")?;
    let docs = stmt
        .query_map(params![room, after], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let messages = docs
        .into_iter()
        .map(|doc| serde_json::from_str::<ChatMessage>(&doc))
        .collect::<serde_json::Result<Vec<_>>>()?;
    let next = messages.last().map(|m| m.messagesyncid).unwrap_or(after);
    let has_more = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM message_changes WHERE roomid=?1 AND messagesyncid>?2)",
        params![room, next],
        |r| r.get(0),
    )?;
    Ok(MessageSync {
        messages,
        next: if has_more {
            next
        } else {
            next.max(cleared_through)
        },
        has_more,
        cleared_through,
    })
}
pub async fn sync(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Api<Json<MessageSync>> {
    Ok(Json(changes(&s.db, &room, cursor.after)?))
}
pub async fn events(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
    Query(cursor): Query<Cursor>,
    headers: HeaderMap,
) -> Api<impl axum::response::IntoResponse> {
    get_room(&s.db, &room)?;
    let mut after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(cursor.after);
    if after < 0 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Invalid cursor".into()));
    }
    let stream = async_stream::stream! {
        let mut cleared_through = after;
        yield Ok::<_,std::convert::Infallible>(Event::default().retry(Duration::from_secs(2)).comment("room sync"));
        loop {
            if crate::auth::authorize(&s, &headers,"GET","/api/rooms").is_err() { break; }
            match changes(&s.db,&room,after){
                Ok(batch)=>{
                    if batch.cleared_through > cleared_through {
                        cleared_through = batch.cleared_through;
                        yield Ok(Event::default().id(cleared_through.to_string()).json_data(serde_json::json!({"kind":"room.cleared","data":{"roomid":room,"messagesyncid":cleared_through}})).unwrap());
                    }
                    for m in batch.messages{after=m.messagesyncid;yield Ok(Event::default().id(after.to_string()).json_data(&m).unwrap());}
                    after = batch.next;
                    if batch.has_more{continue;}
                },
                Err(_)=>break,
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}
/// Broadcast updated references when a stored attachment is removed.
pub fn remove_attachment(
    tx: &Transaction<'_>,
    file: Option<&str>,
    memo: Option<&str>,
) -> Result<()> {
    let docs = tx
        .prepare("SELECT document FROM chat_messages")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for doc in docs {
        let mut m: ChatMessage = serde_json::from_str(&doc)?;
        let old = m.clone();
        if let Some(id) = file {
            m.attachments.retain(|f| f.id != id);
        }
        if memo == Some(m.memo_id.as_str()) {
            m.image_path = None;
        }
        if m != old {
            write(tx, &mut m)?;
        }
    }
    Ok(())
}
