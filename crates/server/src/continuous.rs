//! One backend-owned capture scheduler per session. Manual requests use its queue.
use crate::{Api, ApiError, AppState, PrivateState, db, submit_think};
use axum::{
    Json,
    extract::Path,
    http::{HeaderMap, StatusCode},
};
use causewaybay_shared::{ContinuousRequest, ContinuousStatus, ThinkRequest};
use std::{sync::Arc, time::Duration};

pub struct Running {
    pub status: ContinuousStatus,
    pub task: tokio::task::AbortHandle,
    pub generation: String,
    pub memos: Vec<String>,
    manual: tokio::sync::mpsc::Sender<ThinkRequest>,
}

pub fn current_status(s: &AppState, room: &str) -> anyhow::Result<ContinuousStatus> {
    let bound = crate::invisible_sessions::bound_client(&s.db, room)?;
    let mut status = s
        .continuous
        .lock()
        .unwrap()
        .get(room)
        .map(|r| r.status.clone())
        .unwrap_or(ContinuousStatus {
            roomid: room.into(),
            ..Default::default()
        });
    status.client_ids = bound.into_iter().collect();
    status.capturing |= capture_in_flight(&s.db, room)?;
    Ok(status)
}

pub async fn status(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
) -> Api<Json<ContinuousStatus>> {
    Ok(Json(current_status(&s, &room)?))
}

pub async fn start(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
    headers: HeaderMap,
    Json(mut v): Json<ContinuousRequest>,
) -> Api<Json<ContinuousStatus>> {
    crate::chat::get_room(&s.db, &room)?;
    let actor = crate::auth::authorize(&s, &headers, "POST", "/api/rooms")?;
    if !(2..=300).contains(&v.interval_seconds) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Choose an interval from 2 to 300 seconds".into(),
        ));
    }
    // Cloud AI mode polls at most once a minute; unchanged screens never reach
    // a model in any mode (they are discarded before OCR or AI).
    v.interval_seconds = crate::notes::cloud_floor(&s, v.interval_seconds)?;
    if v.request.capture_mode.as_deref() == Some("region") {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Continuous capture uses the entire display".into(),
        ));
    }
    let online =
        s.db.clients()?
            .into_iter()
            .filter(|c| c.online && crate::transport::can_capture(c, actor.address.as_deref()))
            .collect::<Vec<_>>();
    if v.all {
        v.client_ids = online.iter().map(|c| c.id.clone()).collect();
    }
    if v.client_ids.is_empty() {
        v.client_ids = crate::invisible_sessions::bound_client(&s.db, &room)?
            .into_iter()
            .collect();
    }
    v.client_ids.sort();
    v.client_ids.dedup();
    if v.client_ids.len() != 1 || !online.iter().any(|c| c.id == v.client_ids[0]) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Choose exactly one connected capture computer for this session".into(),
        ));
    }
    let _submit = s.chat_submit.lock().unwrap();
    crate::chat::validate_epoch(
        &s.db,
        Some(&room),
        v.request.clear_epoch,
        v.request.data_epoch,
    )?;
    let mut active = s.continuous.lock().unwrap();
    if active.get(&room).is_some_and(|r| r.status.active) || capture_in_flight(&s.db, &room)? {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Capture is already running in this session".into(),
        ));
    }
    crate::invisible_sessions::register(&s.db, &room, &v.client_ids[0])?;
    crate::activity::log_with(
        &s.db.0.lock().unwrap(),
        s.db.owner(),
        &room,
        "capture",
        &format!("Polling started · every {} seconds", v.interval_seconds),
        "",
        "",
    )?;
    let status = ContinuousStatus {
        roomid: room.clone(),
        active: true,
        interval_seconds: v.interval_seconds,
        client_ids: v.client_ids.clone(),
        ..Default::default()
    };
    let generation = db::id();
    let g = generation.clone();
    let state = s.clone();
    let roomid = room.clone();
    let (manual, rx) = tokio::sync::mpsc::channel(1);
    let (ready, ready_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        if ready_rx.await.is_err() {
            return;
        }
        let result = run(&state, &roomid, &g, v, rx).await;
        let failed = result.is_err();
        let memos = {
            let mut active = state.continuous.lock().unwrap();
            if let Some(r) = active.get_mut(&roomid).filter(|r| r.generation == g) {
                r.status.active = false;
                r.status.capturing = false;
                r.status.capture_queued = false;
                r.status.next_capture_at = None;
                r.status.error = result.err().map(|e| e.to_string());
                r.memos.clone()
            } else {
                vec![]
            }
        };
        crate::activity::log(
            &state.db,
            &roomid,
            "capture",
            if failed {
                "Polling stopped after an error"
            } else {
                "Polling stopped"
            },
            "",
            "",
        );
        cancel_memos(&state, memos).await;
    });
    active.insert(
        room,
        Running {
            status: status.clone(),
            task: task.abort_handle(),
            generation,
            memos: vec![],
            manual,
        },
    );
    let _ = ready.send(());
    Ok(Json(status))
}

pub fn capture_in_flight(database: &db::Database, room: &str) -> anyhow::Result<bool> {
    Ok(database.0.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM memos WHERE roomid=?1 AND status='running' AND kind='capture')",
        [room], |r| r.get(0)
    )?)
}

/// Queue at most one manual capture. It replaces the pending timer and never overlaps a frame.
pub async fn capture_now(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
    Json(req): Json<ThinkRequest>,
) -> Api<Json<ContinuousStatus>> {
    crate::chat::get_room(&s.db, &room)?;
    crate::chat::validate_epoch(&s.db, Some(&room), req.clear_epoch, req.data_epoch)?;
    if !req.attachment_ids().is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Send attachments separately before capturing while polling".into(),
        ));
    }
    let mut active = s.continuous.lock().unwrap();
    let running = active
        .get_mut(&room)
        .filter(|r| r.status.active)
        .ok_or_else(|| {
            ApiError(
                StatusCode::CONFLICT,
                "Polling has stopped. Capture again to take a single screenshot.".into(),
            )
        })?;
    if req
        .client_id
        .as_ref()
        .is_some_and(|id| !running.status.client_ids.contains(id))
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "This session is bound to another capture computer".into(),
        ));
    }
    if running.status.capture_queued {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "A manual capture is already queued".into(),
        ));
    }
    running.manual.try_send(req).map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "A manual capture is already queued".into(),
        )
    })?;
    running.status.capture_queued = true;
    running.status.next_capture_at = None;
    let status = running.status.clone();
    drop(active);
    crate::activity::log(&s.db, &room, "capture", "Manual capture queued", "", "");
    Ok(Json(status))
}

async fn run(
    s: &Arc<AppState>,
    room: &str,
    generation: &str,
    v: ContinuousRequest,
    mut manual: tokio::sync::mpsc::Receiver<ThinkRequest>,
) -> anyhow::Result<()> {
    let mut next = v.request.clone();
    loop {
        {
            let mut active = s.continuous.lock().unwrap();
            let Some(r) = active
                .get_mut(room)
                .filter(|r| r.status.active && r.generation == generation)
            else {
                return Ok(());
            };
            r.status.capturing = true;
            r.status.next_capture_at = None;
        }
        let mut req = next;
        req.capture = true;
        req.file_id = None;
        req.file_ids.clear();
        req.room_id = Some(room.into());
        req.message_id = Some(db::id());
        req.client_id = Some(v.client_ids[0].clone());
        req.capture_mode = Some("screen".into());
        let actor = crate::auth::background_principal(s).map_err(|e| anyhow::anyhow!(e.1))?;
        let memo = submit_think(s.clone(), actor, req, true)
            .await
            .map_err(|e| anyhow::anyhow!(e.1))?
            .0;
        let alive = {
            let mut active = s.continuous.lock().unwrap();
            if let Some(r) = active
                .get_mut(room)
                .filter(|r| r.status.active && r.generation == generation)
            {
                r.memos = vec![memo.id.clone()];
                true
            } else {
                false
            }
        };
        if !alive {
            cancel_memos(s, vec![memo.id]).await;
            return Ok(());
        }
        loop {
            match s.db.get(&memo.id)? {
                Some(m) if m.status == "error" => {
                    anyhow::bail!("{}", m.error.unwrap_or_else(|| "Capture failed".into()))
                }
                Some(m) if m.status == "running" => {
                    tokio::time::sleep(Duration::from_millis(100)).await
                }
                _ => break,
            }
        }
        let delay = Duration::from_secs(v.interval_seconds);
        {
            let mut active = s.continuous.lock().unwrap();
            let Some(r) = active
                .get_mut(room)
                .filter(|r| r.status.active && r.generation == generation)
            else {
                return Ok(());
            };
            r.status.capturing = false;
            r.memos.clear();
            r.status.next_capture_at = (!r.status.capture_queued).then(|| {
                (chrono::Utc::now() + chrono::Duration::seconds(v.interval_seconds as i64))
                    .to_rfc3339()
            });
        }
        next = tokio::select! {
            biased;
            request = manual.recv() => {
                let Some(req) = request else { return Ok(()); };
                if let Some(r) = s.continuous.lock().unwrap().get_mut(room).filter(|r| r.generation == generation) {
                    r.status.capture_queued = false;
                    r.status.next_capture_at = None;
                }
                req
            }
            _ = tokio::time::sleep(delay) => v.request.clone(),
        };
    }
}
async fn cancel_memos(s: &Arc<AppState>, memos: Vec<String>) {
    for id in memos {
        if s.db
            .get(&id)
            .ok()
            .flatten()
            .is_some_and(|m| m.status == "running")
        {
            let _ = crate::cancel(PrivateState(s.clone()), Path(id)).await;
        }
    }
}
pub async fn stop(
    PrivateState(s): PrivateState,
    Path(room): Path<String>,
) -> Api<Json<ContinuousStatus>> {
    crate::chat::get_room(&s.db, &room)?;
    let memos = {
        let mut active = s.continuous.lock().unwrap();
        if let Some(r) = active.get_mut(&room) {
            r.status.active = false;
            r.status.capturing = false;
            r.status.capture_queued = false;
            r.status.next_capture_at = None;
            r.task.abort();
            r.memos.clone()
        } else {
            vec![]
        }
    };
    cancel_memos(&s, memos).await;
    status(PrivateState(s), Path(room)).await
}
