//! Durable client presence, with connection IDs to reject late disconnects.
use crate::PrivateState;
use crate::{Api, AppState, db};
use anyhow::{Result, ensure};
use axum::Json;
use causewaybay_shared::{CaptureClient, Dashboard};
use rusqlite::params;
use std::sync::Arc;

impl db::Database {
    pub fn web_connect(
        &self,
        id: &str,
        name: &str,
        platform: &str,
        connection: &str,
    ) -> Result<()> {
        ensure!(
            !id.is_empty() && id.len() <= 100 && name.len() <= 120 && platform.len() <= 60,
            "Invalid web client identity"
        );
        self.0.lock().unwrap().execute("INSERT INTO web_clients(id,name,platform,connection_id,connected,last_seen,owner_wallet) VALUES(?1,?2,?3,?4,1,?5,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,platform=excluded.platform,connection_id=excluded.connection_id,connected=1,last_seen=excluded.last_seen,owner_wallet=excluded.owner_wallet", params![id,name,platform,connection,chrono::Utc::now().timestamp(),self.owner()])?;
        Ok(())
    }
    pub fn web_heartbeat(&self, id: &str, connection: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE web_clients SET last_seen=?3 WHERE id=?1 AND connection_id=?2 AND connected=1",
            params![id, connection, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }
    pub fn web_disconnect(&self, id: &str, connection: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE web_clients SET connected=0 WHERE id=?1 AND connection_id=?2",
            params![id, connection],
        )?;
        Ok(())
    }
    pub fn web_clients(&self) -> Result<Vec<CaptureClient>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,name,platform,connected,last_seen FROM web_clients WHERE (?1 IS NULL OR owner_wallet=?1) ORDER BY connected DESC,last_seen DESC LIMIT 100")?;
        Ok(stmt
            .query_map([&self.2], |r| {
                let last_seen = r.get::<_, i64>(4)?;
                Ok(CaptureClient {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    platform: r.get(2)?,
                    online: r.get::<_, bool>(3)? && chrono::Utc::now().timestamp() - last_seen < 35,
                    last_seen,
                    owner_wallet: None,
                    can_capture: false,
                    textedit: Default::default(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}

pub fn snapshot(s: &AppState) -> Result<Dashboard> {
    let rust_clients = s.db.clients()?;
    let web_clients = s.db.web_clients()?;
    Ok(Dashboard {
        addresses: s.addresses.get().cloned().unwrap_or_default(),
        rust_connected: rust_clients.iter().filter(|c| c.online).count(),
        web_connected: web_clients.iter().filter(|c| c.online).count(),
        rust_clients,
        web_clients,
        uptime_seconds: s.started.elapsed().as_secs(),
        server_version: env!("CARGO_PKG_VERSION").into(),
    })
}
pub async fn dashboard(
    PrivateState(s): PrivateState,
    headers: axum::http::HeaderMap,
) -> Api<Json<Dashboard>> {
    let p = crate::auth::principal(&s, &headers)?;
    let mut d = snapshot(&s)?;
    for c in &mut d.rust_clients {
        crate::transport::client_permissions(&s, c, p.address.as_deref());
    }
    Ok(Json(d))
}

/// Every socket exit path, including task cancellation, marks its client offline.
pub struct Presence {
    state: Arc<AppState>,
    id: String,
    connection: Option<String>,
}
impl Presence {
    pub fn new(state: Arc<AppState>, id: String, connection: Option<String>) -> Self {
        let this = Self {
            state,
            id,
            connection,
        };
        this.report("connected");
        this
    }
    fn report(&self, action: &str) {
        if let Ok(d) = snapshot(&self.state) {
            println!(
                "  Connections     Rust clients: {} · Web clients: {} ({} client {action})",
                d.rust_connected,
                d.web_connected,
                if self.connection.is_some() {
                    "web"
                } else {
                    "Rust"
                }
            );
        }
    }
}
impl Drop for Presence {
    fn drop(&mut self) {
        if let Some(connection) = &self.connection {
            let _ = self.state.db.web_disconnect(&self.id, connection);
        } else {
            let _ = self.state.db.release_captures(&self.id);
        }
        self.report("disconnected");
    }
}

pub fn report_capture(s: &AppState, action: &str) {
    if let Ok(d) = snapshot(s) {
        println!(
            "  Connections     Rust clients: {} · Web clients: {} (Rust client {action})",
            d.rust_connected, d.web_connected
        );
    }
}
