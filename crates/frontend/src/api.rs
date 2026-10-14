//! WebSocket JSON requests/presence; SSE callbacks with browser-managed cursor replay.
use causewaybay_shared::{ChatMessage, StreamEvent};
use futures_channel::{mpsc, oneshot};
use futures_util::{
    SinkExt, StreamExt,
    future::{Either, select},
};
use gloo_events::EventListener;
use gloo_net::{
    http::Request,
    websocket::{Message, futures::WebSocket},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use yew::Callback;

struct Pending {
    reply: oneshot::Sender<Result<Value, String>>,
}
struct Subscription {
    source: web_sys::EventSource,
    _listener: EventListener,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.source.close();
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ConnectionState {
    #[default]
    Connecting,
    Connected,
    Reconnecting,
}
impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Reconnecting => "Reconnecting",
        }
    }
}
/// Transport errors describe the connection at the time a request failed.
/// They no longer describe its status after a new connection is ready.
pub fn is_connection_error(error: &str) -> bool {
    matches!(
        error,
        "Connection interrupted. Reconnecting to your backend…"
            | "Connection interrupted. Retrying in 2 seconds…"
            | "Could not connect to the backend"
            | "Connection closed"
            | "The server is unreachable. Check your VPN connection."
    )
}
#[derive(Default)]
struct Connection {
    sender: Option<mpsc::UnboundedSender<Message>>,
    pending: HashMap<u64, Pending>,
    subscriptions: HashMap<String, Subscription>,
    next: u64,
    state: ConnectionState,
    listener: Option<Callback<ConnectionState>>,
    retry: Option<gloo_timers::callback::Timeout>,
    client_id: String,
}
thread_local! {static CONNECTION:RefCell<Connection>=RefCell::new(Connection::default());}

pub fn connection_state() -> ConnectionState {
    CONNECTION.with(|c| c.borrow().state)
}

pub fn client_id() -> String {
    CONNECTION.with(|c| {
        let mut c = c.borrow_mut();
        if c.client_id.is_empty() {
            c.client_id = format!(
                "web-{:x}-{:x}",
                js_sys::Date::now() as u64,
                (js_sys::Math::random() * u32::MAX as f64) as u32
            );
        }
        c.client_id.clone()
    })
}
fn hello() -> Value {
    let navigator = web_sys::window().unwrap().navigator();
    let ua = navigator.user_agent().unwrap_or_default();
    let tablet = ua.contains("iPad")
        || (ua.contains("Macintosh")
            && js_sys::Reflect::get(navigator.as_ref(), &"maxTouchPoints".into())
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.)
                > 1.);
    let platform = if tablet {
        "iPad"
    } else if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Mac") {
        "macOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "Browser"
    };
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome") || ua.contains("CriOS") {
        "Chrome"
    } else if ua.contains("Firefox") {
        "Firefox"
    } else if ua.contains("Safari") {
        "Safari"
    } else {
        "Web client"
    };
    json!({"type":"hello","id":client_id(),"name":format!("{browser} · {platform}"),"platform":platform})
}
fn set_state(state: ConnectionState) {
    let listener = CONNECTION.with(|c| {
        let mut c = c.borrow_mut();
        if c.state == state {
            return None;
        }
        c.state = state;
        c.listener.clone()
    });
    if let Some(listener) = listener {
        listener.emit(state);
    }
}
/// Drop the current WebSocket so the next one starts from the backend's
/// current state (its erasure stamp above all); the usual retry reconnects.
pub fn reconnect() {
    let sender = CONNECTION.with(|c| c.borrow_mut().sender.take());
    drop(sender);
}
pub fn watch_connection(listener: Callback<ConnectionState>) {
    let state = CONNECTION.with(|c| {
        let mut c = c.borrow_mut();
        c.listener = Some(listener.clone());
        c.state
    });
    listener.emit(state);
    let _ = connect();
}
fn retry_later() {
    set_state(ConnectionState::Reconnecting);
    CONNECTION.with(|c| {
        let mut c = c.borrow_mut();
        if c.retry.is_none() {
            c.retry = Some(gloo_timers::callback::Timeout::new(2000, || {
                CONNECTION.with(|c| c.borrow_mut().retry = None);
                let _ = connect();
            }));
        }
    });
}
fn connect() -> Result<(), String> {
    if CONNECTION.with(|c| c.borrow().sender.is_some()) {
        return Ok(());
    }
    if CONNECTION.with(|c| c.borrow().retry.is_some()) {
        return Err("Connection interrupted. Retrying in 2 seconds…".into());
    }
    let location = web_sys::window().unwrap().location();
    let host = location
        .host()
        .map_err(|_| "Could not read server address")?;
    let scheme = if location.protocol().unwrap_or_default() == "https:" {
        "wss"
    } else {
        "ws"
    };
    let socket = match WebSocket::open(&format!("{scheme}://{host}/api/ws")) {
        Ok(socket) => socket,
        Err(_) => {
            retry_later();
            return Err("Could not connect to the backend".into());
        }
    };
    let (mut sink, mut source) = socket.split();
    let (sender, mut receiver) = mpsc::unbounded::<Message>();
    let greeting = hello();
    CONNECTION.with(|c| {
        let mut c = c.borrow_mut();
        c.sender = Some(sender.clone());
        let _ = sender.unbounded_send(Message::Text(greeting.to_string()));
    });
    spawn_local(async move {
        let connected_at = js_sys::Date::now();
        let mut last_received = connected_at;
        let mut ready = false;
        // Keep receiving close/error frames while writes wait for a socket to open.
        let writer = async move {
            while let Some(message) = receiver.next().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
        };
        futures_util::pin_mut!(writer);
        loop {
            let now = js_sys::Date::now();
            if now - last_received >= 30_000. || (!ready && now - connected_at >= 10_000.) {
                break;
            }
            let incoming = select(
                source.next(),
                gloo_timers::future::TimeoutFuture::new(10_000),
            );
            futures_util::pin_mut!(incoming);
            match select(writer.as_mut(), incoming).await {
                Either::Left(_) => break,
                Either::Right((Either::Left((Some(Ok(Message::Text(text))), _)), _)) => {
                    last_received = js_sys::Date::now();
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        if v["type"] == "ready" {
                            ready = true;
                            set_state(ConnectionState::Connected);
                        } else if v["type"] == "response" {
                            let pending = CONNECTION.with(|c| {
                                c.borrow_mut()
                                    .pending
                                    .remove(&v["id"].as_u64().unwrap_or(0))
                            });
                            if let Some(p) = pending {
                                let status = v["status"].as_u64().unwrap_or(500);
                                let result = if status == 401 {
                                    Err("AUTH_REQUIRED".into())
                                } else if status >= 400 {
                                    Err(v["data"]["error"]
                                        .as_str()
                                        .unwrap_or("Request failed")
                                        .to_string())
                                } else {
                                    Ok(v["data"].clone())
                                };
                                let _ = p.reply.send(result);
                            }
                        }
                    }
                }
                Either::Right((Either::Left((Some(Ok(_)), _)), _)) => {}
                Either::Right((Either::Right(_), _)) => {
                    let _ = sender
                        .unbounded_send(Message::Text(json!({"type":"heartbeat"}).to_string()));
                }
                _ => break,
            }
        }
        drop(source);
        CONNECTION.with(|c| {
            let mut c = c.borrow_mut();
            c.sender = None;
            for (_, p) in c.pending.drain() {
                let _ = p.reply.send(Err(
                    "Connection interrupted. Reconnecting to your backend…".into()
                ));
            }
        });
        retry_later();
    });
    Ok(())
}

pub async fn api<T: DeserializeOwned>(
    method: &str,
    url: &str,
    body: Option<Value>,
) -> Result<T, String> {
    // Session bootstrap uses HTTP to distinguish an unauthorized WebSocket handshake.
    if url.ends_with("/image")
        || url.starts_with("/api/auth/")
        || url == "/api/session"
        || url == "/api/settings" && method == "GET"
        // Wallet choices are tiny and must not be lost while the socket reconnects.
        || url.starts_with("/api/wallet")
    {
        return http(method, url, body).await;
    }
    connect()?;
    let (tx, rx) = oneshot::channel();
    let id=CONNECTION.with(|c|{let mut c=c.borrow_mut();c.next+=1;let id=c.next;c.pending.insert(id,Pending{reply:tx});let _=c.sender.as_ref().unwrap().unbounded_send(Message::Text(json!({"type":"request","id":id,"method":method,"path":url,"body":body.unwrap_or(Value::Null)}).to_string()));id});
    let value = match select(rx, gloo_timers::future::TimeoutFuture::new(120_000)).await {
        Either::Left((result, _)) => result.map_err(|_| "Connection closed".to_string())??,
        Either::Right(_) => {
            CONNECTION.with(|c| c.borrow_mut().pending.remove(&id));
            return Err("Request timed out. Your saved work is still available.".into());
        }
    };
    serde_json::from_value(value).map_err(|_| "Unexpected response from the backend".into())
}
async fn http<T: DeserializeOwned>(
    method: &str,
    url: &str,
    body: Option<Value>,
) -> Result<T, String> {
    let request = match method {
        "POST" => Request::post(url),
        "PATCH" => Request::patch(url),
        "DELETE" => Request::delete(url),
        _ => Request::get(url),
    };
    let response = if let Some(v) = body {
        request.json(&v).map_err(|e| e.to_string())?.send().await
    } else {
        request.send().await
    }
    .map_err(|_| "The server is unreachable. Check your VPN connection.".to_string())?;
    if response.status() == 401 {
        return Err("AUTH_REQUIRED".into());
    }
    if !response.ok() {
        let value: Value = response.json().await.unwrap_or_default();
        return Err(value["error"]
            .as_str()
            .unwrap_or("The server could not complete this request")
            .into());
    }
    response
        .json()
        .await
        .map_err(|_| "Unexpected response from the backend".into())
}
pub fn subscribe(id: &str, callback: Callback<StreamEvent>) -> Result<(), String> {
    let source = web_sys::EventSource::new(&format!("/api/memos/{}/events", encode(id)))
        .map_err(|_| "Could not open the backend event stream".to_string())?;
    let listener = EventListener::new(&source, "message", move |event| {
        if let Some(message) = event.dyn_ref::<web_sys::MessageEvent>()
            && let Some(data) = message.data().as_string()
            && let Ok(event) = serde_json::from_str::<StreamEvent>(&data)
        {
            callback.emit(event);
        }
    });
    CONNECTION.with(|c| {
        c.borrow_mut().subscriptions.insert(
            id.into(),
            Subscription {
                source,
                _listener: listener,
            },
        );
    });
    Ok(())
}
pub fn unsubscribe(id: &str) {
    CONNECTION.with(|c| {
        c.borrow_mut().subscriptions.remove(id);
    });
}
pub fn encode(s: &str) -> String {
    js_sys::encode_uri_component(s)
        .as_string()
        .unwrap_or_default()
}

pub fn subscribe_room(
    id: &str,
    after: i64,
    callback: Callback<ChatMessage>,
    cleared: Callback<Value>,
) -> Result<(), String> {
    let source =
        web_sys::EventSource::new(&format!("/api/rooms/{}/events?after={after}", encode(id)))
            .map_err(|_| "Could not open room updates".to_string())?;
    let listener = EventListener::new(&source, "message", move |e| {
        if let Some(data) = e
            .dyn_ref::<web_sys::MessageEvent>()
            .and_then(|e| e.data().as_string())
            && let Ok(value) = serde_json::from_str::<Value>(&data)
        {
            if value["kind"] == "room.cleared" {
                cleared.emit(value);
            } else if let Ok(message) = serde_json::from_value(value) {
                callback.emit(message);
            }
        }
    });
    CONNECTION.with(|c| {
        c.borrow_mut().subscriptions.insert(
            format!("room:{id}"),
            Subscription {
                source,
                _listener: listener,
            },
        );
    });
    Ok(())
}
pub fn unsubscribe_room(id: &str) {
    unsubscribe(&format!("room:{id}"));
}
// Share a single notification connection within this tab. Each listener receives
// every event directly, including bursts; components retain independent handles.
#[derive(Default)]
struct NotificationHub {
    next: u64,
    listeners: HashMap<u64, Callback<Value>>,
    subscription: Option<Subscription>,
    ready_listener: Option<EventListener>,
}
thread_local! { static NOTIFICATIONS: RefCell<NotificationHub> = RefCell::new(NotificationHub::default()); }
pub struct Notifications(u64);
impl Drop for Notifications {
    fn drop(&mut self) {
        NOTIFICATIONS.with(|hub| {
            let mut hub = hub.borrow_mut();
            hub.listeners.remove(&self.0);
            if hub.listeners.is_empty() {
                hub.subscription = None;
                hub.ready_listener = None;
            }
        });
    }
}
/// Do not carry authenticated streams or listeners across wallet/session changes.
pub fn reset_notifications() {
    NOTIFICATIONS.with(|hub| {
        let mut hub = hub.borrow_mut();
        hub.subscription = None;
        hub.ready_listener = None;
        hub.listeners.clear();
    });
}
pub fn notifications(callback: Callback<Value>) -> Result<Notifications, String> {
    NOTIFICATIONS.with(|hub| {
        let mut hub = hub.borrow_mut();
        if hub.subscription.is_none() {
            let source = web_sys::EventSource::new("/api/notifications")
                .map_err(|_| "Could not open notifications".to_string())?;
            // Reconcile current state on initial login and reconnect, even if
            // historical room notifications were pruned by an erasure.
            hub.ready_listener = Some(EventListener::new(&source, "open", move |_| {
                let listeners = NOTIFICATIONS
                    .with(|hub| hub.borrow().listeners.values().cloned().collect::<Vec<_>>());
                for listener in listeners {
                    listener.emit(json!({"kind":"notifications.ready"}));
                }
            }));
            let listener = EventListener::new(&source, "message", move |e| {
                if let Some(data) = e
                    .dyn_ref::<web_sys::MessageEvent>()
                    .and_then(|e| e.data().as_string())
                    && let Ok(value) = serde_json::from_str::<Value>(&data)
                {
                    let listeners = NOTIFICATIONS
                        .with(|hub| hub.borrow().listeners.values().cloned().collect::<Vec<_>>());
                    for listener in listeners {
                        listener.emit(value.clone());
                    }
                }
            });
            hub.subscription = Some(Subscription {
                source,
                _listener: listener,
            });
        }
        hub.next += 1;
        let id = hub.next;
        hub.listeners.insert(id, callback);
        Ok(Notifications(id))
    })
}
