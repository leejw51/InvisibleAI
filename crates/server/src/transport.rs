//! JSON WebSocket transport for both the browser and the capture CLI.
use crate::{Api, ApiError, AppState, PrivateState, db};
use anyhow::{Result, ensure};
use axum::{
    body::Body,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, Request, StatusCode},
    response::{
        IntoResponse, Sse,
        sse::{Event, KeepAlive},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use causewaybay_shared::{CaptureClient, CaptureCommand};
use futures_util::{SinkExt, StreamExt};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tower::ServiceExt;

impl db::Database {
    pub fn client_hello(&self, id: &str, name: &str, platform: &str) -> Result<()> {
        ensure!(
            id.len() <= 80 && name.len() <= 120 && platform.len() <= 60 && !id.is_empty(),
            "Invalid client identity"
        );
        let desired = self.get_setting(&format!("node.{id}"))?;
        let name = desired.as_deref().unwrap_or(name);
        self.0.lock().unwrap().execute("INSERT INTO capture_clients(id,name,platform,last_seen) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,platform=excluded.platform,last_seen=excluded.last_seen",params![id,name,platform,chrono::Utc::now().timestamp()])?;
        Ok(())
    }
    pub fn heartbeat(&self, id: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE capture_clients SET last_seen=?2 WHERE id=?1",
            params![id, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }
    pub fn clients(&self) -> Result<Vec<CaptureClient>> {
        let conn = self.0.lock().unwrap();
        let mut stmt=conn.prepare("SELECT id,name,platform,last_seen,owner_wallet FROM capture_clients WHERE (?1 IS NULL OR ?1='local' OR owner_wallet=?1 OR owner_wallet IS NULL) ORDER BY last_seen DESC LIMIT 50")?;
        Ok(stmt
            .query_map([&self.2], |r| {
                let last_seen = r.get::<_, i64>(3)?;
                Ok(CaptureClient {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    platform: r.get(2)?,
                    last_seen,
                    owner_wallet: r.get(4)?,
                    can_capture: true,
                    textedit: Default::default(),
                    online: chrono::Utc::now().timestamp() - last_seen < 45,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn queue_capture(
        &self,
        client: &str,
        memo: &str,
        mode: &str,
        display: u32,
    ) -> Result<String> {
        self.queue_capture_for(client, memo, mode, display, None)
    }
    pub fn queue_capture_for(
        &self,
        client: &str,
        memo: &str,
        mode: &str,
        display: u32,
        requester: Option<&str>,
    ) -> Result<String> {
        let id = db::id();
        self.0.lock().unwrap().execute("INSERT INTO capture_requests(id,client_id,memo_id,mode,display,created_at,requester) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,client,memo,mode,display,chrono::Utc::now().timestamp(),requester])?;
        Ok(id)
    }
    pub fn claim_capture(&self, client: &str) -> Result<Option<CaptureCommand>> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let found=tx.query_row("SELECT c.id,c.mode,c.display FROM capture_requests c JOIN memos m ON m.id=c.memo_id JOIN capture_clients n ON n.id=c.client_id WHERE c.client_id=?1 AND (n.owner_wallet IS NULL OR n.owner_wallet=c.requester) AND c.status='queued' AND m.status='running' ORDER BY c.created_at LIMIT 1",[client],|r|Ok(CaptureCommand{kind:"capture".into(),request_id:r.get(0)?,mode:r.get(1)?,display:r.get(2)?})).optional()?;
        if let Some(c) = &found {
            tx.execute(
                "UPDATE capture_requests SET status='capturing' WHERE id=?1",
                [&c.request_id],
            )?;
        }
        tx.commit()?;
        Ok(found)
    }
    pub fn complete_capture(
        &self,
        client: &str,
        request: &str,
        image: Option<&[u8]>,
        error: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let memo:String=tx.query_row("SELECT c.memo_id FROM capture_requests c JOIN memos m ON m.id=c.memo_id WHERE c.id=?1 AND c.client_id=?2 AND c.status='capturing' AND m.status='running'",params![request,client],|r|r.get(0))?;
        if let Some(bytes) = image {
            let path = self.persist_image(&memo, bytes)?;
            tx.execute(
                "UPDATE memos SET image=NULL,image_path=?2 WHERE id=?1",
                params![memo, path],
            )?;
        }
        tx.execute(
            "UPDATE capture_requests SET status=?2,error=?3 WHERE id=?1",
            params![
                request,
                if error.is_some() { "error" } else { "ready" },
                error
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn capture_status(&self, id: &str) -> Result<(String, Option<String>)> {
        Ok(self.0.lock().unwrap().query_row(
            "SELECT status,error FROM capture_requests WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }
    pub fn release_captures(&self, client: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE capture_requests SET status='queued' WHERE client_id=?1 AND status='capturing'",
            [client],
        )?;
        self.0.lock().unwrap().execute(
            "UPDATE capture_clients SET last_seen=0 WHERE id=?1",
            [client],
        )?;
        Ok(())
    }
}

pub fn can_capture(c: &CaptureClient, wallet: Option<&str>) -> bool {
    c.owner_wallet
        .as_deref()
        .is_none_or(|owner| wallet == Some(owner))
}
pub fn client_permissions(s: &AppState, c: &mut CaptureClient, wallet: Option<&str>) {
    use causewaybay_shared::TextEditAvailability::*;
    c.can_capture = can_capture(c, wallet);
    c.textedit = if !c.can_capture || !c.platform.eq_ignore_ascii_case("macos") {
        Unavailable
    } else if s.auth.admin.is_some() && (wallet.is_none() || c.owner_wallet.as_deref() != wallet) {
        NeedsEnrollment
    } else if !c.online {
        Offline
    } else if !s.notes.capable.lock().unwrap().contains_key(&c.id) {
        UpdateRequired
    } else {
        Ready
    };
}
pub async fn clients(PrivateState(s): PrivateState, headers: HeaderMap) -> Api<impl IntoResponse> {
    let p = crate::auth::principal(&s, &headers)?;
    let mut clients = s.db.clients()?;
    for c in &mut clients {
        client_permissions(&s, c, p.address.as_deref());
    }
    Ok(axum::Json(clients))
}
pub async fn browser_upgrade(
    PrivateState(s): PrivateState,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
) -> impl IntoResponse {
    ws.max_message_size(36 * 1024 * 1024)
        .max_frame_size(36 * 1024 * 1024)
        .on_upgrade(move |socket| browser_socket(s, socket, headers))
}

async fn browser_socket(s: Arc<AppState>, mut socket: WebSocket, headers: HeaderMap) {
    let Ok(Some(Ok(Message::Text(text)))) =
        tokio::time::timeout(Duration::from_secs(10), socket.recv()).await
    else {
        return;
    };
    let Ok(hello) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    if hello["type"] != "hello" {
        return;
    }
    let client = hello["id"].as_str().unwrap_or("").to_string();
    let connection = db::id();
    if s.db
        .web_connect(
            &client,
            hello["name"].as_str().unwrap_or("Web client"),
            hello["platform"].as_str().unwrap_or("Browser"),
            &connection,
        )
        .is_err()
    {
        return;
    }
    let _presence =
        crate::connections::Presence::new(s.clone(), client.clone(), Some(connection.clone()));
    // The erasure stamp lives here, on the backend's side of this connection:
    // what this page has been told about Delete all. Never the browser's word.
    let epoch = Arc::new(std::sync::atomic::AtomicI64::new(
        connection_epoch(&s, &headers).unwrap_or(0),
    ));
    if socket
        .send(Message::Text(
            json!({"type":"ready","client_id":client})
                .to_string()
                .into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(64);
    let mut subscriptions = HashMap::<String, i64>::new();
    let mut rooms = HashMap::<String, i64>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(120));
    let mut tasks = tokio::task::JoinSet::new();
    let mut last_received = tokio::time::Instant::now();
    loop {
        if crate::auth::authorize(&s, &headers, "GET", "/api/ws").is_err() {
            break;
        }
        tokio::select! {
            Some(message)=rx.recv()=>{if sink.send(Message::Text(message.to_string().into())).await.is_err(){break;}},
            incoming=stream.next()=>{
                let Some(Ok(Message::Text(text)))=incoming else{break};
                let Ok(v)=serde_json::from_str::<Value>(&text)else{continue};
                last_received=tokio::time::Instant::now();
                let _=s.db.web_heartbeat(&client,&connection);
                match v["type"].as_str().unwrap_or(""){
                    "heartbeat"=>{if sink.send(Message::Text(json!({"type":"heartbeat"}).to_string().into())).await.is_err(){break;}},
                    "request"=>{
                        let id=v["id"].clone();let method=v["method"].as_str().unwrap_or("GET").to_string();let path=v["path"].as_str().unwrap_or("").to_string();
                        if tasks.len()>=32{let _=tx.send(json!({"type":"response","id":id,"status":429,"data":{"error":"Too many requests"}})).await;continue;}
                        let tx=tx.clone();let s=s.clone();let body=v["body"].clone();let headers=headers.clone();let epoch=epoch.clone();
                        tasks.spawn(async move{
                            let response=dispatch_stamped(s,&method,&path,body,headers,&epoch).await;
                            let(status,data)=match response{Ok(r)=>r,Err(e)=>(e.0.as_u16(),json!({"error":e.1}))};
                            let _=tx.send(json!({"type":"response","id":id,"status":status,"data":data})).await;
                        });
                    },
                    "subscribe"=>{if let Some(id)=v["memo_id"].as_str() && subscriptions.len()<8 && s.db.require("memos",id).is_ok() {subscriptions.insert(id.into(),v["after"].as_i64().unwrap_or(0));}},
                    "subscribe_room"=>{if let Some(id)=v["room_id"].as_str() && rooms.len()<8 && s.db.require("rooms",id).is_ok() {rooms.insert(id.into(),v["after"].as_i64().filter(|n|*n>=0).unwrap_or(0));}},
                    "unsubscribe_room"=>{if let Some(id)=v["room_id"].as_str(){rooms.remove(id);}},
                    "unsubscribe"=>{if let Some(id)=v["memo_id"].as_str(){subscriptions.remove(id);}},
                    _=>{}
                }
            },
            _=tick.tick()=>{
                if last_received.elapsed()>Duration::from_secs(35){break;}
                for (room,cursor) in &mut rooms {
                    if let Ok(batch)=crate::chat::changes(&s.db,room,*cursor) {
                        for m in batch.messages {*cursor=m.messagesyncid;
                            if sink.send(Message::Text(json!({"type":"room_event","room_id":room,"message":m}).to_string().into())).await.is_err(){return;}
                        }
                    }
                }
                let mut finished=vec![];
                for (id,cursor) in &mut subscriptions{
                    match s.db.events(id,*cursor){Ok(events)=>{for e in events{*cursor=e.id;
                        if e.kind=="done"||e.kind=="error"{finished.push(id.clone());}
                        if sink.send(Message::Text(json!({"type":"event","memo_id":id,"event":e}).to_string().into())).await.is_err(){return;}
                    }},Err(_)=>{finished.push(id.clone());}}
                }
                for id in finished{subscriptions.remove(&id);}
            },
            Some(_)=tasks.join_next()=>{}
        }
    }
}

/// The erasure generation the account is at when a page connects.
fn connection_epoch(s: &AppState, headers: &HeaderMap) -> Option<i64> {
    let owner = crate::auth::principal(s, headers).ok()?.address;
    let conn = s.db.0.lock().unwrap();
    crate::chat::data_epoch(&conn, owner.as_deref().unwrap_or("local")).ok()
}
/// Write the stamp into the request's body, over anything the browser sent,
/// then dispatch. `request` is the nested capture request of a batch. A
/// Delete all through this connection moves its stamp forward at once.
pub(crate) async fn dispatch_stamped(
    s: Arc<AppState>,
    method: &str,
    path: &str,
    body: Value,
    headers: HeaderMap,
    epoch: &std::sync::atomic::AtomicI64,
) -> Api<(u16, Value)> {
    let current = epoch.load(std::sync::atomic::Ordering::SeqCst);
    let body = stamped_body(method, path, body, current);
    let result = dispatch(s, method, path, body, headers).await;
    if let Ok((200, data)) = &result
        && method == "DELETE"
        && path == "/api/user/data"
        && let Some(cutoff) = data["messagesyncid"].as_i64()
    {
        epoch.store(cutoff, std::sync::atomic::Ordering::SeqCst);
    }
    result
}
/// The request body as the handler will see it: the connection's stamp written
/// in for content-creating writes (inside the capture request of a batch,
/// whose envelope is strict), everything else untouched.
fn stamped_body(method: &str, path: &str, mut body: Value, epoch: i64) -> Value {
    if !stamped_path(method, path) {
        return body;
    }
    if body.is_null() {
        body = json!({});
    }
    // A capture batch and a polling session start wrap their capture request
    // in a strict envelope; the request inside carries the stamp.
    let nested = path == "/api/capture/batch"
        || (path.starts_with("/api/rooms/") && path.ends_with("/capture"));
    if nested {
        if let Some(request) = body.get_mut("request").filter(|v| v.is_object()) {
            stamp(request, epoch);
        }
    } else if body.is_object() {
        stamp(&mut body, epoch);
    }
    body
}
/// The writes that create content and therefore check the erasure stamp.
fn stamped_path(method: &str, path: &str) -> bool {
    method == "POST"
        && (matches!(
            path,
            "/api/think" | "/api/capture/batch" | "/api/files" | "/api/uploads"
        ) || (path.starts_with("/api/rooms/")
            && (path.ends_with("/snapshot")
                || path.ends_with("/capture")
                || path.ends_with("/capture/now"))))
}
fn stamp(body: &mut Value, epoch: i64) {
    if let Some(map) = body.as_object_mut() {
        map.remove("dataEpoch");
        map.insert("data_epoch".into(), json!(epoch));
    }
}
async fn dispatch(
    s: Arc<AppState>,
    method: &str,
    path: &str,
    body: Value,
    headers: HeaderMap,
) -> Api<(u16, Value)> {
    crate::auth::authorize(&s, &headers, method, path)?;
    // Strip exactly one "/api" so the authorized path and the routed path agree;
    // a repeated prefix such as "/api/api/admin/users" must not reach "/admin/users".
    let inner = path.strip_prefix("/api").filter(|p| p.starts_with('/'));
    let Some(inner) = inner else {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Unsupported WebSocket operation".into(),
        ));
    };
    if inner.starts_with("/api/")
        || path.contains("/events")
        || path.contains("/download")
        || path.contains("/export")
        || path.ends_with("/image")
        || path.ends_with("/thumbnail")
        || path.contains("/ws")
        || !matches!(method, "GET" | "POST" | "PATCH" | "DELETE")
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Unsupported WebSocket operation".into(),
        ));
    }
    let mut request = Request::builder()
        .method(method)
        .uri(inner)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Invalid request".into()))?;
    *request.headers_mut() = headers;
    request
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    // The same guard as HTTP re-authorizes the routed path, so plain-State
    // handlers stay protected even if normalization above ever regresses.
    let response = crate::api_routes()
        .route_layer(axum::middleware::from_fn_with_state(
            s.clone(),
            crate::routes::guard,
        ))
        .with_state(s)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 36 * 1024 * 1024)
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Response too large".into(),
            )
        })?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(json!({"error":"Unexpected response"})),
    ))
}

pub async fn client_upgrade(
    State(s): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.max_message_size(36 * 1024 * 1024)
        .max_frame_size(36 * 1024 * 1024)
        .on_upgrade(move |socket| client_socket(s, socket))
}
/// Bind an enrolled ID to its device secret before changing name, owner or session.
fn register_device(
    s: &AppState,
    id: &str,
    name: &str,
    platform: &str,
    owner: Option<&str>,
    key: Option<&str>,
) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= 80 && name.len() <= 120 && platform.len() <= 60,
        "Invalid client identity"
    );
    ensure!(
        s.demo || key.is_some_and(|k| k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit())),
        "Device credential required"
    );
    let key_hash = key.map(crate::auth::hash);
    let desired = s.db.get_setting(&format!("node.{id}"))?;
    let name = desired.as_deref().unwrap_or(name);
    let mut conn = s.db.0.lock().unwrap();
    let tx = conn.transaction()?;
    let existing: Option<Option<String>> = tx
        .query_row(
            "SELECT device_key_hash FROM capture_clients WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(Some(expected)) = existing {
        ensure!(
            key_hash
                .as_ref()
                .is_some_and(|actual| crate::constant_eq(actual, &expected)),
            "Device credential mismatch"
        );
    }
    tx.execute("INSERT INTO capture_clients(id,name,platform,last_seen,owner_wallet,device_key_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,platform=excluded.platform,last_seen=excluded.last_seen,owner_wallet=excluded.owner_wallet,device_key_hash=excluded.device_key_hash", params![id,name,platform,chrono::Utc::now().timestamp(),owner,key_hash])?;
    tx.commit()?;
    Ok(())
}
async fn client_socket(s: Arc<AppState>, mut socket: WebSocket) {
    let hello = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = hello else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    if v["type"] != "hello" {
        return;
    }
    let id = v["id"].as_str().unwrap_or("").to_string();
    let name = v["name"].as_str().unwrap_or("Capture client");
    let platform = v["platform"].as_str().unwrap_or("unknown");
    let owner = match v["owner_wallet"]
        .as_str()
        .map(causewaybay_shared::identity::normalize_address)
        .transpose()
    {
        Ok(v) => v,
        Err(_) => return,
    };
    if register_device(
        &s,
        &id,
        name,
        platform,
        owner.as_deref(),
        v["device_key"].as_str(),
    )
    .is_err()
    {
        return;
    }
    let session = db::id();
    let previous = s
        .capture_sessions
        .lock()
        .unwrap()
        .insert(id.clone(), session.clone());
    if let Some(previous) = previous {
        crate::notes::disconnected(&s, &id, &previous);
    }
    let _session = CaptureSession {
        state: s.clone(),
        id: id.clone(),
        session: session.clone(),
    };
    if v["textedit_notes_v1"] == true {
        s.notes
            .capable
            .lock()
            .unwrap()
            .insert(id.clone(), session.clone());
        if v["mac_actions_v1"] == true {
            s.notes
                .actions
                .lock()
                .unwrap()
                .insert(id.clone(), session.clone());
        }
    }
    let sse = v["callbacks"] == "sse";
    crate::connections::report_capture(&s, "connected");
    if socket
        .send(Message::Text(
            json!({"type":"ready","client_id":id,"session":session,"callbacks":if sse{"sse"}else{"websocket"}}).to_string().into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    let mut sent_name = String::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut last_received = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _=tick.tick()=>{
                if last_received.elapsed()>Duration::from_secs(35) || s.capture_sessions.lock().unwrap().get(&id)!=Some(&session){break;}
                if !sse {
                    let current=s.db.get_setting(&format!("node.{id}")).ok().flatten().unwrap_or_else(||name.to_string());
                    if current!=sent_name {sent_name=current.clone();if socket.send(Message::Text(json!({"type":"node_config","name":current}).to_string().into())).await.is_err(){break;}}
                }
                if let Some(command)=crate::notes::claim(&s,&id,&session) && socket.send(Message::Text(command.to_string().into())).await.is_err(){break;}
                if !sse && let Ok(Some(command))=s.db.claim_capture(&id) && socket.send(Message::Text(serde_json::to_string(&command).unwrap().into())).await.is_err(){break;}
            },
            incoming=socket.recv()=>{
                let Some(Ok(Message::Text(text)))=incoming else{break};let Ok(v)=serde_json::from_str::<Value>(&text)else{continue};
                if s.capture_sessions.lock().unwrap().get(&id)!=Some(&session){break;}
                last_received=tokio::time::Instant::now();
                let _=s.db.heartbeat(&id);
                match v["type"].as_str().unwrap_or(""){
                    "heartbeat"=>{let _=socket.send(Message::Text(json!({"type":"heartbeat"}).to_string().into())).await;},
                    "notes_result"=>crate::notes::complete(&s,&id,&session,v),
                    "capture_result"=>{
                        let request=v["request_id"].as_str().unwrap_or("");
                        let result=(||->Result<()>{
                            if let Some(error)=v["error"].as_str(){return s.db.complete_capture(&id,request,None,Some(&error.chars().take(2000).collect::<String>()));}
                            let image=STANDARD.decode(v["image_base64"].as_str().unwrap_or(""))?;
                            ensure!(image.len()<=25*1024*1024&&crate::capture::image_mime(&image).is_some(),"Capture must be a PNG, JPEG, or WebP under 25 MB");
                            s.db.complete_capture(&id,request,Some(&image),None)
                        })();
                        let error=result.err().map(|e|e.to_string()).or_else(||v["error"].as_str().map(str::to_string));
                        if let Some(error)=&error { let _=s.db.complete_capture(&id,request,None,Some(error)); }
                        let _=socket.send(Message::Text(json!({"type":"capture_ack","request_id":request,"ok":error.is_none(),"error":error}).to_string().into())).await;
                    },_=>{}
                }
            }
        }
    }
}

pub async fn request_capture(
    s: &Arc<AppState>,
    memo: &str,
    client: Option<&str>,
    mode: &str,
    display: u32,
    requester: Option<&str>,
) -> Result<Vec<u8>> {
    let clients = s.db.clients()?;
    let selected=clients.iter().find(|c|c.online&&client.is_none_or(|id|id==c.id)).ok_or_else(||anyhow::anyhow!("No capture client is connected. Start the Rust CLI on the computer you want to capture."))?;
    ensure!(mode == "screen" || mode == "region", "Invalid capture mode");
    ensure!((1..=16).contains(&display), "Invalid display number");
    ensure!(
        can_capture(selected, requester),
        "This capture client belongs to another wallet"
    );
    let request =
        s.db.queue_capture_for(&selected.id, memo, mode, display, requester)?;
    s.db.event(memo, "stage", &format!("Capturing on {}", selected.name))?;
    tokio::time::timeout(Duration::from_secs(100),async{
        loop{
            let(status,error)=s.db.capture_status(&request)?;
            if let Some(error)=error{anyhow::bail!("{error}");}
            if status=="ready"{return s.db.image(memo)?.ok_or_else(||anyhow::anyhow!("Client returned no screenshot"));}
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    }).await.map_err(|_|anyhow::anyhow!("The capture client did not respond within 100 seconds. Check its connection and screen recording permission."))?
}

struct CaptureSession {
    state: Arc<AppState>,
    id: String,
    session: String,
}
impl Drop for CaptureSession {
    fn drop(&mut self) {
        let mut sessions = self.state.capture_sessions.lock().unwrap();
        if sessions.get(&self.id) == Some(&self.session) {
            sessions.remove(&self.id);
            crate::notes::disconnected(&self.state, &self.id, &self.session);
            let _ = self.state.db.release_captures(&self.id);
            crate::connections::report_capture(&self.state, "disconnected");
        }
    }
}
#[derive(serde::Deserialize)]
pub struct ClientEvents {
    client_id: String,
    #[serde(default)]
    after: i64,
}
pub async fn client_events(
    State(s): State<Arc<AppState>>,
    Query(q): Query<ClientEvents>,
    headers: HeaderMap,
) -> Api<impl IntoResponse> {
    let session = headers
        .get("x-capture-session")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if session.is_empty() || s.capture_sessions.lock().unwrap().get(&q.client_id) != Some(&session)
    {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Connect the client WebSocket before opening callbacks".into(),
        ));
    }
    let stream = async_stream::stream! {
        yield Ok::<_,std::convert::Infallible>(Event::default().retry(Duration::from_secs(2)).comment("capture callbacks ready"));
        // Replay an unacknowledged command after an SSE reconnect; the CLI deduplicates request IDs.
        let pending=s.db.0.lock().unwrap().query_row("SELECT c.id,c.mode,c.display FROM capture_requests c JOIN memos m ON m.id=c.memo_id WHERE c.client_id=?1 AND c.status='capturing' AND m.status='running' ORDER BY c.created_at LIMIT 1",[&q.client_id],|r|Ok(CaptureCommand{kind:"capture".into(),request_id:r.get(0)?,mode:r.get(1)?,display:r.get(2)?})).optional().ok().flatten();
        if let Some(command)=pending { yield Ok(Event::default().id(&command.request_id).json_data(&command).unwrap()); }
        let mut sent_name=String::new();
        let mut after=q.after;
        loop {
            if s.capture_sessions.lock().unwrap().get(&q.client_id)!=Some(&session) { break; }
            if let Ok(Some(name))=s.db.get_setting(&format!("node.{}",q.client_id)) && name!=sent_name {
                sent_name=name.clone();
                yield Ok(Event::default().json_data(json!({"type":"node_config","name":name})).unwrap());
            }
            if let Ok(events)=crate::notifications::changes(&s.db,Some(&q.client_id),after){for event in events{after=event["id"].as_i64().unwrap();yield Ok(Event::default().json_data(event).unwrap());}}
            if let Ok(Some(command))=s.db.claim_capture(&q.client_id) { yield Ok(Event::default().id(&command.request_id).json_data(&command).unwrap()); }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    fn old_socket_disconnect_does_not_release_new_session() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open(dir.path().into(), false, None)?;
        s.db.client_hello("mac", "Mac", "macOS")?;
        s.capture_sessions
            .lock()
            .unwrap()
            .insert("mac".into(), "new".into());
        drop(CaptureSession {
            state: s.clone(),
            id: "mac".into(),
            session: "old".into(),
        });
        assert!(s.db.clients()?[0].online);
        assert_eq!(
            s.capture_sessions
                .lock()
                .unwrap()
                .get("mac")
                .map(String::as_str),
            Some("new")
        );
        drop(CaptureSession {
            state: s.clone(),
            id: "mac".into(),
            session: "new".into(),
        });
        assert!(!s.db.clients()?[0].online);
        assert!(s.capture_sessions.lock().unwrap().is_empty());
        Ok(())
    }

    /// A repeated "/api" prefix in a WebSocket request must not route an
    /// approved member past the admin gate (regression for the dispatcher bypass).
    #[tokio::test]
    async fn websocket_dispatch_rejects_doubled_api_prefix_for_members() -> Result<()> {
        use causewaybay_shared::identity::Identity;
        let dir = tempfile::tempdir()?;
        let mut s =
            AppState::open_with_credentials(dir.path().into(), false, None, Default::default())?;
        let root = Identity::from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .map_err(anyhow::Error::msg)?
        .address();
        Arc::get_mut(&mut s).unwrap().auth = Arc::new(crate::auth::Auth::new(Some(root.clone()))?);
        let member =
            Identity::from_mnemonic("test test test test test test test test test test test junk")
                .map_err(anyhow::Error::msg)?
                .address();
        crate::users::register(&s, &member, None).map_err(|e| anyhow::anyhow!(e.1))?;
        s.db.0
            .lock()
            .unwrap()
            .execute("UPDATE users SET role='user' WHERE address=?1", [&member])?;
        s.db.client_hello("mac", "Mac", "macOS")?;
        let session = |address: &str| -> Result<HeaderMap> {
            let response = crate::auth::create_session(&s, &HeaderMap::new(), Some(address))
                .map_err(|e| anyhow::anyhow!(e.1))?;
            let cookie = response.headers()["set-cookie"]
                .to_str()?
                .split(';')
                .next()
                .unwrap()
                .to_string();
            let mut headers = HeaderMap::new();
            headers.insert("cookie", cookie.parse()?);
            headers.insert("host", "localhost:3000".parse()?);
            Ok(headers)
        };
        let member_headers = session(&member)?;
        let admin_headers = session(&root)?;
        let status =
            |r: Api<(u16, Value)>| r.map(|(status, _)| status).unwrap_or_else(|e| e.0.as_u16());
        for (method, path, body) in [
            ("GET", "/api/api/admin/users", Value::Null),
            ("PATCH", "/api/api/clients/mac", json!({"name":"renamed"})),
            ("POST", "/api/api/ai/check", json!({})),
            ("GET", "/api/admin/users", Value::Null),
            ("PATCH", "/api/clients/mac", json!({"name":"renamed"})),
        ] {
            let code =
                status(dispatch(s.clone(), method, path, body, member_headers.clone()).await);
            assert!(
                matches!(code, 400 | 403 | 404),
                "{method} {path} returned {code} for an approved member"
            );
        }
        assert_eq!(
            s.db.clients()?[0].name,
            "Mac",
            "member must not rename nodes"
        );
        // Non-admin workspace paths still work for members and admins keep setup access.
        assert_eq!(
            status(dispatch(s.clone(), "GET", "/api/rooms", Value::Null, member_headers).await),
            200
        );
        assert_eq!(
            status(
                dispatch(
                    s.clone(),
                    "GET",
                    "/api/admin/users",
                    Value::Null,
                    admin_headers.clone()
                )
                .await
            ),
            200
        );
        assert_eq!(
            status(
                dispatch(
                    s.clone(),
                    "GET",
                    "/api/api/admin/users",
                    Value::Null,
                    admin_headers
                )
                .await
            ),
            400
        );
        Ok(())
    }
}

#[cfg(test)]
mod stamp_tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// The browser's word about erasure is ignored: each connection carries the
    /// backend's own stamp, a Delete all elsewhere stales it, and a page that
    /// reconnects after being told starts from the current one.
    #[tokio::test]
    async fn requests_carry_the_connections_stamp_not_the_browsers() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(
            dir.path().into(),
            true,
            None,
            crate::credentials::Credentials::default(),
        )?;
        let headers = HeaderMap::new();
        let page_a = AtomicI64::new(connection_epoch(&s, &headers).unwrap());
        assert_eq!(page_a.load(Ordering::SeqCst), 0);
        // A stale or forged browser stamp changes nothing.
        let (status, memo) = dispatch_stamped(
            s.clone(),
            "POST",
            "/api/think",
            json!({"prompt":"hello","input_mode":"text","data_epoch":99_999_999_999_i64,"dataEpoch":5}),
            headers.clone(),
            &page_a,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.1))?;
        assert_eq!(status, 200, "{memo}");

        // Another page erases everything: its own stamp moves forward at once.
        let page_b = AtomicI64::new(connection_epoch(&s, &headers).unwrap());
        let (status, deleted) = dispatch_stamped(
            s.clone(),
            "DELETE",
            "/api/user/data",
            Value::Null,
            headers.clone(),
            &page_b,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.1))?;
        assert_eq!(status, 200, "{deleted}");
        let cutoff = deleted["messagesyncid"].as_i64().unwrap();
        assert!(cutoff > 0);
        assert_eq!(page_b.load(Ordering::SeqCst), cutoff);
        let (status, _) = dispatch_stamped(
            s.clone(),
            "POST",
            "/api/think",
            json!({"prompt":"after","input_mode":"text"}),
            headers.clone(),
            &page_b,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.1))?;
        assert_eq!(status, 200, "the deleting page continues");

        // The page that has not heard yet is refused, whatever its browser claims.
        let (status, err) = dispatch_stamped(
            s.clone(),
            "POST",
            "/api/think",
            json!({"prompt":"stale","input_mode":"text","data_epoch":cutoff}),
            headers.clone(),
            &page_a,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.1))?;
        assert_eq!(status, 400);
        assert!(err["error"].as_str().unwrap().contains("erased"), "{err}");
        // Batch captures carry the stamp inside their capture request; strict
        // envelopes and unrelated writes are left exactly as sent.
        assert_eq!(
            stamped_body(
                "POST",
                "/api/capture/batch",
                json!({"all":true,"client_ids":[],"request":{"prompt":"x","dataEpoch":1}}),
                cutoff
            ),
            json!({"all":true,"client_ids":[],"request":{"prompt":"x","data_epoch":cutoff}})
        );
        assert_eq!(
            stamped_body("POST", "/api/think", Value::Null, 7),
            json!({"data_epoch":7})
        );
        assert_eq!(
            stamped_body(
                "POST",
                "/api/rooms/abc/capture",
                json!({"client_id":"mac","interval_seconds":5,"request":{"prompt":"x"}}),
                7
            ),
            json!({"client_id":"mac","interval_seconds":5,"request":{"prompt":"x","data_epoch":7}})
        );
        assert_eq!(
            stamped_body(
                "POST",
                "/api/rooms/abc/capture/now",
                json!({"prompt":"x"}),
                7
            ),
            json!({"prompt":"x","data_epoch":7})
        );
        assert_eq!(
            stamped_body(
                "POST",
                "/api/uploads",
                json!({"name":"a","dataEpoch":99}),
                7
            ),
            json!({"name":"a","data_epoch":7})
        );
        assert_eq!(
            stamped_body("POST", "/api/rooms", json!({"title":"Still mine"}), 7),
            json!({"title":"Still mine"})
        );
        assert_eq!(
            stamped_body("PATCH", "/api/settings", json!({"display":2}), 7),
            json!({"display":2})
        );
        // Reads and other writes are left untouched: no stamp, no refusal.
        let (status, body) = dispatch_stamped(
            s.clone(),
            "GET",
            "/api/settings",
            Value::Null,
            headers.clone(),
            &page_a,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.1))?;
        assert_eq!(status, 200, "{body}");
        assert!(stamped_path("POST", "/api/rooms/abc/capture/now"));
        assert!(!stamped_path("GET", "/api/think") && !stamped_path("POST", "/api/rooms"));

        // Reconnecting (what the page does when told) takes the current stamp.
        let page_a2 = AtomicI64::new(connection_epoch(&s, &headers).unwrap());
        assert_eq!(page_a2.load(Ordering::SeqCst), cutoff);
        let (status, _) = dispatch_stamped(
            s.clone(),
            "POST",
            "/api/think",
            json!({"prompt":"fresh","input_mode":"text"}),
            headers,
            &page_a2,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.1))?;
        assert_eq!(status, 200);
        Ok(())
    }
}

#[cfg(test)]
mod enrollment_regressions {
    use super::*;
    #[tokio::test]
    async fn device_id_alone_cannot_replace_an_enrolled_device() {
        let dir = tempfile::tempdir().unwrap();
        let s = AppState::open_configured(
            dir.path().join("data"),
            dir.path().join("config"),
            false,
            None,
        )
        .unwrap();
        let key = "a".repeat(64);
        register_device(&s, "mac", "My Mac", "macOS", Some("alice"), Some(&key)).unwrap();
        assert!(
            register_device(
                &s,
                "mac",
                "Impostor",
                "macOS",
                Some("bob"),
                Some(&"b".repeat(64))
            )
            .is_err()
        );
        assert!(register_device(&s, "mac", "Impostor", "macOS", Some("bob"), None).is_err());
        let clients = s.db.clients().unwrap();
        assert_eq!(clients[0].name, "My Mac");
        assert_eq!(clients[0].owner_wallet.as_deref(), Some("alice"));
        register_device(&s, "mac", "My Mac", "macOS", Some("alice"), Some(&key)).unwrap();
    }
    #[tokio::test]
    async fn capture_requires_a_secret_and_local_browser_rejects_rebinding() {
        let dir = tempfile::tempdir().unwrap();
        let s = AppState::open_configured(
            dir.path().join("data"),
            dir.path().join("config"),
            false,
            None,
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:3000".parse().unwrap());
        assert!(crate::auth::authorize(&s, &headers, "GET", "/api/client/ws").is_err());
        headers.insert(
            "authorization",
            format!("Bearer {}", s.capture_token.as_ref().unwrap())
                .parse()
                .unwrap(),
        );
        assert!(
            crate::auth::authorize(&s, &headers, "GET", "/api/client/ws")
                .unwrap()
                .node
        );
        headers.remove("authorization");
        assert!(crate::auth::authorize(&s, &headers, "GET", "/api/settings").is_ok());
        headers.insert("host", "attacker.invalid:3000".parse().unwrap());
        headers.insert("origin", "http://attacker.invalid:3000".parse().unwrap());
        assert!(crate::auth::authorize(&s, &headers, "GET", "/api/settings").is_err());
    }
}
