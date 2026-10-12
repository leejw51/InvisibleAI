//! Durable, owner-scoped Invisible AI sessions. Browsers are control clients only.
use crate::{Api, ApiError, AppState, PrivateState, db, notes};
use anyhow::Result;
use axum::{Json, extract::Path, http::StatusCode};
pub use causewaybay_shared::{
    CreateInvisibleSession as Create, InvisibleSession as Session, RemotePrompt,
    UpdateInvisibleSession as Update,
};
use rusqlite::{Connection, OptionalExtension, params};

pub fn init(conn: &Connection) -> Result<()> {
    crate::schema::ensure(conn, "CREATE TABLE IF NOT EXISTS invisible_sessions(
        id TEXT PRIMARY KEY REFERENCES rooms(roomid), owner_wallet TEXT NOT NULL,
        client_id TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
        status TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS invisible_sessions_owner ON invisible_sessions(owner_wallet,created_at);")?;
    let rows = conn
        .prepare("SELECT id,status FROM invisible_sessions")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, value) in rows {
        let mut status: notes::NoteStatus = serde_json::from_str(&value)?;
        if status.active {
            status.active = false;
            status.stage = "Backend restarted. Start this session to resume.".into();
            status.approval = None;
            conn.execute(
                "UPDATE invisible_sessions SET status=?2,updated_at=?3 WHERE id=?1",
                params![id, serde_json::to_string(&status)?, db::now()],
            )?;
        }
    }
    Ok(())
}

pub fn saved_status(database: &db::Database, id: &str) -> Result<Option<notes::NoteStatus>> {
    let value: Option<String> = database
        .0
        .lock()
        .unwrap()
        .query_row(
            "SELECT status FROM invisible_sessions WHERE id=?1 AND owner_wallet=?2",
            params![id, database.owner()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
}

pub fn save_status(database: &db::Database, id: &str, status: &notes::NoteStatus) -> Result<()> {
    let mut saved = status.clone();
    // Approvals are one-use runtime capabilities; never restore them after restart.
    saved.approval = None;
    database.0.lock().unwrap().execute(
        "UPDATE invisible_sessions SET status=?3,updated_at=?4 WHERE id=?1 AND owner_wallet=?2",
        params![
            id,
            database.owner(),
            serde_json::to_string(&saved)?,
            db::now()
        ],
    )?;
    Ok(())
}

/// Bind a room to one capture node, or mark it as a session with an empty client
/// until its first capture binds one. The database lock serializes every entry point.
pub fn register(database: &db::Database, room: &str, client: &str) -> Result<()> {
    let conn = database.0.lock().unwrap();
    database.writable(&conn)?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM rooms WHERE roomid=?1 AND owner_wallet=?2 AND deleted=0)",
        params![room, database.owner()],
        |r| r.get(0),
    )?;
    anyhow::ensure!(exists, "Room no longer exists");
    let bound: Option<String> = conn
        .query_row(
            "SELECT client_id FROM invisible_sessions WHERE id=?1",
            [room],
            |r| r.get(0),
        )
        .optional()?;
    // An empty client keeps whatever binding exists; a named client may bind an
    // unbound session or match the existing one.
    anyhow::ensure!(
        client.is_empty()
            || bound
                .as_deref()
                .is_none_or(|id| id.is_empty() || id == client),
        "This session is bound to another capture computer. Create a new session for that computer."
    );
    let stamp = db::now();
    let status = notes::NoteStatus {
        client_id: client.into(),
        stage: "Ready to start".into(),
        ..Default::default()
    };
    conn.execute(
        "INSERT INTO invisible_sessions(id,owner_wallet,client_id,created_at,updated_at,status)
        VALUES(?1,?2,?3,?4,?4,?5) ON CONFLICT(id) DO UPDATE SET updated_at=excluded.updated_at,
        client_id=CASE WHEN invisible_sessions.client_id='' THEN excluded.client_id ELSE invisible_sessions.client_id END
        WHERE invisible_sessions.owner_wallet=excluded.owner_wallet",
        params![
            room,
            database.owner(),
            client,
            stamp,
            serde_json::to_string(&status).map_err(anyhow::Error::from)?
        ],
    )?;
    Ok(())
}

pub fn bound_client(database: &db::Database, room: &str) -> Result<Option<String>> {
    crate::chat::get_room(database, room)?;
    Ok(database
        .0
        .lock()
        .unwrap()
        .query_row(
            "SELECT client_id FROM invisible_sessions WHERE id=?1 AND owner_wallet=?2",
            params![room, database.owner()],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .filter(|id| !id.is_empty()))
}

fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Session not found".into())
}

pub fn read(s: &AppState, id: &str) -> Api<Session> {
    let mut session = {
        let conn = s.db.0.lock().unwrap();
        conn.query_row(
            "SELECT r.roomid,r.title,COALESCE(i.client_id,''),r.created_at,r.updated_at FROM rooms r
            LEFT JOIN invisible_sessions i ON r.roomid=i.id WHERE r.roomid=?1 AND r.owner_wallet=?2 AND r.deleted=0",
            params![id, s.db.owner()],
            |r| {
                Ok(Session {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    client_id: r.get(2)?,
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                    status: notes::NoteStatus::default(),
                    capture: Default::default(),
                    capture_count: 0,
                    memo_count: 0,
                    recent_captures: vec![],
                    activity: Default::default(),
                })
            },
        )
        .optional()?
        .ok_or_else(missing)?
    };
    session.status = notes::current_status(s, id)?;
    session.capture = crate::continuous::current_status(s, id)?;
    session.activity = crate::activity::read(&s.db, id)?;
    let conn = s.db.0.lock().unwrap();
    (session.capture_count, session.memo_count) = conn.query_row(
        "SELECT COUNT(image_path),COALESCE(SUM(kind='memo'),0) FROM memos WHERE roomid=?1 AND owner_wallet=?2",
        params![id, s.db.owner()], |r| Ok((r.get(0)?, r.get(1)?))
    )?;
    session.recent_captures = conn.prepare(
        "SELECT id FROM memos WHERE roomid=?1 AND owner_wallet=?2 AND image_path IS NOT NULL ORDER BY created_at DESC,rowid DESC LIMIT 4"
    )?.query_map(params![id, s.db.owner()], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(session)
}

pub async fn list(PrivateState(s): PrivateState) -> Api<Json<Vec<Session>>> {
    let ids =
        s.db.0
            .lock()
            .unwrap()
            .prepare(
                "SELECT r.roomid FROM rooms r
        WHERE r.owner_wallet=?1 AND r.deleted=0 ORDER BY r.created_at DESC,r.roomid",
            )?
            .query_map([s.db.owner()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    // A simultaneous deletion can remove one of these IDs between reads.
    let mut sessions = vec![];
    for id in ids {
        match read(&s, &id) {
            Ok(v) => sessions.push(v),
            Err(e) if e.0 == StatusCode::NOT_FOUND => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Json(sessions))
}

pub async fn create(PrivateState(s): PrivateState, Json(v): Json<Create>) -> Api<Json<Session>> {
    if !v.client_id.is_empty() {
        let actor = crate::auth::background_principal(&s)?;
        // Shared Macs may hold capture sessions; starting TextEdit replies checks enrollment.
        if !s.db.clients()?.iter().any(|c| {
            c.id == v.client_id && crate::transport::can_capture(c, actor.address.as_deref())
        }) {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "Choose a capture computer available to your account".into(),
            ));
        }
    }
    let _lock = s.chat_submit.lock().unwrap();
    let id = if let Some(room) = v.room_id {
        crate::chat::get_room(&s.db, &room)?;
        room
    } else {
        crate::chat::create_room(
            &s.db,
            crate::chat::NewRoom {
                roomid: None,
                title: v.title,
            },
        )?
        .roomid
    };
    register(&s.db, &id, &v.client_id)?;
    drop(_lock);
    if let Some(app) = v.app.as_deref().filter(|a| !a.is_empty()) {
        notes::set_app(&s, &id, app)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    if v.start && !v.client_id.is_empty() {
        // The session exists either way; a start failure is shown on its card.
        if let Err(error) = notes::start_worker(s.clone(), id.clone(), v.client_id.clone()) {
            let mut status = notes::current_status(&s, &id)?;
            status.error = Some(error.to_string());
            save_status(&s.db, &id, &status)?;
        }
    }
    Ok(Json(read(&s, &id)?))
}

pub async fn get(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Session>> {
    Ok(Json(read(&s, &id)?))
}
pub async fn update(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Json(v): Json<Update>,
) -> Api<Json<Session>> {
    read(&s, &id)?;
    if let Some(seconds) = v.interval_seconds {
        notes::set_interval(&s, &id, seconds)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    if let Some(app) = v.app {
        notes::set_app(&s, &id, &app)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    if let Some(read_screen) = v.read_screen {
        notes::set_read_screen(&s, &id, read_screen)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    Ok(Json(read(&s, &id)?))
}
/// Queue a request typed on a phone or tablet for the running worker.
pub async fn prompt(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Json(v): Json<RemotePrompt>,
) -> Api<(StatusCode, Json<Session>)> {
    read(&s, &id)?;
    notes::submit_prompt(&s, &id, &v.text)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((StatusCode::ACCEPTED, Json(read(&s, &id)?)))
}
pub async fn start(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Session>> {
    let session = read(&s, &id)?;
    notes::start_worker(s.clone(), id.clone(), session.client_id)?;
    Ok(Json(read(&s, &id)?))
}
pub async fn stop(PrivateState(s): PrivateState, Path(id): Path<String>) -> Api<Json<Session>> {
    read(&s, &id)?;
    let _ = notes::stop(PrivateState(s.clone()), Path(id.clone())).await?;
    Ok(Json(read(&s, &id)?))
}
pub async fn decide(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Json(v): Json<notes::Decision>,
) -> Api<(StatusCode, Json<Session>)> {
    read(&s, &id)?;
    // Acknowledgement is immediate. The task, its errors, and later stop all belong to the backend.
    let _job = notes::decide_worker(s.clone(), id.clone(), v)?;
    Ok((StatusCode::ACCEPTED, Json(read(&s, &id)?)))
}
