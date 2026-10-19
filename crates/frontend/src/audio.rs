//! Audio Capture: a chat session that listens. The capture Mac hears, the
//! backend writes what it heard into the session's chat room, and the cloud
//! AI is asked only when someone presses Solve or Note, from any device. The
//! conversation is the room itself, read through the normal room sync, so
//! every computer signed in to the account shows the same session.
use crate::{
    api::{self, api},
    i18n::{Language, t, tf},
    icons::icon,
};
use causewaybay_shared::{AudioStatus, CaptureClient, ChatMessage, MessageSync};
use gloo_events::EventListener;
use gloo_timers::callback::Interval;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use web_sys::HtmlSelectElement;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub clients: Vec<CaptureClient>,
    /// The workspace's active capture device; the picker starts from it.
    #[prop_or_default]
    pub active_client: String,
    pub demo: bool,
    pub navigate: Callback<String>,
}

pub struct Audio {
    status: AudioStatus,
    loaded: bool,
    polling: bool,
    syncing: bool,
    busy: bool,
    revision: u64,
    error: String,
    client: String,
    /// The room the messages below belong to, and its sync cursor.
    room: String,
    cursor: i64,
    messages: Vec<ChatMessage>,
    _poll: Interval,
    stream: Option<web_sys::EventSource>,
    _stream_listeners: Vec<EventListener>,
    streaming: bool,
}

pub enum Msg {
    Poll,
    Stream(Box<AudioStatus>),
    StreamLost,
    Loaded(u64, Box<Result<AudioStatus, String>>),
    Synced(String, Box<Result<MessageSync, String>>),
    Client(String),
    Start,
    Stop,
    Solve,
    Note,
    NewChat,
    UseRoom(String),
    Done(Box<Result<AudioStatus, String>>),
    OpenChat,
    Opened(Result<Value, String>),
}

/// Memo kind of the notes the backend writes from speech.
const NOTES: &str = "audio";

fn listener(client: &CaptureClient) -> bool {
    client.online && client.can_capture && client.audio
}
fn clock(iso: &str) -> String {
    let date = js_sys::Date::new(&iso.into());
    if date.get_time().is_nan() {
        return String::new();
    }
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}
fn minutes(seconds: f64) -> String {
    let total = seconds.round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}
/// Fold a sync batch into the conversation: newer versions replace older
/// ones, a cleared room drops what came before, order follows the room.
fn merge(messages: &mut Vec<ChatMessage>, batch: Vec<ChatMessage>, cleared_through: i64) {
    if cleared_through > 0 {
        messages.retain(|m| m.messagesyncid > cleared_through);
    }
    for m in batch {
        match messages.iter_mut().find(|o| o.messageid == m.messageid) {
            Some(o) if o.messagesyncid <= m.messagesyncid => *o = m,
            Some(_) => {}
            None => messages.push(m),
        }
    }
    messages.sort_by_key(|m| m.sequence);
}

impl Component for Audio {
    type Message = Msg;
    type Properties = Props;

    fn create(ctx: &Context<Self>) -> Self {
        let link = ctx.link().clone();
        link.send_message(Msg::Poll);
        let stream = web_sys::EventSource::new("/api/audio/events").ok();
        let mut listeners = vec![];
        if let Some(source) = &stream {
            let link = ctx.link().clone();
            listeners.push(EventListener::new(source, "message", move |event| {
                if let Some(data) = event
                    .dyn_ref::<web_sys::MessageEvent>()
                    .and_then(|e| e.data().as_string())
                    && let Ok(status) = serde_json::from_str(&data)
                {
                    link.send_message(Msg::Stream(Box::new(status)));
                }
            }));
            let link = ctx.link().clone();
            listeners.push(EventListener::new(source, "error", move |_| {
                link.send_message(Msg::StreamLost)
            }));
        }
        Self {
            status: AudioStatus::default(),
            loaded: false,
            polling: false,
            syncing: false,
            busy: false,
            revision: 0,
            error: String::new(),
            client: ctx.props().active_client.clone(),
            room: String::new(),
            cursor: 0,
            messages: vec![],
            _poll: Interval::new(1000, move || link.send_message(Msg::Poll)),
            stream,
            _stream_listeners: listeners,
            streaming: false,
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Stream(status) => {
                self.streaming = true;
                self.loaded = true;
                if !self.busy {
                    self.apply(ctx, *status);
                } else if status.room_id == self.status.room_id {
                    // Capture feedback remains live while Stop drains the last
                    // words or Solve waits for its request to be accepted.
                    self.status.listening = status.listening;
                    self.status.input_level = status.input_level;
                    self.status.partial_text = status.partial_text;
                    self.status.stage = status.stage;
                }
                true
            }
            Msg::StreamLost => {
                self.streaming = false;
                self.status.listening = false;
                self.status.input_level = 0.0;
                true
            }
            Msg::Poll => {
                if !self.streaming && !self.polling && !self.busy {
                    self.polling = true;
                    let revision = self.revision;
                    ctx.link().send_future(async move {
                        Msg::Loaded(revision, Box::new(api("GET", "/api/audio", None).await))
                    });
                }
                self.sync(ctx);
                false
            }
            Msg::Loaded(revision, result) => {
                self.polling = false;
                if self.streaming || revision != self.revision || self.busy {
                    return false;
                }
                match *result {
                    Ok(status) => {
                        self.loaded = true;
                        self.apply(ctx, status);
                    }
                    Err(e) if !api::is_connection_error(&e) => self.error = e,
                    Err(_) => {}
                }
                true
            }
            Msg::Synced(room, result) => {
                self.syncing = false;
                if room != self.room {
                    return false;
                }
                match *result {
                    Ok(batch) => {
                        let more = batch.has_more;
                        self.cursor = self.cursor.max(batch.next);
                        merge(&mut self.messages, batch.messages, batch.cleared_through);
                        if more {
                            self.sync(ctx);
                        }
                    }
                    Err(e) if !api::is_connection_error(&e) => self.error = e,
                    Err(_) => {}
                }
                true
            }
            Msg::Client(id) => {
                self.client = id;
                true
            }
            Msg::Start => {
                let client = self.pick(ctx);
                self.call(ctx, "POST", "/api/audio", Some(json!({"client_id":client})))
            }
            Msg::Stop => self.call(ctx, "DELETE", "/api/audio", None),
            Msg::Solve => self.call(ctx, "POST", "/api/audio/solve", None),
            Msg::Note => self.call(ctx, "POST", "/api/audio/note", None),
            Msg::NewChat => self.call(ctx, "POST", "/api/audio/room", Some(json!({"fresh":true}))),
            Msg::UseRoom(room) => {
                if room == self.status.room_id {
                    return false;
                }
                self.call(
                    ctx,
                    "POST",
                    "/api/audio/room",
                    Some(json!({"room_id":room})),
                )
            }
            Msg::Done(result) => {
                self.busy = false;
                match *result {
                    Ok(status) => self.apply(ctx, status),
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::OpenChat => {
                let room = self.status.room_id.clone();
                if room.is_empty() {
                    return false;
                }
                ctx.link().send_future(async move {
                    Msg::Opened(
                        api("POST", "/api/rooms/thought", Some(json!({"room_id":room}))).await,
                    )
                });
                false
            }
            Msg::Opened(result) => {
                match result {
                    Ok(_) => ctx.props().navigate.emit("chat".into()),
                    Err(e) => self.error = e,
                }
                true
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let s = &self.status;
        let listeners: Vec<&CaptureClient> =
            ctx.props().clients.iter().filter(|c| listener(c)).collect();
        let can_start = !self.busy && (!listeners.is_empty() || ctx.props().demo);
        let heard = s.segments.iter().any(|g| !g.text.trim().is_empty());
        let fresh = s
            .segments
            .iter()
            .filter(|g| g.seq > s.solved_through)
            .count();
        let answering = self
            .messages
            .iter()
            .any(|m| !m.deleted && m.role != "user" && m.status == "running");
        let shown: Vec<&ChatMessage> = self.messages.iter().filter(|m| !m.deleted).collect();
        let pick_room = ctx.link().callback(|e: Event| {
            Msg::UseRoom(e.target_unchecked_into::<HtmlSelectElement>().value())
        });
        html! {<section class="audio-page">
            <header class="feature-intro audio-intro"><div><span class="eyebrow">{t("LISTEN. NOTE. SOLVE.")}</span><h1>{t("Your Mac takes the notes. ")}<em>{t("You just ask.")}</em></h1><p>{t("Capture Audio listens through your Mac's microphone and writes what it hears into this chat. Press Solve from any device for an answer to the newest part, or Note for a write-up of the whole session.")}</p><span class="feature-trust">{icon("shield")}{t("Speech is turned into text on your backend Mac. The cloud AI reads it only when you press Solve or Note.")}</span></div><img src="/assets/invisible-ai-hero.png" alt="" width="1536" height="1024"/></header>
            if !self.error.is_empty(){<div class="error-banner" role="alert"><span>{t(&self.error)}</span></div>}
            <div class="audio-toolbar">
                <div class="client-picker">{icon("monitor")}
                    if s.active {<span>{tf("Listening on {name}", &[("name", s.client_name.clone())])}</span>}
                    else if listeners.is_empty() && ctx.props().demo {<span>{t("Demo capture Mac")}</span>}
                    else if listeners.is_empty() {<span>{t("No capture Mac can listen")}</span><button class="text-button" onclick={ctx.props().navigate.reform(|_|"devices".to_string())}>{t("Connect a device")}{icon("arrow")}</button>}
                    else {<label><span>{t("Listen with")}</span><select aria-label={t("Capture Mac to listen with")} value={self.pick(ctx)} disabled={self.busy} onchange={ctx.link().callback(|e:Event|Msg::Client(e.target_unchecked_into::<HtmlSelectElement>().value()))}>
                        {for listeners.iter().map(|c| html!{<option value={c.id.clone()} selected={c.id==self.pick(ctx)}>{c.name.clone()}</option>})}
                    </select></label>}
                </div>
                if s.active {<button class="secondary-button" data-testid="audio-stop" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Stop)}>{icon("stop")}{t("Stop listening")}</button>}
                else {<button class="primary-button" data-testid="audio-start" disabled={!can_start} onclick={ctx.link().callback(|_|Msg::Start)}>{icon("audio")}{t("Capture Audio")}</button>}
            </div>
            <div class="audio-session-bar">
                if s.rooms.len() > 1 {
                    <label class="audio-room-picker"><span>{t("Audio chat")}</span><select aria-label={t("Audio chat")} data-testid="audio-room" disabled={self.busy} onchange={pick_room}>
                        {for s.rooms.iter().map(|r| html!{<option value={r.roomid.clone()} selected={r.roomid==s.room_id}>{r.title.clone()}</option>})}
                    </select></label>
                } else {<strong class="audio-room-title">{icon("message")}{if s.room_title.is_empty(){t("Audio Capture")}else{s.room_title.clone()}}</strong>}
                <button class="secondary-button" data-testid="audio-new-chat" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::NewChat)}>{icon("plus")}{t("New chat")}</button>
                <button class="text-button" disabled={s.room_id.is_empty()} onclick={ctx.link().callback(|_|Msg::OpenChat)}>{icon("message")}{t("Open in Ask anything")}</button>
            </div>
            <div class="audio-board" data-testid="audio-board" data-active={s.active.to_string()}>
                <div class="audio-board-head">
                    <h2>{icon("file")}{t("Conversation")}</h2>
                    <span class={classes!("audio-live",s.listening.then_some("is-listening"))} role="status" data-testid="audio-stage"><i/>{if self.loaded{t(&s.stage)}else{t("Loading…")}}</span>
                </div>
                if s.listening {
                    <div class="audio-listening" data-testid="audio-listening" style={format!("--audio-level:{}", s.input_level.clamp(0.0, 1.0))}>
                        <span class="audio-mic-orbit" aria-hidden="true">{icon("audio")}</span>
                        <div class="audio-listening-label"><strong>{t("Listening…")}</strong><span>{s.client_name.clone()}</span></div>
                        <span class="audio-wave" aria-hidden="true">{for (0..7).map(|_| html!{<i/>})}</span>
                    </div>
                }
                if s.active || heard {
                    <p class="audio-meter">
                        <span>{tf("{minutes} heard", &[("minutes", minutes(s.heard_seconds))])}</span>
                        <span>{tf("{count} notes", &[("count", s.segments.len().to_string())])}</span>
                        if s.pending_chunks>0 {<span>{tf("{count} being transcribed", &[("count", s.pending_chunks.to_string())])}</span>}
                        <span>{t(if s.speech_provider=="mlx"{"MLX Whisper"}else{"Apple Speech"})}</span>
                    </p>
                }
                if !s.summary.is_empty() {
                    <details class="audio-summary"><summary>{icon("book")}{t("Condensed earlier notes")}</summary><p>{s.summary.clone()}</p></details>
                }
                if shown.is_empty() {
                    <p class="audio-empty">{if s.active{t("Nothing heard yet. Speak near the Mac's microphone; each pause becomes a note.")}else{t("Press Capture Audio and your Mac starts writing notes from what it hears.")}}</p>
                } else {
                    <ol class="audio-thread" data-testid="audio-notes" aria-live="polite">
                        {for shown.iter().map(|m| self.message(m))}
                    </ol>
                }
                if s.active && !s.partial_text.is_empty() {
                    <div class="audio-draft" data-testid="audio-partial" aria-live="polite">
                        <div class="audio-message-meta"><strong>{icon("audio")}{t("Heard")}</strong><span>{t("Transcribing…")}</span></div>
                        <p>{s.partial_text.clone()}<span class="audio-caret" aria-hidden="true"/></p>
                    </div>
                }
                <div class="audio-actions">
                    <button class="secondary-button" data-testid="audio-note" disabled={self.busy||answering||!heard} onclick={ctx.link().callback(|_|Msg::Note)}>{icon("file")}{t("Note")}</button>
                    <button class="primary-button" data-testid="audio-solve" disabled={self.busy||answering||!heard} onclick={ctx.link().callback(|_|Msg::Solve)}>{icon("spark")}{t("Solve")}if fresh>0 {<small>{tf("{count} new", &[("count", fresh.to_string())])}</small>}</button>
                </div>
                <p class="audio-hint">{icon("leaf")}<span>{t("Solve answers the newest speech, with the earlier speech and answers in this chat as context. Note writes the whole session up as a note. Older speech is condensed, so the AI's context is never exceeded.")}</span></p>
            </div>
        </section>}
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        if let Some(stream) = self.stream.take() {
            stream.close();
        }
    }
}

impl Audio {
    fn call(
        &mut self,
        ctx: &Context<Self>,
        method: &'static str,
        url: &'static str,
        body: Option<Value>,
    ) -> bool {
        if self.busy {
            return false;
        }
        self.busy = true;
        self.revision += 1;
        self.error.clear();
        ctx.link()
            .send_future(async move { Msg::Done(Box::new(api(method, url, body).await)) });
        true
    }
    fn sync(&mut self, ctx: &Context<Self>) {
        if self.syncing || self.room.is_empty() {
            return;
        }
        self.syncing = true;
        let room = self.room.clone();
        let url = format!("/api/rooms/{room}/sync?after={}", self.cursor);
        ctx.link()
            .send_future(async move { Msg::Synced(room, Box::new(api("GET", &url, None).await)) });
    }
    /// The chosen listener, or the first one that can listen.
    fn pick(&self, ctx: &Context<Self>) -> String {
        let clients = &ctx.props().clients;
        if clients.iter().any(|c| c.id == self.client && listener(c)) {
            return self.client.clone();
        }
        clients
            .iter()
            .filter(|c| listener(c))
            .min_by_key(|c| (&c.name, &c.id))
            .map(|c| c.id.clone())
            .unwrap_or_default()
    }
    /// A new status from the backend; another room (New chat, or a choice
    /// made on another device) starts its conversation from the beginning.
    fn apply(&mut self, ctx: &Context<Self>, status: AudioStatus) {
        if status.room_id != self.room {
            self.room = status.room_id.clone();
            self.cursor = 0;
            self.messages.clear();
            self.syncing = false;
        }
        if let Some(error) = &status.error
            && self.error.is_empty()
        {
            self.error = error.clone();
        }
        self.status = status;
        self.sync(ctx);
    }
    fn message(&self, m: &ChatMessage) -> Html {
        if m.memo_kind == NOTES {
            return html! {<li class="audio-heard" key={m.messageid.clone()} data-kind="heard">
                <div class="audio-message-meta"><strong>{icon("audio")}{t("Heard")}</strong><span>{m.client_name.clone().unwrap_or_default()}</span><time>{clock(&m.created_at)}</time></div>
                <p>{for m.text.lines().map(|line| html!{<span class="audio-line">{line.to_string()}</span>})}</p>
            </li>};
        }
        if m.role == "user" {
            let label = if m.text == "Note" {
                t("You asked for a note")
            } else if m.text == "Solve" {
                t("You pressed Solve")
            } else {
                m.text.clone()
            };
            return html! {<li class="audio-request" key={m.messageid.clone()} data-kind="request"><span>{icon(if m.text=="Note"{"file"}else{"spark"})}{label}</span><time>{clock(&m.created_at)}</time></li>};
        }
        let running = m.status == "running";
        let failed = m.status == "error";
        html! {<li class="audio-answer" key={m.messageid.clone()} data-kind="answer" data-status={m.status.clone()} data-testid="audio-answer">
            <div class="audio-message-meta"><strong>{icon("spark")}{if m.memo_kind=="note"{t("Note")}else{t("AI answer")}}</strong><span>{if running {if m.stage.is_empty(){t("Thinking…")}else{t(&m.stage)}} else if failed {t("Request failed")} else {t("Answered")}}</span>if !m.model.is_empty(){<small>{format!("{} · {}",m.provider,m.model)}</small>}</div>
            if failed {<p class="audio-empty">{m.error.clone().unwrap_or_default()}</p>}
            else if m.text.is_empty() && running {<p class="audio-empty">{t("Reading the notes…")}</p>}
            else {<div class="markdown answer-content">{crate::markdown(&m.text)}</div>}
        </li>}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listeners_need_to_be_online_permitted_and_capable() {
        let mut c = CaptureClient {
            online: true,
            can_capture: true,
            audio: true,
            ..Default::default()
        };
        assert!(listener(&c));
        c.audio = false;
        assert!(!listener(&c));
        c.audio = true;
        c.online = false;
        assert!(!listener(&c));
        assert_eq!(minutes(0.0), "0:00");
        assert_eq!(minutes(61.4), "1:01");
        assert_eq!(minutes(3599.6), "60:00");
    }
    fn message(id: &str, sync: i64, sequence: i64, text: &str) -> ChatMessage {
        ChatMessage {
            messageid: id.into(),
            messagesyncid: sync,
            sequence,
            text: text.into(),
            ..Default::default()
        }
    }
    #[test]
    fn sync_batches_rewrite_messages_in_place_and_keep_room_order() {
        let mut all = vec![];
        merge(&mut all, vec![message("notes", 1, 1, "Hello")], 0);
        merge(&mut all, vec![message("ask", 2, 2, "Solve")], 0);
        // The notes message grows: same id, newer sync id, same place.
        merge(&mut all, vec![message("notes", 3, 1, "Hello\nWorld")], 0);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].text, "Hello\nWorld");
        assert_eq!(all[1].messageid, "ask");
        // An older copy never replaces a newer one.
        merge(&mut all, vec![message("notes", 1, 1, "Hello")], 0);
        assert_eq!(all[0].text, "Hello\nWorld");
        // A cleared room forgets what came before the clear.
        merge(&mut all, vec![message("later", 5, 5, "Again")], 3);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].messageid, "later");
    }
}
