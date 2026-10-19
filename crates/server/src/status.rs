//! Counts only: system oversight never returns another wallet's saved content.
use crate::{Api, AppState, PrivateState};
use anyhow::Result;
use axum::{Json, http::HeaderMap};
use causewaybay_shared::{
    CaptureClient, Dashboard, SystemStatus, UserStats, WorkspaceStats, WorkspaceStatus,
};
use rusqlite::params;

pub fn stats(s: &AppState) -> Result<WorkspaceStats> {
    let conn = s.db.0.lock().unwrap();
    let owner = &s.db.2;
    let count = |sql: &str| -> rusqlite::Result<u64> { conn.query_row(sql, [owner], |r| r.get(0)) };
    let mut result = WorkspaceStats {
        rooms: count(
            "SELECT COUNT(*) FROM rooms WHERE deleted=0 AND (?1 IS NULL OR owner_wallet=?1)",
        )?,
        messages: count(
            "SELECT COUNT(*) FROM chat_messages c JOIN rooms r USING(roomid) WHERE r.deleted=0 AND COALESCE(json_extract(c.document,'$.deleted'),0)=0 AND (?1 IS NULL OR r.owner_wallet=?1)",
        )?,
        memos: count("SELECT COUNT(*) FROM memos WHERE (?1 IS NULL OR owner_wallet=?1)")?,
        pinned: count(
            "SELECT COUNT(*) FROM memos WHERE pinned=1 AND (?1 IS NULL OR owner_wallet=?1)",
        )?,
        files: count("SELECT COUNT(*) FROM files WHERE (?1 IS NULL OR owner_wallet=?1)")?,
        images: count(
            "SELECT (SELECT COUNT(*) FROM files WHERE mime LIKE 'image/%' AND (?1 IS NULL OR owner_wallet=?1)) + (SELECT COUNT(*) FROM memos WHERE image_path IS NOT NULL AND (?1 IS NULL OR owner_wallet=?1))",
        )?,
        file_bytes: count(
            "SELECT COALESCE(SUM(size),0) FROM files WHERE (?1 IS NULL OR owner_wallet=?1)",
        )?,
        running: count(
            "SELECT COUNT(*) FROM memos WHERE status='running' AND (?1 IS NULL OR owner_wallet=?1)",
        )?,
        failed: count(
            "SELECT COUNT(*) FROM memos WHERE status='error' AND (?1 IS NULL OR owner_wallet=?1)",
        )?,
        queued_captures: count(
            "SELECT COUNT(*) FROM capture_requests c JOIN memos m ON m.id=c.memo_id WHERE c.status IN ('queued','capturing') AND m.status='running' AND (?1 IS NULL OR m.owner_wallet=?1)",
        )?,
        rust_total: count(
            "SELECT COUNT(*) FROM capture_clients WHERE (?1 IS NULL OR owner_wallet=?1 OR owner_wallet IS NULL)",
        )?,
        web_total: count("SELECT COUNT(*) FROM web_clients WHERE (?1 IS NULL OR owner_wallet=?1)")?,
        ..Default::default()
    };
    let now = chrono::Utc::now().timestamp();
    result.rust_online = conn.query_row("SELECT COUNT(*) FROM capture_clients WHERE last_seen>?2-45 AND (?1 IS NULL OR owner_wallet=?1 OR owner_wallet IS NULL)", params![owner,now], |r| r.get(0))?;
    result.web_online = conn.query_row("SELECT COUNT(*) FROM web_clients WHERE connected=1 AND last_seen>?2-35 AND (?1 IS NULL OR owner_wallet=?1)", params![owner,now], |r| r.get(0))?;
    for path in conn.prepare("SELECT image_path FROM memos WHERE image_path IS NOT NULL AND (?1 IS NULL OR owner_wallet=?1)")?.query_map([owner], |r| r.get::<_,String>(0))? {
        let path = crate::artifacts::resolve(&s.data_dir, &path?)?;
        match std::fs::metadata(path) {
            Ok(info) => result.screenshot_bytes += info.len(),
            Err(e) if e.kind()==std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
        }
    }
    Ok(result)
}

fn connections(s: &AppState) -> Result<Dashboard> {
    let conn = s.db.0.lock().unwrap();
    let now = chrono::Utc::now().timestamp();
    let rust_clients=conn.prepare("SELECT id,name,platform,last_seen,owner_wallet FROM capture_clients WHERE (?1 IS NULL OR owner_wallet=?1 OR owner_wallet IS NULL) ORDER BY last_seen DESC")?
        .query_map([&s.db.2],|r|{let seen=r.get::<_,i64>(3)?;Ok(CaptureClient{id:r.get(0)?,name:r.get(1)?,platform:r.get(2)?,last_seen:seen,online:now-seen<45,owner_wallet:r.get(4)?,can_capture:false,textedit:Default::default(),audio:false})})?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let web_clients=conn.prepare("SELECT id,name,platform,last_seen,connected FROM web_clients WHERE (?1 IS NULL OR owner_wallet=?1) ORDER BY connected DESC,last_seen DESC")?
        .query_map([&s.db.2],|r|{let seen=r.get::<_,i64>(3)?;Ok(CaptureClient{id:r.get(0)?,name:r.get(1)?,platform:r.get(2)?,last_seen:seen,online:r.get::<_,bool>(4)?&&now-seen<35,owner_wallet:None,can_capture:false,textedit:Default::default(),audio:false})})?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Dashboard {
        rust_connected: rust_clients.iter().filter(|c| c.online).count(),
        web_connected: web_clients.iter().filter(|c| c.online).count(),
        rust_clients,
        web_clients,
        addresses: s.addresses.get().cloned().unwrap_or_default(),
        uptime_seconds: s.started.elapsed().as_secs(),
        server_version: env!("CARGO_PKG_VERSION").into(),
    })
}

pub async fn get(PrivateState(s): PrivateState, headers: HeaderMap) -> Api<Json<WorkspaceStatus>> {
    let principal = crate::auth::principal(&s, &headers)?;
    // Holding the submission gate yields a consistent snapshot across counts and erasure.
    let _gate = s.chat_submit.lock().unwrap();
    let personal = stats(&s)?;
    let personal_connections = connections(&s)?;
    let system = if principal.admin {
        let mut global = (*s).clone();
        global.db.2 = None;
        let users = {
            let conn = s.db.0.lock().unwrap();
            let mut result = UserStats::default();
            for row in conn.prepare("SELECT role,COUNT(*) FROM (SELECT address,CASE WHEN address=?1 THEN 'admin' ELSE role END role FROM users UNION SELECT ?1,'admin' WHERE ?1 IS NOT NULL) GROUP BY role")?.query_map([&s.auth.admin], |r| Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?)))? {
                let (role,count) = row?;
                result.total += count;
                match role.as_str() {
                    "admin" => result.admins += count,
                    "user" => result.approved += count,
                    "pending" => result.pending += count,
                    "blocked" => result.blocked += count,
                    _ => {},
                }
            }
            result
        };
        Some(SystemStatus {
            stats: stats(&global)?,
            users,
            connections: connections(&global)?,
            providers: s.settings()?.providers,
            semantic_status: s.semantic.status.lock().unwrap().clone(),
            demo: s.demo,
        })
    } else {
        None
    };
    Ok(Json(WorkspaceStatus {
        stats: personal,
        connections: personal_connections,
        system,
        package: crate::client_package::info(&s, principal.address),
        updated_at: chrono::Utc::now().to_rfc3339(),
    }))
}
