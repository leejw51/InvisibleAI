use crate::i18n::{t, tf};
use crate::tags::{MAX_TAGS, normalize_tag};
use crate::{
    api::{ConnectionState, api},
    icons::icon,
    message_cache,
};
use base64::Engine;
use causewaybay_shared::registry::PendingTransfer;
use causewaybay_shared::*;
use gloo_timers::callback::{Interval, Timeout};
use serde_json::{Value, json};
use web_sys::{HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement};
use yew::prelude::*;
#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: crate::i18n::Language,
    pub owner: String,
    pub data_epoch: i64,
    pub clients: Vec<CaptureClient>,
    pub devices: crate::devices::Choices,
    pub choose_device: Callback<String>,
    pub settings: Settings,
    pub ai_saved: Callback<()>,
    pub connection: ConnectionState,
    pub admin: bool,
    pub thought: bool,
    pub thought_epoch: u64,
    pub navigate: Callback<String>,
    pub backend_settings: Callback<()>,
    pub library_attachment: Option<FileInfo>,
    pub attachment_consumed: Callback<()>,
    pub copy: Callback<String>,
}
struct Stream(String);
impl Drop for Stream {
    fn drop(&mut self) {
        crate::api::unsubscribe_room(&self.0);
    }
}
pub struct Chat {
    rooms: Vec<Room>,
    rooms_version: u64,
    thought_sync_version: u64,
    cutoffs: std::collections::HashMap<String, i64>,
    room: String,
    room_version: u64,
    draft_version: u64,
    messages: Vec<ChatMessage>,
    prompt: String,
    mode: InputMode,
    agent: bool,
    attachments: Vec<FileInfo>,
    /// A video upload in progress: its phase and how far it has got.
    upload_progress: Option<crate::uploads::Progress>,
    capture: ContinuousStatus,
    capture_loaded: bool,
    /// The Capture page is restoring a room whose conversation is not known
    /// yet: the hero's shape depends on it, so the page stays hidden until
    /// the first sync lands rather than switch layouts in front of the user.
    settling: bool,
    ai: Option<AiInfo>,
    skills: Vec<String>,
    error: String,
    busy: bool,
    creating: bool,
    restore_room: bool,
    checking: bool,
    retry: Option<ThinkRequest>,
    stream: Option<Stream>,
    cache: futures_channel::mpsc::UnboundedSender<message_cache::Write>,
    _poll: Interval,
    provider: String,
    model: String,
    models: Value,
    rename: Option<String>,
    removing_room: bool,
    clearing_room: bool,
    deleting: bool,
    data_cutoff: i64,
    multi_capture: bool,
    editing: Option<ChatMessage>,
    edit_text: String,
    removing_message: Option<ChatMessage>,
    /// The message whose tags are being edited, the list as edited so far, and
    /// the tag being typed.
    tagging: Option<ChatMessage>,
    tag_draft: Vec<String>,
    tag_input: String,
    saving_tags: bool,
    /// A tag chip opens the Wiki on the second tap only: the first tap arms
    /// the chip (message id, tag) for a moment, so a stray touch on a tag
    /// beside a message does not leave the chat.
    armed_tag: Option<(String, String)>,
    arm_timer: Option<Timeout>,
    /// Tags are shown and edited only while the Wiki is on: the chat mirrors
    /// the Wiki's tags, so without it there would be nothing to show.
    wiki_enabled: bool,
    /// With automatic Wiki updates on, an untagged complete message is still
    /// being analyzed, and says so instead of offering to add tags.
    wiki_auto: bool,
    bottom: NodeRef,
    scroll: bool,
    /// Replies that solved a captured problem while this chat watched, each
    /// with its place in this visit's streak; they play the celebration once.
    celebrations: std::collections::HashMap<String, usize>,
    /// Message ids this tab posted: only the tab that asked for a transfer
    /// sends it, so two tabs on one room never sign the same request.
    posted: std::collections::HashSet<String>,
    /// Transfers the AI prepared that this tab is sending, by request id,
    /// with the step under way or why it stopped.
    transfers: std::collections::HashMap<String, Transfer>,
    /// After Capture: the screenshot, then the answer, each eased into view
    /// once as it arrives, so a hurried reader never has to scroll.
    focus: Option<CaptureFocus>,
    /// The message to ease into view on the next render, and whether this is
    /// a settle (the same message, now complete) rather than its first showing.
    focus_to: Option<(String, bool)>,
    tween: Option<crate::tween::ScrollTween>,
    /// The message glowing right now, and the timer that ends the glow. Kept
    /// in state because Yew rewrites `class` on every render.
    glow: Option<String>,
    glow_generation: u64,
    glow_timer: Option<Timeout>,
    _notifications: Option<crate::api::Notifications>,
}
/// A prepared transfer this tab is sending hands-off: the same checks and
/// signing as the Wallet's Send form, step by step, reported to the backend
/// so the card follows. One that stops before broadcast stays `pending` for
/// the Wallet page to finish.
#[derive(Clone)]
struct Transfer {
    coin: causewaybay_shared::registry::Coin,
    from: String,
    to: String,
    /// What is happening now, as a translatable label; empty once done.
    step: &'static str,
    error: Option<String>,
}
/// Following one capture: the newest message before it, and how far the
/// capture post and its reply have been shown.
struct CaptureFocus {
    after: i64,
    capture: Stage,
    reply: Stage,
}
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Stage {
    Waiting,
    Started,
    Done,
}
pub enum Msg {
    Load,
    /// A focused message glows for a moment, then the glow ends.
    Glow(String),
    GlowEnd(u64),
    Thought(bool),
    CurrentThought(u64, u64, Result<Option<Room>, String>),
    Notification(Value),
    /// Pick the AI that answers in this chat; saved to the account policy so
    /// On-device, Hybrid and Cloud modes follow the choice on every device.
    /// The shared AI picker changed the provider or model.
    AiChosen(String, String),
    Rename,
    RoomTitle(String),
    SaveRoom,
    Renamed(Result<Room, String>),
    RemoveRoom,
    ClearRoom,
    MultiCapture,
    Snapshot(String, u64, Result<FileInfo, String>),
    CancelMulti,
    ConfirmRoom,
    Removed(String, Result<Value, String>),
    Edit(ChatMessage),
    EditText(String),
    SaveEdit,
    Changed(Result<ChatMessage, String>),
    RemoveMessage(ChatMessage),
    ConfirmMessage,
    CancelEdit,
    /// Tag editing on one user message: open, type, add the typed tag, remove
    /// one, save to the memo (the Wiki and every device follow), or cancel.
    EditTags(ChatMessage),
    TagInput(String),
    AddTag,
    RemoveTag(String),
    SaveTags,
    TagsSaved(String, Result<Memo, String>),
    CancelTags,
    /// First tap on a tag chip: arm it (message id, tag) until the second tap
    /// opens the Wiki or the moment passes.
    ArmTag(String, String),
    DisarmTag,
    /// A line the chat kept as a note is sent to the AI after all.
    AskInstead(String),
    /// Keep a message in the Wiki, or take it out, overriding the analysis.
    WikiKeep(String, bool),
    Assistance(Result<Value, String>),
    StopAnswer(String),
    Rooms(u64, Result<Vec<Room>, String>),
    New,
    Created(u64, bool, Result<Room, String>),
    Room(String),
    ChooseRoom(String),
    UseSession(String),
    Synced(u64, String, Result<MessageSync, String>),
    Event(ChatMessage),
    Prompt(String),
    Mode(InputMode),
    Agent(bool),
    SaveMemo,
    CaptureRequested(String, u64, Result<ContinuousStatus, String>),
    Send(bool),
    Sent(String, u64, Result<Memo, String>),
    Retry,
    Upload(Vec<web_sys::File>),
    Uploaded(String, u64, Vec<Result<FileInfo, String>>),
    UploadProgress(String, u64, crate::uploads::Progress),
    Detach(String),
    Tick,
    Status(String, Result<ContinuousStatus, String>),
    Ai(Result<AiInfo, String>),
    /// The account's live skills, named in the AI & tools panel.
    Skills(Result<Value, String>),
    CheckAi,
    Error(String),
    /// A hands-off transfer, step by step: prepared (balances read, transaction
    /// built), broadcast (hash), arrived (receipt), settled (balances after).
    TransferPrepared(String, Result<crate::wallet::Prepared, String>),
    TransferBroadcast(String, Result<String, String>),
    TransferArrived(String, String, Result<bool, String>),
    TransferSettled(String, String, Result<(String, String), String>),
    TransferReported(String, Result<Value, String>),
}
impl Component for Chat {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        ctx.link().send_future(async {
            Msg::Assistance(api("GET", "/api/assistance/preferences", None).await)
        });
        if ctx.props().thought {
            ctx.link()
                .send_message(Msg::Thought(ctx.props().thought_epoch > 0));
        }
        let preferences = crate::ai_settings::preferences(&ctx.props().owner);
        let (provider, model) = crate::ai_settings::ai_choice(&ctx.props().settings, &preferences);
        let link = ctx.link().clone();
        Self {
            rooms: vec![],
            rooms_version: 0,
            thought_sync_version: 0,
            cutoffs: Default::default(),
            room: String::new(),
            room_version: 0,
            draft_version: 0,
            messages: vec![],
            prompt: String::new(),
            // Full screenshots let one-click capture answer visual questions too.
            // An explicit local Text mode preference still uses OCR only.
            mode: if causewaybay_shared::text_only_provider(&provider)
                || preferences["input_mode"] == "text"
            {
                InputMode::Text
            } else {
                InputMode::Image
            },
            agent: false,
            attachments: vec![],
            upload_progress: None,
            capture: ContinuousStatus::default(),
            capture_loaded: false,
            settling: ctx.props().thought && ctx.props().thought_epoch == 0,
            ai: None,
            skills: vec![],
            error: String::new(),
            busy: false,
            // The Capture page resolves its room on mount: it starts in the
            // resolving state so the first frame already shows the conversation
            // area, never the empty state the room swap would replace.
            creating: ctx.props().thought,
            restore_room: true,
            checking: false,
            retry: None,
            stream: None,
            cache: message_cache::writer(
                ctx.props().owner.clone(),
                ctx.link().callback(Msg::Error),
            ),
            provider,
            model,
            models: preferences["models"].clone(),
            rename: None,
            removing_room: false,
            clearing_room: false,
            deleting: false,
            data_cutoff: ctx.props().data_epoch,
            multi_capture: false,
            editing: None,
            edit_text: String::new(),
            removing_message: None,
            tagging: None,
            tag_draft: vec![],
            tag_input: String::new(),
            saving_tags: false,
            armed_tag: None,
            arm_timer: None,
            wiki_enabled: false,
            wiki_auto: false,
            bottom: NodeRef::default(),
            scroll: false,
            celebrations: Default::default(),
            posted: Default::default(),
            transfers: Default::default(),
            focus: None,
            focus_to: None,
            tween: None,
            glow: None,
            glow_generation: 0,
            glow_timer: None,
            _notifications: crate::api::notifications(ctx.link().callback(Msg::Notification)).ok(),
            _poll: Interval::new(2000, move || link.send_message(Msg::Tick)),
        }
    }
    fn changed(&mut self, ctx: &Context<Self>, old: &Props) -> bool {
        if ctx.props().data_epoch > self.data_cutoff {
            ctx.link().send_message(Msg::Notification(
                json!({"kind":"data.deleted","data":{"messagesyncid":ctx.props().data_epoch}}),
            ));
        }
        // Wallet login can mount Chat before the asynchronous settings arrive.
        if (old.settings.user_ai != ctx.props().settings.user_ai
            || self.provider.is_empty()
            || (matches!(self.provider.as_str(), "apple" | "mlx")
                && self.model
                    != crate::ai_settings::provider_model(
                        &ctx.props().settings,
                        &json!({}),
                        &self.provider,
                    ))
            || (ctx.props().settings.local_ai.local_only
                && !matches!(self.provider.as_str(), "apple" | "mlx")))
            && !ctx.props().settings.provider.is_empty()
        {
            let (provider, model) = crate::ai_settings::ai_choice(
                &ctx.props().settings,
                &crate::ai_settings::preferences(&ctx.props().owner),
            );
            // A model the user is typing for the same cloud provider survives the
            // account round trip; only a provider change replaces it.
            if provider != self.provider
                || self.model.is_empty()
                || matches!(provider.as_str(), "apple" | "mlx")
            {
                self.model = model;
            }
            self.provider = provider;
            if causewaybay_shared::text_only_provider(&self.provider) {
                self.mode = InputMode::Text;
            }
        }
        if ctx.props().thought && (!old.thought || ctx.props().thought_epoch != old.thought_epoch) {
            ctx.link().send_message(Msg::Thought(true));
        }
        if old.connection != ctx.props().connection
            && ctx.props().connection == ConnectionState::Connected
        {
            self.reconcile_connection_error(ctx.props().connection);
            self.load_rooms(ctx);
            self.load_current_thought(ctx);
        }
        true
    }
    fn rendered(&mut self, ctx: &Context<Self>, _: bool) {
        if let Some((id, settle)) = self.focus_to.take() {
            let target = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| {
                    d.query_selector(&format!(".chat-message[data-message-id=\"{id}\"]"))
                        .ok()
                        .flatten()
                })
                .and_then(|e| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(e).ok());
            // A settle leaves a reader who is already looking at it alone.
            if let Some(target) = target
                && !(settle && crate::tween::in_view(&target))
            {
                self.tween = crate::tween::ease_into_view(&target, FOCUS_MS);
                ctx.link().send_message(Msg::Glow(id));
            }
            self.scroll = false;
        }
        if self.scroll {
            if let Some(el) = self.bottom.cast::<web_sys::HtmlElement>()
                && let Some(parent) = el.parent_element()
            {
                parent.set_scroll_top(parent.scroll_height());
                if self.messages.iter().any(|m| !m.deleted) {
                    parent.scroll_into_view_with_bool(false);
                }
            }
            self.scroll = false;
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Glow(id) => {
                self.glow = Some(id);
                self.glow_generation += 1;
                let generation = self.glow_generation;
                let link = ctx.link().clone();
                self.glow_timer = Some(Timeout::new(GLOW_MS, move || {
                    link.send_message(Msg::GlowEnd(generation))
                }));
            }
            Msg::GlowEnd(generation) => {
                if generation != self.glow_generation {
                    return false;
                }
                self.glow = None;
                self.glow_timer = None;
            }
            Msg::Notification(v) => {
                if matches!(
                    v["kind"].as_str(),
                    Some("notifications.ready" | "room.thought" | "room.cleared" | "room.deleted")
                ) {
                    self.load_current_thought(ctx);
                }
                if v["kind"] == "notifications.ready" {
                    self.load_rooms(ctx);
                }
                if v["kind"] == "message.deleted"
                    && let Ok(message) =
                        serde_json::from_value::<ChatMessage>(v["data"]["message"].clone())
                {
                    let _ = self
                        .cache
                        .unbounded_send(message_cache::Write::Message(Box::new(message.clone())));
                    self.merge(message);
                }
                if v["kind"] == "data.deleted" {
                    let cutoff = v["data"]["messagesyncid"].as_i64().unwrap_or(0);
                    if cutoff > self.data_cutoff {
                        self.data_cutoff = cutoff;
                        let _ = self
                            .cache
                            .unbounded_send(message_cache::Write::ClearAll(cutoff));
                        self.messages.retain(|m| m.messagesyncid > cutoff);
                        self.reset_draft();
                        self.editing = None;
                        self.removing_message = None;
                    }
                    ctx.link().send_message(Msg::Load);
                }
                if matches!(v["kind"].as_str(), Some("room.deleted" | "room.cleared"))
                    && let Some(room) = v["data"]["roomid"].as_str()
                {
                    let cutoff = v["data"]["messagesyncid"].as_i64().unwrap_or(0);
                    let entry = self.cutoffs.entry(room.into()).or_default();
                    let new_erasure = cutoff > *entry;
                    *entry = (*entry).max(cutoff);
                    let _ = self
                        .cache
                        .unbounded_send(message_cache::Write::ClearRoom(room.into(), cutoff));
                    if room == self.room && new_erasure {
                        self.messages.retain(|m| m.messagesyncid > cutoff);
                        self.attachments.clear();
                        self.prompt.clear();
                        self.multi_capture = false;
                        self.capture = ContinuousStatus::default();
                        self.draft_version += 1;
                        self.busy = false;
                        self.retry = None;
                        self.editing = None;
                        self.removing_message = None;
                    }
                }
                if v["kind"].as_str().is_some_and(|k| k.starts_with("room.")) {
                    ctx.link().send_message(Msg::Load);
                }
            }
            Msg::Thought(fresh) => {
                let version = self.begin_room();
                self.restore_room = !fresh;
                // A fresh room has no conversation; a restored one is unknown.
                self.settling = !fresh;
                ctx.link().send_future(async move {
                    Msg::Created(
                        version,
                        false,
                        api("POST", "/api/rooms/thought", Some(json!({"fresh":fresh}))).await,
                    )
                });
            }
            Msg::CurrentThought(version, room_version, result) => {
                if version != self.thought_sync_version
                    || room_version != self.room_version
                    || !ctx.props().thought
                    || !self.restore_room
                    || self.creating
                {
                    return false;
                }
                match result {
                    Ok(Some(room)) if room.roomid != self.room => {
                        self.rooms_version += 1;
                        self.rooms.retain(|r| r.roomid != room.roomid);
                        self.rooms.insert(0, room.clone());
                        self.error.clear();
                        return Component::update(self, ctx, Msg::Room(room.roomid));
                    }
                    Ok(None) => ctx.link().send_message(Msg::Thought(false)),
                    Err(e) => self.error = e,
                    _ => return false,
                }
            }
            Msg::AiChosen(provider, model) => {
                let switched = provider != self.provider;
                self.provider = provider;
                self.model = model;
                self.models[&self.provider] = json!(self.model);
                if switched && causewaybay_shared::text_only_provider(&self.provider) {
                    self.mode = InputMode::Text;
                }
                self.persist(ctx);
            }
            Msg::Rename => {
                self.rename = self
                    .rooms
                    .iter()
                    .find(|r| r.roomid == self.room)
                    .map(|r| r.title.clone());
            }
            Msg::RoomTitle(v) => self.rename = Some(v),
            Msg::SaveRoom => {
                if let Some(title) = self.rename.clone() {
                    let room = self.room.clone();
                    ctx.link().send_future(async move {
                        Msg::Renamed(
                            api(
                                "PATCH",
                                &format!("/api/rooms/{room}"),
                                Some(json!({"title":title})),
                            )
                            .await,
                        )
                    });
                }
            }
            Msg::Renamed(r) => match r {
                Ok(room) => {
                    self.rename = None;
                    if let Some(old) = self.rooms.iter_mut().find(|r| r.roomid == room.roomid) {
                        *old = room;
                    }
                }
                Err(e) => self.error = e,
            },
            Msg::RemoveRoom => {
                self.removing_room = true;
                self.clearing_room = false;
            }
            Msg::ClearRoom => {
                self.removing_room = true;
                self.clearing_room = true;
            }
            Msg::ConfirmRoom => {
                if self.deleting {
                    return false;
                }
                self.deleting = true;
                let room = self.room.clone();
                let clear = self.clearing_room;
                ctx.link().send_future(async move {
                    let result = if clear {
                        api("POST", &format!("/api/rooms/{room}/clear"), None).await
                    } else {
                        api("DELETE", &format!("/api/rooms/{room}"), None).await
                    };
                    Msg::Removed(room, result)
                });
            }
            Msg::Removed(room, r) => {
                self.deleting = false;
                match r {
                    Ok(value) => {
                        self.rooms.retain(|r| r.roomid != room);
                        let cutoff = if value["deleted"] == true {
                            i64::MAX
                        } else {
                            value["messagesyncid"].as_i64().unwrap_or(0)
                        };
                        self.cutoffs.insert(room.clone(), cutoff);
                        let _ = self
                            .cache
                            .unbounded_send(message_cache::Write::ClearRoom(room, cutoff));
                        self.removing_room = false;
                        self.stream = None;
                        self.room.clear();
                        self.messages.clear();
                        self.reset_draft();
                        if ctx.props().thought {
                            ctx.link().send_message(Msg::Thought(false));
                        } else {
                            ctx.link().send_message(Msg::Load);
                        }
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Edit(m) => {
                self.edit_text = m.text.clone();
                self.editing = Some(m);
                self.removing_message = None;
            }
            Msg::EditText(v) => self.edit_text = v,
            Msg::SaveEdit => {
                if let Some(m) = &self.editing {
                    let url = format!("/api/rooms/{}/messages/{}", m.roomid, m.messageid);
                    let body = json!({"text":self.edit_text,"messagesyncid":m.messagesyncid});
                    ctx.link().send_future(async move {
                        Msg::Changed(api("PATCH", &url, Some(body)).await)
                    });
                }
            }
            Msg::RemoveMessage(m) => {
                self.removing_message = Some(m);
                self.editing = None;
            }
            Msg::ConfirmMessage => {
                if let Some(m) = &self.removing_message {
                    let url = format!("/api/rooms/{}/messages/{}", m.roomid, m.messageid);
                    let body = json!({"messagesyncid":m.messagesyncid});
                    ctx.link().send_future(async move {
                        Msg::Changed(api("DELETE", &url, Some(body)).await)
                    });
                }
            }
            Msg::Changed(r) => match r {
                Ok(m) => {
                    self.editing = None;
                    self.removing_message = None;
                    self.merge(m);
                }
                Err(e) => self.error = e,
            },
            Msg::EditTags(m) => {
                // The category is not edited here; it follows the message.
                self.tag_draft = m
                    .tags
                    .iter()
                    .filter(|t| !causewaybay_shared::is_fixed_tag(t))
                    .cloned()
                    .collect();
                self.tag_input.clear();
                self.saving_tags = false;
                self.tagging = Some(m);
                self.editing = None;
                self.removing_message = None;
            }
            Msg::TagInput(v) => {
                // A comma (or Enter) finishes the tag being typed; spaces stay, so
                // "machine learning" becomes one tag, machine-learning.
                if v.ends_with(',') {
                    self.tag_input = v.trim_end_matches(',').to_string();
                    ctx.link().send_message(Msg::AddTag);
                } else {
                    self.tag_input = v;
                }
            }
            Msg::Assistance(r) => {
                if let Ok(v) = r {
                    self.wiki_enabled = v["wiki_enabled"] == true;
                    self.wiki_auto = v["wiki_auto_update"] == true;
                }
            }
            Msg::AddTag => {
                if let Some(tag) = normalize_tag(&self.tag_input)
                    && !self.tag_draft.contains(&tag)
                    && self.tag_draft.len() < MAX_TAGS
                {
                    self.tag_draft.push(tag);
                }
                self.tag_input.clear();
            }
            Msg::RemoveTag(tag) => self.tag_draft.retain(|t| *t != tag),
            Msg::SaveTags => {
                if let Some(m) = &self.tagging {
                    if let Some(tag) = normalize_tag(&self.tag_input)
                        && !self.tag_draft.contains(&tag)
                        && self.tag_draft.len() < MAX_TAGS
                    {
                        self.tag_draft.push(tag);
                    }
                    self.tag_input.clear();
                    self.saving_tags = true;
                    let memo = m.memo_id.clone();
                    let url = format!("/api/memos/{}", crate::api::encode(&memo));
                    let body = json!({"tags":self.tag_draft});
                    ctx.link().send_future(async move {
                        Msg::TagsSaved(memo, api("PATCH", &url, Some(body)).await)
                    });
                }
            }
            Msg::TagsSaved(memo, r) => {
                self.saving_tags = false;
                match r {
                    Ok(saved) => {
                        // Show the saved list at once; the retagged revision arrives
                        // through room sync and confirms it.
                        for m in self.messages.iter_mut() {
                            if m.memo_id == memo {
                                m.tags = saved.tags.clone();
                            }
                        }
                        self.tagging = None;
                        self.tag_draft.clear();
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::AskInstead(text) => {
                if self.busy || self.deleting || self.room.is_empty() || text.trim().is_empty() {
                    return false;
                }
                if !self.ai_ready(ctx) {
                    self.error = "Enable an AI provider to ask this".into();
                    return true;
                }
                self.error.clear();
                let mut req = self.request(ctx, false);
                req.prompt = text;
                req.file_ids.clear();
                req.ask = true;
                self.retry = Some(req.clone());
                self.send(ctx, req);
            }
            Msg::WikiKeep(memo, keep) => {
                self.error.clear();
                let url = format!("/api/memos/{}", crate::api::encode(&memo));
                ctx.link().send_future(async move {
                    Msg::TagsSaved(memo, api("PATCH", &url, Some(json!({"wiki":keep}))).await)
                });
            }
            Msg::ArmTag(messageid, tag) => {
                self.armed_tag = Some((messageid, tag));
                let link = ctx.link().clone();
                self.arm_timer = Some(Timeout::new(ARM_MS, move || {
                    link.send_message(Msg::DisarmTag)
                }));
            }
            Msg::DisarmTag => {
                self.armed_tag = None;
                self.arm_timer = None;
            }
            Msg::CancelTags => {
                self.tagging = None;
                self.tag_draft.clear();
                self.tag_input.clear();
                self.saving_tags = false;
            }
            Msg::CancelEdit => {
                self.editing = None;
                self.removing_message = None;
                self.rename = None;
                self.removing_room = false;
            }
            Msg::StopAnswer(id) => {
                ctx.link().send_future(async move {
                    match api::<Value>("POST", &format!("/api/memos/{id}/cancel"), None).await {
                        Ok(_) => Msg::Tick,
                        Err(e) => Msg::Error(e),
                    }
                });
            }
            Msg::Load => {
                self.load_rooms(ctx);
                ctx.link()
                    .send_future(async { Msg::Ai(api("GET", "/api/ai", None).await) });
                ctx.link()
                    .send_future(async { Msg::Skills(api("GET", "/api/skills", None).await) });
            }
            Msg::Rooms(version, r) => {
                if version != self.rooms_version {
                    return false;
                }
                match r {
                    Ok(v) => {
                        if !self.room.is_empty() && !v.iter().any(|r| r.roomid == self.room) {
                            let _ = self.cache.unbounded_send(message_cache::Write::RemoveRoom(
                                self.room.clone(),
                            ));
                            self.stream = None;
                            self.messages.clear();
                            self.room.clear();
                            self.capture = ContinuousStatus::default();
                            self.reset_draft();
                        }
                        self.rooms = v;
                        if self.room.is_empty()
                            && self.restore_room
                            && !self.creating
                            && !ctx.props().thought
                            && let Some(room) = linked_room()
                                .and_then(|id| self.rooms.iter().find(|r| r.roomid == id))
                                .or_else(|| self.rooms.first())
                        {
                            ctx.link().send_message(Msg::Room(room.roomid.clone()));
                        }
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::New => {
                let version = self.begin_room();
                let title = format!("Conversation {}", self.rooms.len() + 1);
                ctx.link().send_future(async move {
                    Msg::Created(
                        version,
                        true,
                        api("POST", "/api/rooms", Some(json!({"title":title}))).await,
                    )
                });
            }
            Msg::Created(version, new, r) => {
                if version != self.room_version {
                    return false;
                }
                self.creating = false;
                self.busy = false;
                match r {
                    Ok(room) => {
                        self.error.clear();
                        self.restore_room = true;
                        self.rooms_version += 1;
                        self.rooms.retain(|r| r.roomid != room.roomid);
                        self.rooms.insert(0, room.clone());
                        if new {
                            ctx.props().navigate.emit("chat".into());
                        }
                        let changed = Component::update(self, ctx, Msg::Room(room.roomid));
                        if !new {
                            self.load_current_thought(ctx);
                        }
                        return changed;
                    }
                    Err(e) => {
                        self.settling = false;
                        self.error = e;
                    }
                }
            }
            Msg::UseSession(room) => {
                if room == self.room || self.creating || self.deleting {
                    return false;
                }
                let version = self.begin_room();
                ctx.link().send_future(async move {
                    Msg::Created(
                        version,
                        false,
                        api("POST", "/api/rooms/thought", Some(json!({"room_id":room}))).await,
                    )
                });
            }
            Msg::ChooseRoom(room) => {
                if ctx.props().thought {
                    ctx.props().navigate.emit("chat".into());
                }
                ctx.link().send_message(Msg::Room(room));
            }
            Msg::Room(room) => {
                if room == self.room && self.stream.is_some() {
                    return false;
                }
                self.focus = None;
                self.tween = None;
                self.glow = None;
                self.stream = None;
                self.creating = false;
                self.restore_room = true;
                self.room_version += 1;
                self.draft_version += 1;
                let version = self.room_version;
                self.room = room.clone();
                // Keep an explicit Cloud deep link on the selected room without
                // adding history entries or changing the existing menu routes.
                if !ctx.props().thought
                    && linked_room().is_some()
                    && let Some(history) = web_sys::window().and_then(|w| w.history().ok())
                {
                    let _ = history.replace_state_with_url(
                        &wasm_bindgen::JsValue::NULL,
                        "",
                        Some(&format!("/chat?room={room}")),
                    );
                }
                self.busy = false;
                self.messages.clear();
                self.editing = None;
                self.removing_message = None;
                self.rename = None;
                self.removing_room = false;
                self.capture = ContinuousStatus::default();
                self.capture_loaded = false;
                self.retry = None;
                self.attachments.clear();
                self.multi_capture = false;
                self.prompt.clear();
                self.error.clear();
                self.scroll = true;
                if let Some(file) = ctx.props().library_attachment.clone() {
                    self.busy = true;
                    ctx.props().attachment_consumed.emit(());
                    let room = room.clone();
                    let draft_version = self.draft_version;
                    let clear_epoch = *self.cutoffs.get(&room).unwrap_or(&0);
                    let data_epoch = self.data_cutoff;
                    ctx.link().send_future(async move {
                        let result = async {
                            let response = gloo_net::http::Request::get(&format!("/api/files/{}/download", file.id))
                                .send().await.map_err(|_|"Could not download this file")?;
                            if !response.ok() { return Err("This file is no longer available".into()); }
                            let bytes = response.binary().await.map_err(|_|"Could not read this file")?;
                            if file.mime.starts_with("video/") {
                                let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
                                let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(|_|"Could not read this file")?;
                                let target = crate::uploads::VideoTarget { room: Some(room.clone()), clear_epoch: Some(clear_epoch), data_epoch: Some(data_epoch), wiki: false };
                                return crate::uploads::upload_video(blob, file.name.clone(), target, None).await;
                            }
                            api("POST", "/api/files", Some(json!({"name":file.name,"data_base64":base64::engine::general_purpose::STANDARD.encode(bytes),"room_id":room,"clear_epoch":clear_epoch,"data_epoch":data_epoch}))).await
                        }.await;
                        Msg::Uploaded(room, draft_version, vec![result])
                    });
                }
                ctx.link().send_future(async move {
                    // Reconcile every page before displaying a room. Old local
                    // content must never flash on login before erasure replay.
                    let result = async {
                        let mut snapshot = MessageSync::default();
                        loop {
                            let batch: MessageSync = api(
                                "GET",
                                &format!("/api/rooms/{room}/sync?after={}", snapshot.next),
                                None,
                            )
                            .await?;
                            snapshot.next = batch.next;
                            snapshot.cleared_through =
                                snapshot.cleared_through.max(batch.cleared_through);
                            snapshot.messages.extend(batch.messages);
                            if !batch.has_more {
                                break;
                            }
                        }
                        Ok(snapshot)
                    }
                    .await;
                    Msg::Synced(version, room, result)
                });
                // Room changes invalidate earlier list requests. Take a fresh
                // snapshot here so login cannot lose saved rooms when SSE
                // replay/open arrives before room creation finishes.
                self.load_rooms(ctx);
                ctx.link().send_message(Msg::Tick);
            }
            Msg::Synced(version, room, r) => {
                if room != self.room || version != self.room_version {
                    return false;
                }
                self.settling = false;
                let mut after = 0;
                match r {
                    Ok(snapshot) => {
                        after = snapshot.next;
                        let cutoff = self.cutoffs.entry(room.clone()).or_default();
                        *cutoff = (*cutoff).max(snapshot.cleared_through);
                        let _ = self
                            .cache
                            .unbounded_send(message_cache::Write::ClearRoom(room.clone(), *cutoff));
                        for m in snapshot.messages {
                            let _ = self
                                .cache
                                .unbounded_send(message_cache::Write::Message(Box::new(m.clone())));
                            self.merge(m);
                        }
                        self.start_transfers(ctx);
                    }
                    Err(e) => self.error = e,
                }
                // Drop the old guard before registering its replacement. Both use
                // the room ID as a key, so dropping it afterwards closes the new stream.
                self.stream = None;
                match crate::api::subscribe_room(
                    &room,
                    after,
                    ctx.link().callback(Msg::Event),
                    ctx.link().callback(Msg::Notification),
                ) {
                    Ok(()) => self.stream = Some(Stream(room)),
                    Err(e) => self.error = e,
                }
            }
            Msg::Event(m) => {
                if m.roomid == self.room {
                    // Follow streaming output only while the reader is near the end.
                    self.scroll |= self
                        .bottom
                        .cast::<web_sys::HtmlElement>()
                        .and_then(|el| el.parent_element())
                        .is_some_and(|el| {
                            el.scroll_height() - el.scroll_top() - el.client_height() < 90
                        });
                    let _ = self
                        .cache
                        .unbounded_send(message_cache::Write::Message(Box::new(m.clone())));
                    self.merge(m);
                    self.start_transfers(ctx);
                    if self.focus.is_some() {
                        // The capture focus decides where the list goes.
                        self.scroll = false;
                    }
                }
            }
            Msg::Prompt(v) => self.prompt = v,
            Msg::TransferPrepared(id, result) => {
                let Some(transfer) = self.transfers.get_mut(&id) else {
                    return false;
                };
                match result {
                    Ok(prepared) => {
                        // The balances go to the backend first, and only a request
                        // still pending there is signed: one cancelled meanwhile
                        // (Dismiss on the Wallet page, say) is never sent.
                        let (from_before, to_before) = prepared.balances_before();
                        transfer.step = "Sending…";
                        let (coin, from) = (transfer.coin.clone(), transfer.from.clone());
                        let url = format!("/api/wallet/requests/{id}");
                        ctx.link().send_future(async move {
                            let still_pending = api::<Value>(
                                "PATCH",
                                &url,
                                Some(json!({"status":"pending","from_before":from_before,"to_before":to_before})),
                            )
                            .await;
                            Msg::TransferBroadcast(
                                id,
                                match still_pending {
                                    Ok(_) => crate::wallet::hands_off::send(&coin, &from, &prepared).await,
                                    Err(e) => Err(e),
                                },
                            )
                        });
                    }
                    Err(e) => {
                        transfer.step = "";
                        transfer.error = Some(e);
                    }
                }
            }
            Msg::TransferBroadcast(id, result) => {
                let Some(transfer) = self.transfers.get_mut(&id) else {
                    return false;
                };
                match result {
                    Ok(hash) => {
                        Self::report(ctx, &id, json!({"status":"sent","hash":hash}));
                        transfer.step = "Waiting for the network…";
                        let coin = transfer.coin.clone();
                        ctx.link().send_future(async move {
                            let arrived = crate::wallet::hands_off::arrival(&coin, &hash).await;
                            Msg::TransferArrived(id, hash, arrived)
                        });
                    }
                    Err(e) => {
                        transfer.step = "";
                        transfer.error = Some(e);
                    }
                }
            }
            Msg::TransferArrived(id, hash, result) => {
                let Some(transfer) = self.transfers.get_mut(&id) else {
                    return false;
                };
                match result {
                    Ok(true) => {
                        let (coin, from, to) = (
                            transfer.coin.clone(),
                            transfer.from.clone(),
                            transfer.to.clone(),
                        );
                        ctx.link().send_future(async move {
                            let after =
                                crate::wallet::hands_off::balances_after(&coin, &from, &to).await;
                            Msg::TransferSettled(id, hash, after)
                        });
                    }
                    Ok(false) => {
                        transfer.step = "";
                        transfer.error =
                            Some("The network rejected the transfer. Check the explorer.".into());
                    }
                    Err(e) => {
                        transfer.step = "";
                        transfer.error = Some(e);
                    }
                }
            }
            Msg::TransferSettled(id, hash, result) => {
                let Some(transfer) = self.transfers.get_mut(&id) else {
                    return false;
                };
                transfer.step = "";
                match result {
                    Ok((from_after, to_after)) => {
                        Self::report(
                            ctx,
                            &id,
                            json!({"status":"sent","hash":hash,"from_after":from_after,"to_after":to_after}),
                        );
                    }
                    // The transfer arrived; only the fresh numbers are missing.
                    Err(e) => transfer.error = Some(e),
                }
            }
            Msg::TransferReported(id, result) => {
                // The card follows the room's events; only a refusal is shown here.
                if let (Err(e), Some(transfer)) = (result, self.transfers.get_mut(&id)) {
                    transfer.step = "";
                    transfer.error = Some(e);
                }
            }
            Msg::Mode(v) => {
                self.mode = if causewaybay_shared::text_only_provider(&self.provider) {
                    InputMode::Text
                } else {
                    v
                };
                self.persist(ctx);
            }
            Msg::Agent(v) => {
                self.agent = v;
            }
            Msg::SaveMemo => {
                if self.busy
                    || self.deleting
                    || self.room.is_empty()
                    || (self.prompt.trim().is_empty() && self.attachments.is_empty())
                {
                    return false;
                }
                self.busy = true;
                self.error.clear();
                let room = self.room.clone();
                let version = self.draft_version;
                // Kept text and files become one memo: indexed, organized into the
                // Wiki, and part of this chat's context. No AI runs for a keep.
                let body = json!({"content":self.prompt,"room_id":room,"message_id":guid(),
                    "file_ids":self.attachments.iter().map(|f|f.id.clone()).collect::<Vec<_>>(),
                    "clear_epoch":self.cutoffs.get(&room).copied().unwrap_or(0),"data_epoch":self.data_cutoff});
                ctx.link().send_future(async move {
                    Msg::Sent(room, version, api("POST", "/api/memos", Some(body)).await)
                });
            }
            Msg::CaptureRequested(room, version, result) => {
                if room != self.room || version != self.draft_version {
                    return false;
                }
                self.busy = false;
                match result {
                    Ok(status) => {
                        self.capture = status;
                        self.prompt.clear();
                        self.error.clear();
                    }
                    Err(e) => {
                        self.focus = None;
                        self.error = e;
                    }
                }
            }
            Msg::Send(capture) => {
                if self.busy
                    || self.deleting
                    || (capture && self.multi_capture)
                    || self.room.is_empty()
                    || (capture && !self.capture_ready(ctx))
                    || (!capture && self.prompt.trim().is_empty() && self.attachments.is_empty())
                {
                    return false;
                }
                if !capture && !self.ai_ready(ctx) {
                    // Without an enabled AI provider, Enter still keeps the thought.
                    return Component::update(self, ctx, Msg::SaveMemo);
                }
                self.error.clear();
                let req = self.request(ctx, capture);
                if capture && self.capture.active {
                    if self.capture.capture_queued || !self.attachments.is_empty() {
                        return false;
                    }
                    self.begin_focus(capture);
                    self.busy = true;
                    let room = self.room.clone();
                    let version = self.draft_version;
                    ctx.link().send_future(async move {
                        let result = api(
                            "POST",
                            &format!("/api/rooms/{room}/capture/now"),
                            Some(json!(req)),
                        )
                        .await;
                        Msg::CaptureRequested(room, version, result)
                    });
                } else {
                    self.begin_focus(capture);
                    self.retry = Some(req.clone());
                    self.send(ctx, req);
                }
            }
            Msg::MultiCapture => {
                if self.busy || self.deleting || self.room.is_empty() || !self.capture_ready(ctx) {
                    return false;
                }
                if self.attachments.len() >= MAX_CHAT_ATTACHMENTS {
                    self.error = "Attach up to 8 photos or files per message".into();
                    return true;
                }
                self.multi_capture = true;
                self.busy = true;
                self.retry = None;
                self.error.clear();
                let room = self.room.clone();
                let version = self.draft_version;
                let body = json!({"client_id": (!self.capture_client(ctx).is_empty()).then(||self.capture_client(ctx).to_owned()), "display":ctx.props().settings.display,"clear_epoch":self.cutoffs.get(&room).copied().unwrap_or(0),"data_epoch":self.data_cutoff});
                ctx.link().send_future(async move {
                    let result =
                        api("POST", &format!("/api/rooms/{room}/snapshot"), Some(body)).await;
                    Msg::Snapshot(room, version, result)
                });
            }
            Msg::Snapshot(room, version, result) => {
                if room != self.room || version != self.draft_version {
                    return false;
                }
                self.busy = false;
                match result {
                    Ok(file) => self.attachments.push(file),
                    Err(e) => self.error = e,
                }
            }
            Msg::CancelMulti => {
                if self.busy {
                    return false;
                }
                self.multi_capture = false;
            }
            Msg::Retry => {
                if self.busy {
                    return false;
                }
                if let Some(req) = &self.retry {
                    self.send(ctx, req.clone());
                }
            }
            Msg::Sent(room, version, r) => {
                if room != self.room || version != self.draft_version {
                    return false;
                }
                self.busy = false;
                match r {
                    Ok(_) => {
                        self.scroll = self.focus.is_none();
                        self.advance_focus();
                        self.prompt.clear();
                        self.attachments.clear();
                        self.multi_capture = false;
                        self.retry = None;
                        self.error.clear();
                        ctx.link().send_message(Msg::Tick);
                    }
                    Err(e) => {
                        self.focus = None;
                        self.error = e;
                    }
                }
            }
            Msg::Upload(files) => {
                if self.busy || self.room.is_empty() || files.is_empty() {
                    return false;
                }
                if self.attachments.len() + files.len() > MAX_CHAT_ATTACHMENTS {
                    self.error = "Attach up to 8 photos or files per message".into();
                    return true;
                }
                self.busy = true;
                self.error.clear();
                let room = self.room.clone();
                let version = self.draft_version;
                let clear_epoch = *self.cutoffs.get(&room).unwrap_or(&0);
                let data_epoch = self.data_cutoff;
                let progress_room = room.clone();
                let progress = ctx.link().callback(move |p: crate::uploads::Progress| {
                    Msg::UploadProgress(progress_room.clone(), version, p)
                });
                ctx.link().send_future(async move {
                    let mut results = vec![];
                    for file in files {
                        let result = async {
                            // Videos go up in parallel raw slices (up to 1 GB);
                            // other files keep the single 25 MB request.
                            if crate::uploads::is_video(&file) {
                                let name = file.name();
                                let target = crate::uploads::VideoTarget { room: Some(room.clone()), clear_epoch: Some(clear_epoch), data_epoch: Some(data_epoch), wiki: false };
                                return crate::uploads::upload_video(file.into(), name, target, Some(progress.clone())).await;
                            }
                            if file.size() > 25.0*1024.0*1024.0 { return Err("Uploads must be below 25 MB".into()); }
                            let bytes = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(|_| "Could not read file")?;
                            let data = js_sys::Uint8Array::new(&bytes).to_vec();
                            api("POST", "/api/files", Some(json!({"name":file.name(),"data_base64":base64::engine::general_purpose::STANDARD.encode(data),"room_id":room,"clear_epoch":clear_epoch,"data_epoch":data_epoch}))).await
                        }.await;
                        results.push(result);
                    }
                    Msg::Uploaded(room, version, results)
                });
            }
            Msg::UploadProgress(room, version, progress) => {
                if room != self.room || version != self.draft_version || !self.busy {
                    return false;
                }
                self.upload_progress = Some(progress);
            }
            Msg::Uploaded(room, version, results) => {
                if room != self.room || version != self.draft_version {
                    return false;
                }
                self.busy = false;
                self.upload_progress = None;
                for result in results {
                    match result {
                        Ok(f) => self.attachments.push(f),
                        Err(e) => self.error = e,
                    }
                }
            }
            Msg::Detach(id) => {
                if self.busy {
                    return false;
                }
                self.attachments.retain(|f| f.id != id);
                self.retry = None;
            }
            Msg::Tick => {
                if ctx.props().connection == ConnectionState::Connected {
                    // A login/reconnect can interrupt the initial room request.
                    if ctx.props().thought
                        && self.restore_room
                        && self.room.is_empty()
                        && !self.busy
                    {
                        ctx.link().send_message(Msg::Thought(false));
                    }
                    if self.ai.is_none() {
                        ctx.link()
                            .send_future(async { Msg::Ai(api("GET", "/api/ai", None).await) });
                    }
                }
                if !self.room.is_empty() && ctx.props().connection == ConnectionState::Connected {
                    let room = self.room.clone();
                    ctx.link().send_future(async move {
                        let r = api("GET", &format!("/api/rooms/{room}/capture"), None).await;
                        Msg::Status(room, r)
                    });
                }
                return false;
            }
            Msg::Status(room, r) => {
                if room != self.room {
                    return false;
                }
                match r {
                    Ok(s) => {
                        self.capture_loaded = true;
                        if let Some(e) = &s.error {
                            self.error = e.clone();
                        }
                        self.capture = s;
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Skills(r) => {
                if let Ok(v) = r {
                    self.skills = v["skills"]
                        .as_array()
                        .map(|k| {
                            k.iter()
                                .filter(|k| k["ready"] == true)
                                .filter_map(|k| k["name"].as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                }
            }
            Msg::Ai(r) => {
                self.checking = false;
                match r {
                    Ok(ai) => self.ai = Some(ai),
                    Err(e) => self.error = e,
                }
            }
            Msg::CheckAi => {
                self.checking = true;
                let body = json!({"provider": self.provider, "model": self.model});
                ctx.link().send_future(async move {
                    Msg::Ai(api("POST", "/api/ai/check", Some(body)).await)
                });
            }
            Msg::Error(e) => self.error = e,
        }
        // Request callbacks and prop updates can run in either order. Consult
        // the transport so a stale Connected prop cannot erase a new failure.
        self.reconcile_connection_error(crate::api::connection_state());
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let available = ctx
            .props()
            .clients
            .iter()
            .filter(|c| c.online && c.can_capture)
            .collect::<Vec<_>>();
        let ai_ready = self.ai_ready(ctx);
        // Files dropped anywhere on the page or pasted into the box are staged
        // like Add any file; nothing else is required before Enter.
        let dropped = |files: Option<web_sys::FileList>| -> Vec<web_sys::File> {
            files
                .map(|files| (0..files.length()).filter_map(|i| files.get(i)).collect())
                .unwrap_or_default()
        };
        let ondragover = Callback::from(|e: DragEvent| e.prevent_default());
        let ondrop = ctx.link().batch_callback(move |e: DragEvent| {
            let files = dropped(e.data_transfer().and_then(|d| d.files()));
            if files.is_empty() {
                // Dragged text still lands in the box as the browser intends.
                return None;
            }
            e.prevent_default();
            Some(Msg::Upload(files))
        });
        let onpaste = ctx.link().batch_callback(move |e: Event| {
            use wasm_bindgen::JsCast;
            let files = dropped(
                e.dyn_ref::<web_sys::ClipboardEvent>()
                    .and_then(|c| c.clipboard_data())
                    .and_then(|d| d.files()),
            );
            if files.is_empty() {
                return None;
            }
            e.prevent_default();
            Some(Msg::Upload(files))
        });
        let local_ai = causewaybay_shared::text_only_provider(&self.provider);
        let has_photos = self.messages.iter().any(|m| {
            !m.deleted
                && (m.image_path.is_some()
                    || m.attachments.iter().any(|f| f.mime.starts_with("image/")))
        });
        // On the Capture page the fundamental action is one press: Capture is
        // the primary button and Enter on an empty box presses it.
        let capture_page = ctx.props().thought;
        let enter_captures =
            capture_page && self.prompt.trim().is_empty() && self.attachments.is_empty();
        let companion_busy = self.busy
            || self.creating
            || self.checking
            || self.deleting
            || self
                .messages
                .iter()
                .any(|m| !m.deleted && m.status == "running");
        html! {<section class={classes!("chat-page",capture_page.then_some("is-capture"),(self.settling && (self.creating || !self.room.is_empty())).then_some("is-settling"),has_photos.then_some("has-photos"),self.messages.iter().any(|m|!m.deleted).then_some("has-conversation"))} {ondragover} {ondrop}><div class="page-intro"><div><div class="eyebrow">{icon("spark")}{t("A LITTLE SPACE FOR YOU")}</div><h2>{if ctx.props().thought {t("Hey, what’s on your mind?")}else{t("Keep the thought going.")}}</h2><p>{if ctx.props().thought {t("Big ideas, tiny questions, or just a little help with your day.")}else{t("Pick up an idea, or start something new.")}}</p></div><button class="secondary-button" disabled={self.creating || self.deleting} onclick={ctx.link().callback(|_|Msg::New)}>{icon("plus")}{t("New chat")}</button><div class="quick-picks" role="group" aria-label={t("Capture shortcuts")}>{self.device_picker(ctx)}<crate::ai_settings::AiPicker language={ctx.props().language} owner={ctx.props().owner.clone()} settings={ctx.props().settings.clone()} prefix="Quick" compact=true disabled={self.busy||self.deleting} changed={ctx.link().callback(|(p,m)|Msg::AiChosen(p,m))} ai_saved={ctx.props().ai_saved.clone()}/></div><invisible-companion data-state={if companion_busy {"thinking"} else if !self.prompt.trim().is_empty() {"listening"} else {"idle"}}><div class="companion-world" aria-hidden="true"/><div class="companion-halo" aria-hidden="true"/><button type="button" class="companion-touch" aria-label={t("Say hello to your companion")}><img class="companion-figure" src="/assets/everyday-companion.png" alt="" width="1280" height="1280"/></button><span class="companion-spark" aria-hidden="true">{"✦"}</span><span class="companion-state" role="status"><i/>{if companion_busy {t("Working…")} else if !self.prompt.trim().is_empty() {t("I’m all ears")} else {t("Ready when you are")}}</span></invisible-companion></div>
            if ctx.props().thought {<crate::invisible::Workspace language={ctx.props().language} settings={ctx.props().settings.clone()} client={self.capture_client(ctx).to_owned()} room={self.room.clone()} choose_room={ctx.link().callback(Msg::UseSession)} clients={ctx.props().clients.clone()} saved={ctx.props().ai_saved.clone()} setup={ctx.props().backend_settings.clone()} navigate={ctx.props().navigate.clone()} admin={ctx.props().admin}/>}
            if self.capture.active{<div class="live-capture-banner" role="status"><span><i/>{t("Continuous capture is running")}</span><a class="text-button" href="/autonomous">{t("Manage in Autonomous AI")}{icon("arrow")}</a></div>}
            if !ctx.props().settings.demo && !ai_ready {
                <aside class="chat-setup" aria-label={t("AI setup")}><strong>{t("Choose an AI to get started.")}</strong>
                    <p>if ctx.props().admin {<button class="text-button" onclick={ctx.props().backend_settings.reform(|_|())}>{t("Set up AI")}</button>}else{<span>{t(" Ask your admin to enable one, then choose it in AI & tools.")}</span>}</p>
                    <p class="small-muted">{t("Everything you type or drop here is still kept, indexed, and organized into your Wiki.")}</p>
                </aside>
            }
            if !self.error.is_empty(){<div class="error-banner" role="alert">{t(&self.error)}if self.retry.is_some(){<button class="text-button" onclick={ctx.link().callback(|_|Msg::Retry)}>{t("Retry message")}</button>}</div>}
            if self.room.is_empty() && !self.creating {{self.toolbar(ctx)}{self.toolbar_prompts(ctx)}<div class="empty-state">{icon("spark")}<h3>{t("A fresh place to think.")}</h3><p>{t("Start a chat to ask questions or capture your screen.")}</p><button class="primary-button" disabled={self.creating || self.deleting} onclick={ctx.link().callback(|_|Msg::New)}>{t("Start a conversation")}</button></div>}
            else {
                <div class="chat-messages" aria-label={t("Chat messages")}>if self.messages.iter().all(|m|m.deleted){<div class="chat-placeholder"><p>{t("Let’s make today a little easier.")}</p><span>{t("Ask a question, share a photo, or bring a half-formed idea.")}</span></div>}{for self.messages.iter().filter(|m|!m.deleted).map(|m|self.message(ctx,m))}<div ref={self.bottom.clone()}/></div>
                <details class="conversation-options"><summary>{icon("settings")}{t("Conversation options")}</summary>
                {self.toolbar(ctx)}
                {self.toolbar_prompts(ctx)}
                <div class="chat-compose"><div class="chat-ai-row"><div class="capture-reading-mode"><span>{t("How to read the capture")}</span><div class="mode-switch"><button class={classes!((self.mode==InputMode::Text).then_some("selected"))} aria-pressed={(self.mode==InputMode::Text).to_string()} onclick={ctx.link().callback(|_|Msg::Mode(InputMode::Text))}>{icon("file")}{t("Text mode")}</button><button class={classes!((self.mode==InputMode::Image).then_some("selected"))} aria-pressed={(self.mode==InputMode::Image).to_string()} disabled={local_ai} aria-describedby={local_ai.then_some("local-ocr-hint")} onclick={ctx.link().callback(|_|Msg::Mode(InputMode::Image))}>{icon("screen")}{t("Image mode")}</button></div>if local_ai {<span id="local-ocr-hint">{t("Local AI reads captured text with OCR.")}</span>}</div></div>
                <details class="chat-options"><summary>{t("AI & tools")}</summary>if let Some(ai)=&self.ai{<div class="ai-server" aria-label={t("AI server")}><strong>{format!("{} · {}",self.provider,self.model)}</strong><span>{if ai.demo{t("Demo responses")}else if matches!(self.provider.as_str(),"apple"|"mlx"){t("Local processing on your backend")}else{t("Cloud processing · keys managed by server")}}</span>if !ai.status.is_empty(){<span>{t(&ai.status)}</span>}if ctx.props().admin{<button class="text-button" disabled={self.checking} onclick={ctx.link().callback(|_|Msg::CheckAi)}>{if self.checking{t("Checking…")}else{t("Check AI connection")}}</button>}</div>}<crate::ai_settings::AiPicker language={ctx.props().language} owner={ctx.props().owner.clone()} settings={ctx.props().settings.clone()} prefix="Chat" disabled={self.busy||self.deleting} changed={ctx.link().callback(|(p,m)|Msg::AiChosen(p,m))} ai_saved={ctx.props().ai_saved.clone()}/>if !local_ai && !self.skills.is_empty(){<p class="small-muted chat-skills-available" data-testid="chat-skills-available">{icon("spark")}{tf("{count} of your skills can be used in this chat: {names}", &[("count",self.skills.len().to_string()),("names",self.skills.iter().take(6).cloned().collect::<Vec<_>>().join(", "))])}</p>}<label class="chat-agent"><input type="checkbox" disabled={local_ai} checked={self.agent && !local_ai} onchange={ctx.link().callback(|e:Event|Msg::Agent(e.target_unchecked_into::<HtmlInputElement>().checked()))}/>{t("Allow file creation & compilation")}</label>if local_ai {<p class="small-muted" data-testid="chat-tools-unavailable">{t("This AI cannot run skills or create files in Chat. Run skills from Skills, or use Coding to build programs.")}</p>}</details>

                <p class="small-muted chat-context-hint" id="chat-context-hint">{t("Anything you keep here is indexed and organized into your Wiki, and later questions in this chat use it as context.")}</p>
                if self.attachments.iter().any(|f|attachment_kind(f)=="Metadata only") {<p class="small-muted chat-context-hint">{t("Files marked Metadata only include their name, size, and file signature; their contents cannot be decoded.")}</p>}
                <div class="chat-extra-actions"><label class="secondary-button upload-label"><input type="file" multiple=true disabled={self.deleting || self.busy || self.attachments.len() >= MAX_CHAT_ATTACHMENTS} aria-label={t("Attach to chat")} aria-describedby="chat-context-hint" title={t("Any file · up to 8 files, 25 MB each")} onchange={ctx.link().batch_callback(|e:Event|{let input=e.target_unchecked_into::<HtmlInputElement>();let files=input.files().map(|files|(0..files.length()).filter_map(|i|files.get(i)).collect::<Vec<_>>()).unwrap_or_default();input.set_value("");Some(Msg::Upload(files))})}/>{icon("plus")}{t("Add any file")}</label><button class="secondary-button" title={t("Save without a chat reply; your selected AI may tidy and tag the note")} disabled={self.room.is_empty()||self.busy||self.deleting||(self.prompt.trim().is_empty()&&self.attachments.is_empty())} onclick={ctx.link().callback(|_|Msg::SaveMemo)}>{icon("file")}{t("Save memo")}</button><button class="secondary-button" aria-pressed={self.multi_capture.to_string()} disabled={self.room.is_empty()||self.busy||self.deleting||self.capture.active||!self.capture_ready(ctx)||self.attachments.len()>=MAX_CHAT_ATTACHMENTS} onclick={ctx.link().callback(|_|Msg::MultiCapture)}>{icon("screen")}{t("Multi Capture")}</button></div>
                </div></details>
            }
                <div class="chat-dock">
                <div class="chat-attachments">
                {self.upload_bar()}
                {for self.attachments.iter().map(|f| {
                    let id=f.id.clone();
                    let audio=f.mime.starts_with("audio/");
                    let image=f.mime.starts_with("image/");
                    let kind=attachment_kind(f);
                    html!{<div class={classes!("chat-attachment",audio.then_some("has-audio"))} key={f.id.clone()}>
                        if image {<a href={format!("/api/files/{}/image",f.id)} target="_blank" rel="noopener"><img decoding="async" loading="lazy" src={format!("/api/files/{}/thumbnail",f.id)} alt={f.name.clone()}/></a>}
                        else {<span class="attachment-icon">{icon(if audio {"audio"} else {"file"})}</span>}
                        <span class="attachment-details"><strong>{&f.name}</strong><small>{t(kind)}{" · "}{crate::file_size(f.size)}</small></span>
                        <button class="icon-button" disabled={self.busy} aria-label={format!("{}: {}",t("Remove attachment"),f.name)} onclick={ctx.link().callback(move |_|Msg::Detach(id.clone()))}>{icon("close")}</button>
                        if audio {<audio controls=true preload="none" aria-label={tf("Preview {name}", &[("name",f.name.clone())])} src={format!("/api/files/{}/download",f.id)}/>}
                    </div>}
                })}</div>
                if self.multi_capture{<p class="capture-draft-status" role="status">{t("Keep clicking Multi Capture to add screenshots. Send message for one response.")}<button class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::CancelMulti)}>{t("Cancel")}</button></p>}
                if !ctx.props().settings.demo && available.is_empty() {<p class="capture-action-hint capture-setup-hint" id="capture-action-hint" aria-label={t("Capture setup")}>{icon("screen")}{t("Connect a computer to capture its screen.")}<button class="text-button" onclick={ctx.props().navigate.reform(|_|"devices".into())}>{t("Connect a device")}{icon("arrow")}</button></p>}
                else {<p class="capture-action-hint" id="capture-action-hint">{icon("screen")}{t("Capture saves a screenshot to this chat.")}if capture_page {{" "}{t("Enter on an empty message captures too.")}}</p>}
                if !ai_ready && !ctx.props().settings.demo {<p class="small-muted compose-keep-hint" role="status">{icon("file")}{t("No AI provider is enabled. Enter keeps the thought without asking AI.")}</p>}
                <div class="chat-compose-actions" role="group" aria-label={t("Message toolbar")}>
                    <button class={classes!(if capture_page {"primary-button"} else {"secondary-button"},"dock-capture")} aria-label={t("Capture")} aria-describedby="capture-action-hint" disabled={self.room.is_empty()||self.busy||self.deleting||self.multi_capture||!ai_ready||!self.capture_ready(ctx)||(self.capture.active&&(self.capture.capture_queued||!self.attachments.is_empty()))} onclick={ctx.link().callback(|_|Msg::Send(true))}>{icon("screen")}<span class="dock-label">{t("Capture")}</span></button>
                    <textarea aria-label={t("Chat message")} onkeydown={ctx.link().batch_callback(move |e:KeyboardEvent|if e.key()=="Enter"&&!e.shift_key()&&!e.is_composing(){e.prevent_default();Some(Msg::Send(enter_captures))}else{None})} disabled={self.room.is_empty() || self.deleting || (self.busy && self.retry.is_some())} placeholder={t("Type, paste, or drop anything…")} value={self.prompt.clone()} {onpaste} oninput={ctx.link().callback(|e:InputEvent|Msg::Prompt(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/>

                    <button class={classes!(if capture_page {"secondary-button"} else {"primary-button"},"dock-send")} aria-label={if ai_ready {t("Send message")} else {t("Keep in chat")}} disabled={self.room.is_empty()||self.busy||self.deleting||(self.prompt.trim().is_empty()&&self.attachments.is_empty())} onclick={ctx.link().callback(|_|Msg::Send(false))}><span class="dock-label">{if ai_ready {t("Send message")} else {t("Keep in chat")}}</span>{icon("arrow")}</button>
                </div></div>
        </section>}
    }
}
impl Chat {
    /// A capture request is on its way: follow it from the newest message on.
    /// A typed message is followed the usual way, from the bottom.
    fn begin_focus(&mut self, capture: bool) {
        self.focus = capture.then(|| CaptureFocus {
            after: self.messages.iter().map(|m| m.sequence).max().unwrap_or(0),
            capture: Stage::Waiting,
            reply: Stage::Waiting,
        });
    }
    /// Ease the capture post into view when it appears and again once its
    /// screenshot is in, then the reply when its first words arrive and again
    /// when it is complete. An error counts as the final state.
    fn advance_focus(&mut self) {
        let Some(focus) = &mut self.focus else {
            return;
        };
        let Some(post) = self
            .messages
            .iter()
            .find(|m| m.role == "user" && m.sequence > focus.after)
        else {
            return;
        };
        if post.deleted {
            // An unchanged screenshot was discarded: nothing to show.
            self.focus = None;
            return;
        }
        let post_stage = if post.image_path.is_some() || post.status == "error" {
            Stage::Done
        } else {
            Stage::Started
        };
        let reply = self
            .messages
            .iter()
            .find(|m| !m.deleted && m.role != "user" && m.memo_id == post.memo_id);
        let reply_stage = match reply {
            Some(r) if r.status != "running" => Stage::Done,
            Some(r) if !r.text.is_empty() => Stage::Started,
            _ => Stage::Waiting,
        };
        if reply_stage > focus.reply {
            let settle = focus.reply == Stage::Started;
            focus.reply = reply_stage;
            focus.capture = Stage::Done;
            self.focus_to = reply.map(|r| (r.messageid.clone(), settle));
        } else if post_stage > focus.capture {
            let settle = focus.capture == Stage::Started;
            focus.capture = post_stage;
            self.focus_to = Some((post.messageid.clone(), settle));
        }
        if focus.reply == Stage::Done {
            self.focus = None;
        }
    }
    fn reconcile_connection_error(&mut self, connection: ConnectionState) {
        if connection == ConnectionState::Connected && crate::api::is_connection_error(&self.error)
        {
            if self.retry.is_some() {
                // Recovery does not confirm delivery. Keep the original request
                // and its message ID available for an explicit, deduplicated retry.
                self.error = "Message delivery could not be confirmed. Please retry.".into();
            } else {
                self.error.clear();
            }
        }
    }
    fn begin_room(&mut self) -> u64 {
        // Invalidate pending snapshots, uploads, submissions and room creation
        // before the network responds, so nothing can leak into the new draft.
        self.stream = None;
        self.room_version += 1;
        self.rooms_version += 1;
        self.room.clear();
        self.messages.clear();
        self.reset_draft();
        self.error.clear();
        self.editing = None;
        self.removing_message = None;
        self.rename = None;
        self.removing_room = false;
        self.busy = true;
        self.creating = true;
        // A failed explicit reset must never silently reopen the old context.
        self.restore_room = false;
        self.room_version
    }
    fn reset_draft(&mut self) {
        self.prompt.clear();
        self.attachments.clear();
        self.multi_capture = false;
        self.retry = None;
        self.busy = false;
        self.draft_version += 1;
        self.capture = ContinuousStatus::default();
    }
    fn ai_ready(&self, ctx: &Context<Self>) -> bool {
        ctx.props()
            .settings
            .providers
            .iter()
            .any(|p| p.id == self.provider && (p.configured || ctx.props().settings.demo))
    }
    fn capture_ready(&self, ctx: &Context<Self>) -> bool {
        ctx.props().connection == ConnectionState::Connected
            && self.capture_loaded
            && (ctx
                .props()
                .clients
                .iter()
                .any(|c| c.id == self.capture_client(ctx) && c.online && c.can_capture)
                || (ctx.props().settings.demo && self.capture_client(ctx).is_empty()))
    }
    fn capture_client<'a>(&'a self, ctx: &'a Context<Self>) -> &'a str {
        self.capture
            .client_ids
            .first()
            .map(String::as_str)
            .unwrap_or(&ctx.props().devices.active)
    }
    /// Room choice and room-wide actions: shown above the empty state and,
    /// once a conversation is open, just above the composer.
    /// Rename, clear/delete room and delete-all confirmations, shown right under
    /// the toolbar that opened them (they used to sit above the message list,
    /// off screen when the conversation is long).
    fn toolbar_prompts(&self, ctx: &Context<Self>) -> Html {
        html! {<>
            if let Some(title)=&self.rename{<div class="chat-inline-edit"><label>{t("Chatroom name")}<input aria-label={t("Chatroom name")} value={title.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::RoomTitle(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label><button class="primary-button" onclick={ctx.link().callback(|_|Msg::SaveRoom)}>{t("Save name")}</button><button class="text-button" onclick={ctx.link().callback(|_|Msg::CancelEdit)}>{t("Cancel")}</button></div>}
            if self.removing_room{<div class="error-banner"><span>{if self.clearing_room{t("Permanently clear all messages, captures, files, and results in this chatroom?")}else{t("Permanently delete this chatroom and all its messages, captures, files, and results?")}}</span><button class="danger-button" onclick={ctx.link().callback(|_|Msg::ConfirmRoom)}>{if self.clearing_room{t("Confirm clear chatroom")}else{t("Confirm delete chatroom")}}</button><button class="text-button" onclick={ctx.link().callback(|_|Msg::CancelEdit)}>{t("Cancel")}</button></div>}
        </>}
    }
    fn toolbar(&self, ctx: &Context<Self>) -> Html {
        html! {<div class="chat-toolbar"><select aria-label={t("Chat room")} value={self.room.clone()} onchange={ctx.link().callback(|e:Event|Msg::ChooseRoom(e.target_unchecked_into::<HtmlSelectElement>().value()))}><option value="" disabled=true selected={self.room.is_empty()}>{t("Choose a conversation")}</option>{for self.rooms.iter().map(|r|html!{<option value={r.roomid.clone()} selected={r.roomid==self.room}>{&r.title}</option>})}</select>
                if !self.room.is_empty(){<button class="text-button" aria-label={t("Rename chatroom")} onclick={ctx.link().callback(|_|Msg::Rename)}>{t("Rename")}</button><button class="text-button" aria-label={t("Clear chatroom")} onclick={ctx.link().callback(|_|Msg::ClearRoom)}>{t("Clear")}</button><button class="text-button danger-text" aria-label={t("Delete chatroom")} onclick={ctx.link().callback(|_|Msg::RemoveRoom)}>{t("Delete")}</button>}
                <button class="text-button" aria-label={t("Copy all text")} disabled={self.messages.iter().all(|m|m.deleted)} onclick={ctx.props().copy.reform({let text=self.messages.iter().filter(|m|!m.deleted).map(message_text).filter(|s|!s.is_empty()).collect::<Vec<_>>().join("\n\n");move |_|text.clone()})}>{icon("copy")}{t("Copy all")}</button>
        </div>}
    }
    /// The capture computer, always on screen beside the companion: one tap
    /// to change it, kept on the account so the next login starts with it.
    fn device_picker(&self, ctx: &Context<Self>) -> Html {
        let p = ctx.props();
        let selected = self.capture_client(ctx).to_owned();
        let current = p.clients.iter().find(|c| c.id == selected && c.can_capture);
        let unavailable = !selected.is_empty() && !current.is_some_and(|c| c.online);
        let locked = !self.capture.client_ids.is_empty();
        html! {<div class={classes!("quick-pick-group",unavailable.then_some("is-offline"))}><label class="quick-pick quick-device" for="active-capture-device" title={t(if locked {"This session keeps the same capture computer."} else {"Choose one capture computer for this session."})}>{icon("screen")}<span>{t("Device")}</span>
            <select id="active-capture-device" aria-label={t("Active capture device")} value={selected.clone()} disabled={!self.capture_loaded||locked}
                onchange={p.choose_device.reform(|e:Event|e.target_unchecked_into::<HtmlSelectElement>().value())}>
                if selected.is_empty() {<option value="" selected=true disabled=true>{if p.settings.demo {t("Demo capture")}else{t("Choose a device")}}</option>}
                if !selected.is_empty() && current.is_none() {<option value={selected.clone()} selected=true disabled=true>{t("Saved device unavailable")}</option>}
                {for p.clients.iter().filter(|c|c.can_capture).map(|c|html!{<option value={c.id.clone()} selected={c.id==selected} disabled={!c.online}>
                    {if c.online {c.name.clone()} else {format!("{} · {}",c.name,t("Offline"))}}
                </option>})}
            </select></label>
            if unavailable {<p class="device-unavailable" role="status">{t("Selected device is offline or unavailable. Reconnect it or choose another device.")}</p>}
        </div>}
    }
    /// The video upload bar: checking reads the video on this device to
    /// fingerprint it, uploading sends it.
    fn upload_bar(&self) -> Html {
        let Some(progress) = self.upload_progress else {
            return Html::default();
        };
        let label = t(if progress.phase == crate::uploads::Phase::Checksum {
            "Checking video"
        } else {
            "Uploading video"
        });
        let percent = (progress.fraction() * 100.0).round() as u32;
        html! {<div class="chat-upload-progress" role="progressbar" aria-label={label.clone()} aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent.to_string()}><span style={format!("width:{percent}%")}></span><em>{format!("{label} {percent}%")}</em></div>}
    }
    fn persist(&mut self, ctx: &Context<Self>) {
        let value = json!({"provider":self.provider,"models":self.models,"input_mode":self.mode});
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            && storage
                .set_item(
                    &format!("causewaybay.chat.{}", ctx.props().owner),
                    &value.to_string(),
                )
                .is_err()
        {
            self.error =
                "Browser storage is unavailable; this selection lasts for this session.".into();
        }
    }
    /// Send every transfer the AI prepared for a message this tab posted, once:
    /// the key must be this browser's, and the request must still be pending.
    /// Anything else is left for the Wallet page.
    fn start_transfers(&mut self, ctx: &Context<Self>) {
        let owner = ctx.props().owner.clone();
        if !owner.starts_with("0x")
            || crate::account::saved_address().as_deref() != Some(owner.as_str())
        {
            return;
        }
        let mine: Vec<String> = self
            .messages
            .iter()
            .filter(|u| u.role == "user" && self.posted.contains(&u.messageid))
            .map(|u| u.memo_id.clone())
            .collect();
        let fresh: Vec<PendingTransfer> = self
            .messages
            .iter()
            .filter(|m| m.role != "user" && mine.contains(&m.memo_id))
            .filter_map(|m| m.wallet_request.clone())
            .filter(|r| {
                r.status == "pending" && r.from == owner && !self.transfers.contains_key(&r.id)
            })
            .collect();
        for r in fresh {
            let Some(coin) = registry::coin(&r.coin) else {
                continue;
            };
            self.transfers.insert(
                r.id.clone(),
                Transfer {
                    coin: coin.clone(),
                    from: r.from.clone(),
                    to: r.to.clone(),
                    step: "Reading balances…",
                    error: None,
                },
            );
            let id = r.id.clone();
            ctx.link().send_future(async move {
                Msg::TransferPrepared(
                    id,
                    crate::wallet::hands_off::prepare(&coin, &r.from, &r.to, &r.amount).await,
                )
            });
        }
    }
    /// Tell the backend how a hands-off transfer is going; the card follows.
    fn report(ctx: &Context<Self>, id: &str, body: Value) {
        let url = format!("/api/wallet/requests/{id}");
        let id = id.to_string();
        ctx.link().send_future(async move {
            Msg::TransferReported(id, api("PATCH", &url, Some(body)).await)
        });
    }
    /// The card for a transfer the AI prepared: who sends what to whom, both
    /// balances before and after once read, the transaction, and what is
    /// happening now. Confirmed in Wallet only when this tab could not send it.
    fn transfer_card(&self, ctx: &Context<Self>, r: &PendingTransfer) -> Html {
        let net = registry::net_of(&r.network);
        let local = self.transfers.get(&r.id);
        let running = local.is_some_and(|x| !x.step.is_empty() && x.error.is_none());
        let error = local.and_then(|x| x.error.clone());
        let balance = |v: &str| {
            if v.is_empty() {
                "…".to_string()
            } else {
                format!("{v} {}", r.symbol)
            }
        };
        let line = match r.status.as_str() {
            "sent" if !r.from_after.is_empty() => t("Arrived"),
            "sent" if running => t(local.map(|x| x.step).unwrap_or("Waiting for the network…")),
            "sent" => t("Sent from your wallet"),
            "cancelled" => t("Cancelled in your wallet"),
            _ if running => t(local.map(|x| x.step).unwrap_or("Sending…")),
            _ if error.is_some() => t("Not sent from this browser; finish it under Wallet."),
            _ => t("Waiting for your confirmation in Wallet"),
        };
        html! {<div class={classes!("chat-transfer",format!("is-{}",r.status),running.then_some("is-running"))} data-testid="chat-transfer" data-status={r.status.clone()} data-step={local.map(|x| x.step).unwrap_or_default()} role="note" aria-busy={running.to_string()}>
            {icon("wallet")}
            <div class="chat-transfer-text"><strong>{&r.amount}{" "}{&r.symbol}</strong><span>{t("to")}{" "}<code title={r.to.clone()}>{crate::wallet::short(&r.to)}</code>{" · "}{net.map(|n|n.name).unwrap_or_default()}</span>
                if !r.from_before.is_empty() {<dl class="chat-transfer-balances" data-testid="transfer-balances">
                    <div><dt>{t("From")}</dt><dd><code title={r.from.clone()}>{crate::wallet::short(&r.from)}</code><span data-testid="from-before">{balance(&r.from_before)}</span>{" → "}<span data-testid="from-after">{balance(&r.from_after)}</span></dd></div>
                    <div><dt>{t("To")}</dt><dd><code title={r.to.clone()}>{crate::wallet::short(&r.to)}</code><span data-testid="to-before">{balance(&r.to_before)}</span>{" → "}<span data-testid="to-after">{balance(&r.to_after)}</span></dd></div>
                </dl>}
                if !r.hash.is_empty() {<span class="chat-transfer-hash">{t("Transaction")}{" "}<a data-testid="transfer-hash" href={net.map(|n|n.tx_url(&r.hash)).unwrap_or_default()} target="_blank" rel="noopener noreferrer"><code title={r.hash.clone()}>{crate::wallet::short(&r.hash)}</code></a></span>}
                <small data-testid="transfer-state">{line}</small>
                if let Some(e)=&error {<p class="login-error" data-testid="transfer-error">{t(e)}</p>}</div>
            if r.status=="pending" && !running {<button type="button" class="secondary-button" data-testid="review-in-wallet" onclick={ctx.props().navigate.reform(|_|"wallet".to_string())}>{t("Review in Wallet")}{icon("arrow")}</button>}
            else if r.status=="sent" && !r.hash.is_empty() {<a class="text-button" href={net.map(|n|n.tx_url(&r.hash)).unwrap_or_default()} target="_blank" rel="noopener noreferrer">{t("Explorer")}{icon("arrow")}</a>}
        </div>}
    }
    fn merge(&mut self, m: ChatMessage) {
        if m.roomid != self.room
            || m.messagesyncid
                <= self
                    .data_cutoff
                    .max(*self.cutoffs.get(&m.roomid).unwrap_or(&0))
        {
            return;
        }
        if let Some(old) = self
            .messages
            .iter_mut()
            .find(|old| old.messageid == m.messageid)
        {
            if m.messagesyncid > old.messagesyncid {
                let finished = old.status != "complete" && m.status == "complete";
                *old = m.clone();
                // Only a reply finishing now celebrates, never one loaded later.
                if finished && self.solved(&m).is_some() {
                    let streak = self.celebrations.len() + 1;
                    self.celebrations.insert(m.messageid, streak);
                }
            }
        } else {
            self.messages.push(m);
        }
        self.messages.sort_by_key(|m| m.sequence);
        self.advance_focus();
    }
    fn load_rooms(&mut self, ctx: &Context<Self>) {
        self.rooms_version += 1;
        let version = self.rooms_version;
        ctx.link()
            .send_future(async move { Msg::Rooms(version, api("GET", "/api/rooms", None).await) });
    }
    fn load_current_thought(&mut self, ctx: &Context<Self>) {
        if !ctx.props().thought || !self.restore_room || self.creating {
            return;
        }
        self.thought_sync_version += 1;
        let version = self.thought_sync_version;
        let room_version = self.room_version;
        ctx.link().send_future(async move {
            Msg::CurrentThought(
                version,
                room_version,
                api("GET", "/api/rooms/thought", None).await,
            )
        });
    }
    fn request(&self, ctx: &Context<Self>, capture: bool) -> ThinkRequest {
        ThinkRequest {
            prompt: self.prompt.clone(),
            capture,
            agent: self.agent && !causewaybay_shared::text_only_provider(&self.provider),
            provider: Some(self.provider.clone()),
            model: Some(self.model.clone()),
            input_mode: self.mode,
            client_id: (capture && !self.capture_client(ctx).is_empty())
                .then(|| self.capture_client(ctx).to_owned()),
            file_ids: self.attachments.iter().map(|f| f.id.clone()).collect(),
            room_id: Some(self.room.clone()),
            message_id: Some(guid()),
            clear_epoch: Some(*self.cutoffs.get(&self.room).unwrap_or(&0)),
            data_epoch: Some(self.data_cutoff),
            display: Some(ctx.props().settings.display),
            capture_mode: Some(ctx.props().settings.capture_mode.clone()),
            ..Default::default()
        }
    }
    fn send(&mut self, ctx: &Context<Self>, req: ThinkRequest) {
        self.busy = true;
        if let Some(id) = &req.message_id {
            self.posted.insert(id.clone());
        }
        let room = req.room_id.clone().unwrap_or_default();
        let version = self.draft_version;
        ctx.link().send_future(async move {
            Msg::Sent(
                room,
                version,
                api("POST", "/api/think", Some(json!(req))).await,
            )
        });
    }
    /// The captured problem a finished reply got right, checked against the
    /// screen text of the capture it answers.
    fn solved(&self, m: &ChatMessage) -> Option<solved::Solved> {
        if m.role == "user" || m.status != "complete" || m.text.is_empty() {
            return None;
        }
        let capture = self
            .messages
            .iter()
            .find(|u| u.role == "user" && u.memo_id == m.memo_id && !u.ocr_text.is_empty())?;
        solved::check(&capture.ocr_text, &m.text)
    }
    fn message(&self, ctx: &Context<Self>, m: &ChatMessage) -> Html {
        let user = m.role == "user";
        let solved = self.solved(m);
        let celebrate = self.celebrations.get(&m.messageid).map(|n| n.to_string());
        let failed_without_reply = !user && m.status == "error" && m.text.is_empty();
        let edit = m.clone();
        let remove = m.clone();
        let stop = m.memo_id.clone();
        let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(&m.created_at));
        let time = format!("{:02}:{:02}", date.get_hours(), date.get_minutes());
        // Keyed so a message keeps its DOM node when the list grows: the focus
        // glow, image state and enter motion all belong to that one message.
        html! {<article key={m.messageid.clone()} class={classes!("chat-message",if user{"from-user"}else{"from-ai"},self.glow.as_deref().is_some_and(|g|g==m.messageid).then_some("is-focused"))} data-message-id={m.messageid.clone()} data-status={m.status.clone()} data-sync-id={m.messagesyncid.to_string()} data-celebrate={celebrate.clone()} data-celebrate-title={celebrate.is_some().then(||t("Solved it!"))} data-celebrate-streak={celebrate.is_some().then(||t("Streak"))} data-celebrate-detail={celebrate.and(solved.as_ref()).map(|s|format!("{} = {}",s.problem,s.result))}><div class="chat-message-meta"><strong>{if user{t("You")}else if failed_without_reply{t(if m.stage.starts_with("Capturing "){"Capture failed"}else{"Request failed"})}else if m.memo_kind=="note"{t("Tidied note")}else{"Invisible AI".into()}}</strong><span>{if failed_without_reply {t("No AI response")} else if m.provider=="memo" || (user && m.memo_kind=="note") {t("Kept as a note")} else if m.provider=="remote" {t("Mac action")} else if user {m.client_name.clone().unwrap_or_default()} else {format!("{} · {} · {}",t(if matches!(m.provider.as_str(),"apple"|"mlx") {"Local AI"} else {"Cloud LLM"}),m.provider,m.model)}}</span></div>
            if self.editing.as_ref().is_some_and(|e|e.messageid==m.messageid){<div class="chat-edit"><textarea aria-label={t("Edit message")} value={self.edit_text.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::EditText(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/><button class="primary-button" onclick={ctx.link().callback(|_|Msg::SaveEdit)}>{t("Save message")}</button><button class="text-button" onclick={ctx.link().callback(|_|Msg::CancelEdit)}>{t("Cancel")}</button></div>}
            if self.removing_message.as_ref().is_some_and(|e|e.messageid==m.messageid){<div class="chat-edit"><span>{if m.role=="user"{t("Remove this message, its response, and generated results on all your devices?")}else{t("Remove this message and its generated results on all your devices?")}}</span><button class="danger-button" onclick={ctx.link().callback(|_|Msg::ConfirmMessage)}>{t("Confirm remove message")}</button><button class="text-button" onclick={ctx.link().callback(|_|Msg::CancelEdit)}>{t("Cancel")}</button></div>}
            if m.image_path.is_some(){<figure class="chat-image"><a href={format!("/api/memos/{}/image",m.memo_id)} target="_blank" rel="noopener" aria-label={t("Open full screenshot")}><img decoding="async" loading="lazy" src={format!("/api/memos/{}/thumbnail",m.memo_id)} alt={t("Captured screen")}/></a><figcaption>{if m.input_mode==InputMode::Text{t("Screenshot · Text mode")}else{t("Screenshot · Image mode")}}</figcaption></figure>}
            if user && m.text.is_empty() && m.attachments.is_empty(){<p class="chat-user-text">{t("Screen capture")}</p>}
            if !m.text.is_empty(){if user && !m.text.contains("```") && !m.text.contains("~~~"){<p class="chat-user-text">{&m.text}</p>}else{<div class="answer-content">{copyable_markdown(&m.text, ctx.props().copy.clone())}</div>}}
            if let Some(s)=&solved{<p class="chat-solved" data-testid="chat-solved">{icon("spark")}<strong>{t("Solved")}</strong><span>{format!("{} = {}",s.problem,s.result)}</span></p>}
            if !m.sources.is_empty(){<div class="chat-sources" aria-label={t("Web sources")}><strong>{t("Sources")}</strong>{for m.sources.iter().filter(|s|s.url.starts_with("https://")).map(|source|html!{<a href={source.url.clone()} target="_blank" rel="noopener noreferrer">{&source.title}</a>})}</div>}
            // A transfer the AI prepared: sent by this tab when it asked, else confirmed in Wallet.
            if let Some(r)=&m.wallet_request {{self.transfer_card(ctx,r)}}
            if !m.ocr_text.is_empty(){<details class="chat-ocr"><summary>{t("Extracted text · Apple Vision")}</summary><pre>{&m.ocr_text}</pre></details>}
            if !m.transcript.is_empty(){<div class="chat-ocr"><strong>{t("Audio transcript")}</strong><pre>{&m.transcript}</pre></div>}
            {for m.attachments.iter().map(|f|html!{<div class="chat-file" key={f.id.clone()}>if f.mime.starts_with("image/"){<a href={format!("/api/files/{}/image",f.id)} target="_blank" rel="noopener"><img decoding="async" loading="lazy" class="chat-photo" src={format!("/api/files/{}/thumbnail",f.id)} alt={f.name.clone()}/></a>}if f.mime.starts_with("audio/"){<audio controls=true preload="none" src={format!("/api/files/{}/download",f.id)}/>}<a href={format!("/api/files/{}/download",f.id)}>{icon("file")}{&f.name}</a></div>})}
            if m.status=="running"||m.status=="capturing"{{self.waiting(m)}}
            if !user && !failed_without_reply && m.provider!="memo" && m.provider!="remote" && m.status!="running" && m.status!="capturing" {<p class="chat-llm-log" data-testid="chat-llm-log">{icon(if local_model(&m.provider){"shield"}else{"globe"})}<span>{t(if m.memo_kind=="note"{"Tidied by"}else{"Answered by"})}{" "}{t(if local_model(&m.provider){"Local LLM"}else{"Cloud AI"})}{" · "}{&m.provider}{" · "}{&m.model}</span>if let Some(u)=&m.usage {<span class="chat-usage" data-testid="chat-usage">{" · "}{wait_text(u.elapsed_ms)}{" · "}{t("tokens")}{" "}{u.input_tokens.map(|v|v.to_string()).unwrap_or("—".into())}{"/"}{u.output_tokens.map(|v|v.to_string()).unwrap_or("—".into())}{" · "}{if local_model(&m.provider){t("No cloud cost")}else if let Some(usd)=u.estimated_usd {format!("{} ${usd:.4}",t("est."))}else{t("Cost unknown")}}</span>}</p>}
            if !user && !m.skills.is_empty() {<p class="chat-skills" data-testid="chat-skills">{icon("spark")}<span>{t("Skills used")}{": "}</span>{for m.skills.iter().map(|k|html!{<span key={k.name.clone()} class={classes!("chat-skill",(!k.success).then_some("is-failed"))} title={if k.success{t("ran cleanly")}else{t("failed")}}>{&k.name}{if k.success{" ✓"}else{" ✗"}}</span>})}</p>}
            if !user {if let Some(e)=&m.error{<p class="login-error">{t(e)}</p>}}
            if self.wiki_enabled && !m.memo_id.is_empty() {{self.tags(ctx,m)}}
            <div class="message-actions"><time datetime={m.created_at.clone()}>{time}</time>if m.edited_at.is_some(){<span>{t("Edited")}</span>}
            if !user && m.memo_kind=="agent" && !m.memo_id.is_empty() {<a class="text-button" href={format!("/memos/{}",m.memo_id)}>{icon("code")}{t("Workspace activity")}</a>}
            if self.wiki_enabled && !m.memo_id.is_empty() && m.status!="running" && m.status!="capturing" && self.tagging.as_ref().is_none_or(|e|e.messageid!=m.messageid) {
                if m.wiki_state=="skipped" {<span class="chat-tags-pending chat-wiki-skipped" data-testid="chat-wiki-skipped">{icon("book")}{t("Not in Wiki")}</span><button type="button" class="text-button chat-wiki-keep" data-testid="chat-wiki-keep" onclick={ctx.link().callback({let id=m.memo_id.clone();move |_|Msg::WikiKeep(id.clone(),true)})}>{icon("plus")}{t("Keep in Wiki")}</button>}
                else {
                    // Still being analyzed: say so, and still offer to tag by hand
                    // (a list set now applies when the analysis completes, and a
                    // message the analysis never reaches is not stuck).
                    if m.tags.is_empty() && self.wiki_auto {<span class="chat-tags-pending" data-testid="chat-tags-pending">{icon("hash")}{t("Tagging…")}</span>}
                    <button type="button" class="text-button chat-tags-edit" aria-label={t("Edit tags")} onclick={ctx.link().callback({let edit=m.clone();move |_|Msg::EditTags(edit.clone())})}>{icon("hash")}{if m.tags.is_empty(){t("Add tags")}else{t("Edit tags")}}</button>
                }
            }
            if m.status=="running"||m.status=="capturing"{<button class="text-button" onclick={ctx.link().callback(move |_|Msg::StopAnswer(stop.clone()))}>{t("Stop answer")}</button>}
            if user && (m.provider=="memo" || m.memo_kind=="note") && !m.text.trim().is_empty() {<button type="button" class="text-button chat-ask-instead" data-testid="chat-ask-instead" onclick={ctx.link().callback({let text=m.text.clone();move |_|Msg::AskInstead(text.clone())})}>{icon("spark")}{t("Ask AI instead")}</button>}
            <button class="text-button" aria-label={t("Copy message")} onclick={ctx.props().copy.reform({let text=message_text(m);move |_|text.clone()})}>{icon("copy")}{t("Copy")}</button><button class="text-button" aria-label={t("Edit message")} onclick={ctx.link().callback(move |_|Msg::Edit(edit.clone()))}>{t("Edit")}</button><button class="text-button" aria-label={t("Remove message")} onclick={ctx.link().callback(move |_|Msg::RemoveMessage(remove.clone()))}>{t("Remove")}</button></div>
        </article>}
    }
}
impl Chat {
    /// A message's Wiki tags, shared by a post and its generated response: each
    /// one opens that tag in the Wiki; an inline editor (opened from the actions
    /// row) adds and removes them. Tags appear once the on-device analysis of
    /// the post and the response has run.
    fn tags(&self, ctx: &Context<Self>, m: &ChatMessage) -> Html {
        let link = ctx.link();
        if self
            .tagging
            .as_ref()
            .is_some_and(|e| e.messageid == m.messageid)
        {
            let full = self.tag_draft.len() >= MAX_TAGS;
            return html! {<div class="chat-tags is-editing" data-testid="chat-tags-editor" role="group" aria-label={t("Edit tags")}>
                {for m.tags.iter().filter(|t|causewaybay_shared::is_fixed_tag(t)).map(|tag|html!{<span class={classes!("chat-tag",if causewaybay_shared::is_type(tag){"is-type"}else{"is-category"})} key={tag.clone()} title={if causewaybay_shared::is_type(tag){t("Type")}else{t("Category")}}>{"#"}{tag}</span>})}
                {for self.tag_draft.iter().map(|tag|{let remove=tag.clone();html!{<span class="chat-tag" key={tag.clone()}>{"#"}{tag}<button type="button" class="chat-tag-remove" aria-label={tf("Remove tag {tag}", &[("tag",tag.clone())])} onclick={link.callback(move |_|Msg::RemoveTag(remove.clone()))}>{icon("close")}</button></span>}})}
                <input class="chat-tag-input" type="text" aria-label={t("Add tag")} placeholder={if full{t("Up to 12 tags")}else{t("Add tag, Enter")}} disabled={full} value={self.tag_input.clone()} maxlength="40" autocapitalize="none" autocomplete="off" enterkeyhint="done"
                    oninput={link.callback(|e:InputEvent|Msg::TagInput(e.target_unchecked_into::<HtmlInputElement>().value()))}
                    onkeydown={link.batch_callback(|e:KeyboardEvent|{if e.key()=="Enter"{e.prevent_default();Some(Msg::AddTag)}else if e.key()=="Escape"{Some(Msg::CancelTags)}else{None}})}/>
                <button type="button" class="primary-button chat-tags-save" disabled={self.saving_tags} onclick={link.callback(|_|Msg::SaveTags)}>{if self.saving_tags{t("Saving…")}else{t("Save tags")}}</button>
                <button type="button" class="text-button" onclick={link.callback(|_|Msg::CancelTags)}>{t("Cancel")}</button>
                <button type="button" class="text-button danger-text chat-wiki-remove" data-testid="chat-wiki-remove" onclick={link.batch_callback({let id=m.memo_id.clone();move |_|vec![Msg::CancelTags,Msg::WikiKeep(id.clone(),false)]})}>{icon("trash")}{t("Remove from Wiki")}</button>
            </div>};
        }
        // No row until there are tags: an untagged message keeps its shape, and
        // the actions row offers to add some.
        if m.tags.is_empty() {
            return html! {};
        }
        html! {<div class="chat-tags" data-testid="chat-tags">
            {for m.tags.iter().map(|tag|{
                let armed = self.armed_tag.as_ref().is_some_and(|(id, armed)| *id == m.messageid && armed == tag);
                let (messageid, name) = (m.messageid.clone(), tag.clone());
                // One tap (or click) arms the chip; the second lets the link
                // open the Wiki. A keyboard activation (detail 0) opens at once.
                let onclick = link.callback(move |e: MouseEvent| {
                    if armed || e.detail() == 0 {
                        return Msg::DisarmTag;
                    }
                    e.prevent_default();
                    Msg::ArmTag(messageid.clone(), name.clone())
                });
                let title = if armed {t("Tap again to open in Wiki")} else if causewaybay_shared::is_fixed_tag(tag){t("Category · tap twice to open in Wiki")}else{t("Tap twice to open in Wiki")};
                html!{<a class={classes!("chat-tag",causewaybay_shared::is_type(tag).then_some("is-type"),causewaybay_shared::is_category(tag).then_some("is-category"),armed.then_some("is-armed"))} key={tag.clone()} href={format!("/wiki/tag/{}",crate::api::encode(tag))} title={title.clone()} aria-label={format!("#{tag} · {title}")} {onclick}>{"#"}{tag}</a>}
            })}
        </div>}
    }
}
/// How long a tapped tag chip stays armed for the second tap that opens it.
const ARM_MS: u32 = 1500;
fn wait_text(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
fn local_model(provider: &str) -> bool {
    matches!(provider, "apple" | "mlx")
}
impl Chat {
    /// Live progress under a running reply: an animated indicator, the stage, and
    /// once a model is working, which LLM it is, where it runs, and how long we waited.
    fn waiting(&self, m: &ChatMessage) -> Html {
        let stage = if m.stage.is_empty() {
            "Thinking…"
        } else {
            &m.stage
        };
        let ai_phase = m.stage.starts_with("Thinking") || m.stage.starts_with("Working");
        let local = local_model(&m.provider);
        let started = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(&m.created_at)).get_time();
        let waited = ((js_sys::Date::now() - started) / 1000.0).floor().max(0.0) as u64;
        html! {<div class={classes!("chat-waiting",ai_phase.then_some(if local {"is-local"} else {"is-cloud"}))} role="status" aria-live="polite" data-testid="chat-waiting">
            <span class="cloud-pulse" aria-hidden="true"><i/><i/><i/></span>
            <span class="chat-waiting-text">{t(stage)}</span>
            if ai_phase {
                <span class={classes!("ai-location",if local {"is-local"} else {"is-cloud"})}>{icon(if local {"shield"} else {"globe"})}{t(if local {"Local LLM"} else {"Cloud AI"})}</span>
                <span class="cloud-llm">{&m.provider}<small>{" · "}{&m.model}</small></span>
            }
            if !started.is_nan() {<span class="chat-wait-time" data-testid="chat-wait-time">{t("Waited")}{" "}{waited}{"s"}</span>}
            if let Some(p)=&m.progress {{progress_view(p)}}
        </div>}
    }
}
/// Preparation progress pushed by the backend: a measured bar for a download,
/// a sweeping one while the request waits its turn or the model loads.
fn progress_view(p: &causewaybay_shared::ModelProgress) -> Html {
    html! {<div class="chat-progress" data-testid="chat-progress">
        <span class="chat-progress-label">{t(&p.label)}if !p.detail.is_empty(){<small>{" · "}{&p.detail}</small>}</span>
        if let Some(f)=p.fraction {<crate::tween::ProgressBar fraction={f} label={p.label.clone()} detail={p.detail.clone()}/>}
        else {<div class="model-progress"><div class="model-progress-track indeterminate" role="progressbar" aria-label={p.label.clone()}><i/></div></div>}
    </div>}
}
fn linked_room() -> Option<String> {
    let query = web_sys::window()?.location().search().ok()?;
    query.trim_start_matches('?').split('&').find_map(|part| {
        let id = part.strip_prefix("room=")?;
        (id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
            .then(|| id.to_owned())
    })
}
fn message_text(m: &ChatMessage) -> String {
    let mut parts = vec![m.text.clone()];
    if !m.ocr_text.is_empty() {
        parts.push(m.ocr_text.clone());
    }
    if !m.transcript.is_empty() {
        parts.push(m.transcript.clone());
    }
    parts.extend(m.attachments.iter().map(|f| f.name.clone()));
    parts.extend(m.sources.iter().map(|s| format!("{}: {}", s.title, s.url)));
    if let Some(error) = &m.error {
        parts.push(error.clone());
    }
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn attachment_kind(file: &FileInfo) -> &'static str {
    if file.mime.starts_with("audio/") {
        return "Audio → transcript";
    }
    if file.mime.starts_with("image/") {
        return "Image context";
    }
    let extension = file
        .name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "pdf" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp"
    ) || file.mime == "application/pdf"
    {
        "Document → text"
    } else if extension == "zip" || file.mime == "application/zip" {
        "Archive listing"
    } else if file.mime == "application/octet-stream" {
        "Metadata only"
    } else {
        "Text context"
    }
}

fn copyable_markdown(text: &str, copy: Callback<String>) -> Html {
    use pulldown_cmark::{Event, Tag, TagEnd};
    use wasm_bindgen::JsCast;
    let mut codes = Vec::<String>::new();
    let mut in_code = false;
    let mut events = vec![];
    for event in pulldown_cmark::Parser::new_ext(text, pulldown_cmark::Options::all()) {
        match &event {
            Event::Start(Tag::CodeBlock(_)) => {
                events.push(Event::Html(format!("<div class=\"code-block\"><div class=\"code-toolbar\"><button type=\"button\" class=\"text-button\" data-copy-code=\"{}\" aria-label=\"{}\">{}</button></div>", codes.len(),ammonia::clean_text(&t("Copy code")),ammonia::clean_text(&t("Copy"))).into()));
                codes.push(String::new());
                in_code = true;
            }
            Event::Text(value) if in_code => codes.last_mut().unwrap().push_str(value),
            Event::End(TagEnd::CodeBlock) => {
                events.push(event);
                events.push(Event::Html("</div>".into()));
                in_code = false;
                continue;
            }
            _ => {}
        }
        events.push(event);
    }
    let mut output = String::new();
    pulldown_cmark::html::push_html(&mut output, events.into_iter());
    let mut sanitizer = ammonia::Builder::default();
    sanitizer
        .rm_tags(&["img"])
        .add_tags(&["button"])
        .add_generic_attributes(&["class"])
        .add_tag_attributes("button", &["type", "data-copy-code", "aria-label"]);
    let output = Html::from_html_unchecked(AttrValue::from(sanitizer.clean(&output).to_string()));
    html! {<div onclick={Callback::from(move |event:MouseEvent| {
        if let Some(button) = event.target().and_then(|e|e.dyn_into::<web_sys::Element>().ok())
            .and_then(|e|e.closest("button[data-copy-code]").ok().flatten())
            && let Some(index) = button.get_attribute("data-copy-code").and_then(|v|v.parse::<usize>().ok())
            && let Some(code) = codes.get(index) {
                copy.emit(code.clone());
            }
    })}>{output}</div>}
}

/// Fast enough for a hurried check, long enough to follow where it went.
const FOCUS_MS: f64 = 560.0;
/// How long a focused message glows: the scroll, then a soft settle.
const GLOW_MS: u32 = 1500;
pub(crate) fn guid() -> String {
    let mut b = [0u8; 16];
    web_sys::window()
        .unwrap()
        .crypto()
        .unwrap()
        .get_random_values_with_u8_array(&mut b)
        .expect("Browser random source");
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let h = b.iter().map(|v| format!("{v:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}
