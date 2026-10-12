mod account;
mod admin;
mod ai_activity;
mod ai_settings;
mod api;
mod assistance;
mod chat;
mod clipboard;
mod cloud;
mod code;
mod coding;
mod companion;
mod connection;
mod dashboard;
mod devices;
mod i18n;
mod icons;
mod images;
mod invisible;
mod login;
mod wallet;
mod message_cache;
mod motion;
mod ondevice;
mod save_anything;
mod skills;
mod status;
mod tags;
mod theme;
mod tween;
mod uploads;
mod videos;
mod wiki;
use api::{api, encode};
use causewaybay_shared::*;
use gloo_events::EventListener;
use gloo_timers::callback::{Interval, Timeout};
use i18n::{Language, t, tf};
use icons::{brand, icon};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement};
use yew::prelude::*;

fn main() {
    yew::Renderer::<App>::new().render();
}

struct BatchRun {
    item: CaptureBatchItem,
    stage: String,
    cursor: i64,
}
struct App {
    language: Language,
    theme: theme::Theme,
    _language_storage: EventListener,
    settings: Settings,
    rename_id: String,
    node_name: String,
    quiz: quiz::Quiz,
    quiz_answer: bool,
    batch: Vec<BatchRun>,
    capture_ids: Vec<String>,
    browser: Value,
    devices: devices::Choices,
    memos: Vec<Memo>,
    files: Vec<FileInfo>,
    total: usize,
    page: String,
    thought_epoch: u64,
    query: String,
    filter: String,
    prompt: String,
    selected: Option<Memo>,
    attachment: Option<FileInfo>,
    stage: String,
    phase: String,
    events: Vec<StreamEvent>,
    busy: bool,
    loading: bool,
    error: String,
    toast: String,
    modal: String,
    settings_provider: String,
    local_ai_draft: LocalAiSettings,
    /// The backend's own check of its AI setup, shown in Backend settings.
    setup_report: Option<SetupReport>,
    checking_setup: bool,
    default_ai_draft: UserAiPreferences,
    model: String,
    capture_mode: String,
    display: u32,
    note_title: String,
    note_body: String,
    preview: Option<FileInfo>,
    preview_text: String,
    compile_output: String,
    agent: bool,
    auth: bool,
    session: Value,
    token: String,
    mobile_menu: bool,
    delete_confirm: bool,
    delete_file: String,
    stream_id: Option<String>,
    last_event: i64,
    clients: Vec<CaptureClient>,
    client_id: String,
    connection: api::ConnectionState,
    dashboard: Option<Dashboard>,
    booted: bool,
    debounce: Option<Timeout>,
    draft_timer: Option<Timeout>,
    toast_timer: Option<Timeout>,
    search_version: u64,
    selection_version: u64,
    notifications: Option<api::Notifications>,
    notification_cache: Option<futures_channel::mpsc::UnboundedSender<message_cache::Write>>,
    _keyboard: EventListener,
    _history: EventListener,
    _refresh: Interval,
    _dashboard_refresh: Interval,
}
enum Msg {
    Language(Language),
    LanguageStorage,
    ToggleTheme,
    ThemeStorage,
    DeviceStorage(Option<String>),
    CaptureDevice(String),
    DefaultCaptureDevice(String),
    CopyText(String),
    Notification(Value),
    Boot,
    Session(Result<Value, String>),
    Logout,
    Loaded(Result<Settings, String>),
    List(Result<SearchResults, String>, u64),
    Files(u64, Result<Vec<FileInfo>, String>),
    DeleteFile(String),
    /// The right to be forgotten: every record this wallet made, after a confirmation.
    DeleteEverything,
    DeletedEverything(Result<Value, String>),
    FileDeleted(Result<Value, String>),
    ProcessFile,
    Processed(u64, Result<ProcessedFile, String>),
    Navigate(String),
    History,
    Query(String),
    Filter(String),
    Search,
    Prompt(String),
    PersistDraft(String),
    Think(bool),
    BatchStarted(u64, Result<Vec<CaptureBatchItem>, String>),
    BatchEvent(String, StreamEvent),
    CaptureClientToggle(String),
    QuizKind(quiz::Kind),
    NewQuiz(bool),
    RevealQuiz,
    QuizCapture,
    Started(u64, Result<Memo, String>),
    Select(String),
    Selected(u64, Result<Memo, String>),
    Stream(u64, StreamEvent),
    Cancel,
    Clients(Result<Vec<CaptureClient>, String>),
    Client(String),
    Connection(api::ConnectionState),
    DashboardTick,
    DashboardLoaded(Result<Dashboard, String>),
    RenameNode(CaptureClient),
    NodeName(String),
    SaveNodeName,
    NodeRenamed(Result<CaptureClient, String>),
    Modal(String),
    SettingProvider(String),
    /// An on-device model card: its provider becomes the server default and
    /// its repo id the MLX model, in one tap.
    ChooseLocalModel(String, String),
    CheckSetup,
    SetupChecked(Result<SetupReport, String>),
    LocalAi(&'static str, String),
    DefaultMode(&'static str),
    DefaultCloud(String),
    Model(String),
    CaptureMode(String),
    Display(String),
    SaveSettings,
    SettingsSaved(Result<Settings, String>),
    InputMode(String),
    NoteTitle(String),
    NoteBody(String),
    SaveNote,
    SavedNote(u64, Result<Memo, String>),
    Pin,
    DeleteAsk,
    Delete,
    Deleted(Result<Value, String>),
    Upload(web_sys::File),
    Uploaded(u64, Result<FileInfo, String>),
    Detach,
    Preview(FileInfo),
    PreviewText(u64, Result<Value, String>),
    Compile,
    Compiled(u64, Result<Value, String>),
    Toast(String),
    ClearToast,
    ClearError,
    Token(String),
    Login,
    LoggedIn(Result<Value, String>),
    ToggleMenu,
    Copy,
    Shortcut(bool),
    Refresh,
    Refreshed(Result<Settings, String>),
}

impl Component for App {
    type Message = Msg;
    type Properties = ();
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Boot);
        api::watch_connection(ctx.link().callback(Msg::Connection));
        let link = ctx.link().clone();
        let keyboard = EventListener::new(&web_sys::window().unwrap(), "keydown", move |e| {
            if let Some(e) = e.dyn_ref::<KeyboardEvent>() {
                if (e.meta_key() || e.ctrl_key()) && e.key() == "k" {
                    e.prevent_default();
                    link.send_message(Msg::Shortcut(false));
                }
                if (e.meta_key() || e.ctrl_key())
                    && e.key() == "Enter"
                    && e.target()
                        .and_then(|t| t.dyn_into::<HtmlTextAreaElement>().ok())
                        .is_some_and(|t| t.id() == "question")
                {
                    e.prevent_default();
                    link.send_message(Msg::Shortcut(true));
                }
                if e.key() == "Escape" {
                    link.send_message(Msg::Modal(String::new()));
                }
            }
        });
        let link = ctx.link().clone();
        let history = EventListener::new(&web_sys::window().unwrap(), "popstate", move |_| {
            // Views that restore remembered state on mount (the Wiki) check this
            // stamp so Back/Forward show the URL the browser asked for.
            mark_history_traversal();
            link.send_message(Msg::History);
        });
        let link = ctx.link().clone();
        let refresh = Interval::new(15_000, move || link.send_message(Msg::Refresh));
        let link = ctx.link().clone();
        let dashboard_refresh = Interval::new(2000, move || link.send_message(Msg::DashboardTick));
        let theme = theme::load();
        theme::activate(theme);
        let language = i18n::load();
        i18n::activate(language);
        let link = ctx.link().clone();
        let language_storage =
            EventListener::new(&web_sys::window().unwrap(), "storage", move |event| {
                if let Some(event) = event.dyn_ref::<web_sys::StorageEvent>()
                    && let Some(storage) = event.storage_area()
                    && web_sys::window()
                        .and_then(|w| w.local_storage().ok().flatten())
                        .as_ref()
                        == Some(&storage)
                {
                    if event.key().is_none() || event.key().as_deref() == Some(i18n::STORAGE_KEY) {
                        link.send_message(Msg::LanguageStorage);
                    }
                    if event.key().is_none() || event.key().as_deref() == Some(theme::STORAGE_KEY) {
                        link.send_message(Msg::ThemeStorage);
                    }
                    link.send_message(Msg::DeviceStorage(event.key()));
                }
            });
        Self {
            language,
            theme,
            _language_storage: language_storage,
            settings: Settings::default(),
            local_ai_draft: LocalAiSettings::default(),
            setup_report: None,
            checking_setup: false,
            default_ai_draft: UserAiPreferences {
                mode: "cloudai".into(),
                local_provider: "apple".into(),
                cloud_provider: "grok".into(),
            },
            rename_id: String::new(),
            node_name: String::new(),
            quiz: quiz::generate(quiz::Kind::Math, random_seed()),
            quiz_answer: false,
            batch: vec![],
            capture_ids: vec![],
            browser: load_browser("local"),
            devices: devices::load("local"),
            memos: vec![],
            files: vec![],
            total: 0,
            thought_epoch: 0,
            page: location_page(),
            query: String::new(),
            filter: "all".into(),
            prompt: String::new(),
            selected: None,
            attachment: None,
            stage: String::new(),
            phase: "capture".into(),
            events: vec![],
            busy: false,
            loading: true,
            error: String::new(),
            toast: String::new(),
            modal: String::new(),
            settings_provider: "anthropic".into(),
            model: String::new(),
            capture_mode: "screen".into(),
            display: 1,
            note_title: String::new(),
            note_body: String::new(),
            preview: None,
            preview_text: String::new(),
            compile_output: String::new(),
            agent: false,
            auth: true,
            session: Value::Null,
            token: String::new(),
            mobile_menu: false,
            delete_confirm: false,
            delete_file: String::new(),
            stream_id: None,
            last_event: 0,
            clients: vec![],
            client_id: String::new(),
            connection: api::ConnectionState::Connecting,
            dashboard: None,
            booted: false,
            debounce: None,
            draft_timer: None,
            toast_timer: None,
            search_version: 0,
            selection_version: 0,
            notifications: None,
            notification_cache: None,
            _keyboard: keyboard,
            _history: history,
            _refresh: refresh,
            _dashboard_refresh: dashboard_refresh,
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Language(language) => {
                self.language = language;
                i18n::activate(language);
                if !i18n::save(language) {
                    ctx.link().send_message(Msg::Toast(
                        "Browser storage is unavailable; this selection lasts for this session."
                            .into(),
                    ));
                }
            }
            Msg::LanguageStorage => {
                self.language = i18n::load();
                i18n::activate(self.language);
            }
            Msg::ToggleTheme => {
                self.theme = self.theme.toggle();
                theme::activate(self.theme);
                if !theme::save(self.theme) {
                    ctx.link().send_message(Msg::Toast(
                        "Browser storage is unavailable; this selection lasts for this session."
                            .into(),
                    ));
                }
            }
            Msg::ThemeStorage => {
                self.theme = theme::load();
                theme::activate(self.theme);
            }
            Msg::DeviceStorage(key) => {
                let owner = self.session["address"].as_str().unwrap_or("local");
                if key.is_some_and(|key| key != devices::key(owner)) {
                    return false;
                }
                self.devices = devices::load(owner);
                self.devices.initialize(&self.clients);
            }
            Msg::CaptureDevice(id) => {
                if !self
                    .clients
                    .iter()
                    .any(|c| c.id == id && c.online && c.can_capture)
                {
                    return false;
                }
                self.devices.active = id;
                self.persist_devices();
            }
            Msg::DefaultCaptureDevice(id) => {
                if !self.clients.iter().any(|c| c.id == id && c.can_capture) {
                    return false;
                }
                self.devices.default = id.clone();
                self.devices.active = id;
                self.persist_devices();
            }

            Msg::CopyText(text) => {
                let link = ctx.link().clone();
                if clipboard::copy_on_http(&text) {
                    link.send_message(Msg::Toast("Copied".into()));
                } else {
                    spawn_local(async move {
                        let navigator = web_sys::window().unwrap().navigator();
                        let available = js_sys::Reflect::get(&navigator, &"clipboard".into())
                            .is_ok_and(|v| !v.is_undefined() && !v.is_null());
                        let copied = available
                            && wasm_bindgen_futures::JsFuture::from(
                                navigator.clipboard().write_text(&text),
                            )
                            .await
                            .is_ok();
                        link.send_message(Msg::Toast(
                            if copied {
                                "Copied"
                            } else {
                                "Could not copy. Select and copy the text manually."
                            }
                            .into(),
                        ));
                    });
                }
            }
            Msg::Notification(v) => {
                // History replayed on connect still purges caches, but the views
                // loaded a moment ago are newer than it: resetting them (and
                // discarding a memo fetch in flight) would show stale emptiness.
                let replay = v["replay"] == json!(true);
                if v["kind"] == "message.deleted" {
                    if let (Some(cache), Ok(message)) = (
                        &self.notification_cache,
                        serde_json::from_value::<ChatMessage>(v["data"]["message"].clone()),
                    ) {
                        let _ =
                            cache.unbounded_send(message_cache::Write::Message(Box::new(message)));
                    }
                    if replay {
                        return false;
                    }
                    self.selected = None;
                    self.preview = None;
                    self.preview_text.clear();
                    self.events.clear();
                    self.attachment = None;
                    self.compile_output.clear();
                    self.close_stream();
                    self.clear_batch();
                    self.memos.clear();
                    self.files.clear();
                    self.search_version += 1;
                    self.selection_version += 1;
                    self.reload(ctx);
                }

                if v["kind"] == "data.deleted" {
                    let cutoff = v["data"]["messagesyncid"].as_i64().unwrap_or(0);
                    if let Some(cache) = &self.notification_cache {
                        let _ = cache.unbounded_send(message_cache::Write::ClearAll(
                            v["data"]["messagesyncid"].as_i64().unwrap_or(0),
                        ));
                    }
                    let erased = self.browser["erased"]
                        .as_str()
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(0);
                    if replay {
                        if cutoff > erased {
                            self.browser["erased"] = json!(cutoff.to_string());
                            self.persist_browser();
                        }
                        return false;
                    }
                    self.selected = None;
                    self.preview = None;
                    self.preview_text.clear();
                    self.events.clear();
                    self.attachment = None;
                    if cutoff > erased {
                        // The backend keeps the erasure stamp per connection; a
                        // fresh connection starts from the one this deletion created.
                        api::reconnect();
                        self.close_stream();
                        self.clear_batch();
                        self.selection_version += 1;
                        self.search_version += 1;
                        self.memos.clear();
                        self.files.clear();
                        self.total = 0;
                        self.prompt.clear();
                        self.query.clear();
                        self.note_title.clear();
                        self.note_body.clear();
                        self.compile_output.clear();
                        self.draft_timer = None;
                        self.busy = false;
                        self.modal.clear();
                        self.browser["draft"] = json!("");
                        self.browser["erased"] = json!(cutoff.to_string());
                        self.persist_browser();
                    }
                    self.reload(ctx);
                }
                if matches!(v["kind"].as_str(), Some("room.deleted" | "room.cleared"))
                    && let Some(room) = v["data"]["roomid"].as_str()
                {
                    if let Some(cache) = &self.notification_cache {
                        let _ = cache.unbounded_send(message_cache::Write::ClearRoom(
                            room.into(),
                            v["data"]["messagesyncid"].as_i64().unwrap_or(0),
                        ));
                    }
                    if replay {
                        return false;
                    }
                    self.selected = None;
                    self.preview = None;
                    self.preview_text.clear();
                    self.events.clear();
                    self.attachment = None;
                    self.compile_output.clear();
                    self.close_stream();
                    self.clear_batch();
                    self.memos.clear();
                    self.files.clear();
                    self.selection_version += 1;
                    self.search_version += 1;
                    self.reload(ctx);
                }
            }
            Msg::DeleteFile(id) => {
                if self.delete_file != id {
                    self.delete_file = id;
                } else {
                    self.delete_file.clear();
                    ctx.link().send_future(async move {
                        Msg::FileDeleted(api("DELETE", &format!("/api/files/{id}"), None).await)
                    });
                }
            }
            Msg::FileDeleted(r) => match r {
                Ok(_) => {
                    self.attachment = None;
                    self.reload(ctx);
                }
                Err(e) => self.handle_error(e),
            },
            Msg::ProcessFile => {
                if let Some(f) = &self.preview {
                    let id = f.id.clone();
                    let version = self.selection_version;
                    self.preview_text = "Processing on the backend…".into();
                    ctx.link().send_future(async move {
                        Msg::Processed(
                            version,
                            api(
                                "POST",
                                &format!("/api/files/{id}/process"),
                                Some(json!({"input_mode":"text"})),
                            )
                            .await,
                        )
                    });
                }
            }
            Msg::Processed(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                match r {
                    Ok(v) => {
                        self.preview_text = format!(
                            "{}\n\n{}{}",
                            v.processor,
                            v.text,
                            if v.truncated {
                                "\n[Preview truncated]"
                            } else {
                                ""
                            }
                        )
                    }
                    Err(e) => self.preview_text = e,
                }
            }
            Msg::Boot => {
                ctx.link()
                    .send_future(async { Msg::Session(api("GET", "/api/auth/info", None).await) });
                return false;
            }
            Msg::Session(r) => match r {
                Ok(v) => {
                    let allowed = v["authenticated"] == true && v["approved"] == true;
                    let reload = allowed && (!self.booted || self.auth);
                    self.auth = !allowed;
                    if self.session["address"] != v["address"] {
                        // Login resumes this wallet's shared thought; an earlier
                        // wallet's navigation clicks must not create a new one.
                        self.thought_epoch = 0;
                        api::reset_notifications();
                        self.notifications = None;
                        self.notification_cache = None;
                        self.browser = load_browser(v["address"].as_str().unwrap_or("local"));
                        self.devices = devices::load(v["address"].as_str().unwrap_or("local"));
                        self.clients.clear();
                        self.client_id.clear();
                        self.dashboard = None;
                        self.prompt.clear();
                        self.attachment = None;
                        self.preview = None;
                        self.preview_text.clear();
                        self.batch.clear();
                    }
                    account::retain_for(v["address"].as_str());
                    self.session = v;
                    if allowed && self.notifications.is_none() {
                        self.notification_cache = Some(message_cache::writer(
                            self.session["address"].as_str().unwrap_or("local").into(),
                            Callback::noop(),
                        ));
                        self.notifications =
                            api::notifications(ctx.link().callback(Msg::Notification)).ok();
                    }
                    if self.auth {
                        api::reset_notifications();
                        self.notifications = None;
                        self.notification_cache = None;
                        self.loading = false;
                        self.booted = false;
                        self.selected = None;
                        self.memos.clear();
                        self.files.clear();
                        self.close_stream();
                        self.clear_batch();
                    }
                    if !self.is_admin() && self.page == "admin" {
                        self.page = "home".into();
                    }
                    if !self.is_admin() && matches!(self.modal.as_str(), "settings" | "rename") {
                        self.modal.clear();
                    }
                    if reload {
                        ctx.link().send_future(async {
                            Msg::Loaded(api("GET", "/api/settings", None).await)
                        });
                    }
                }
                Err(e) => self.error = e,
            },
            Msg::Logout => {
                account::forget();
                ctx.link().send_future(async {
                    let _: Result<Value, _> = api("POST", "/api/auth/logout", None).await;
                    let _ = web_sys::window().unwrap().location().reload();
                    Msg::Boot
                });
                return false;
            }
            Msg::Loaded(r) => match r {
                Ok(mut s) => {
                    self.merge_browser(&mut s);
                    self.prompt = s.draft.clone();
                    self.capture_mode = s.capture_mode.clone();
                    self.display = s.display;
                    self.settings = s;
                    self.booted = true;
                    ctx.link().send_future(async {
                        Msg::Clients(api("GET", "/api/clients", None).await)
                    });
                    self.loading = false;
                    self.auth = false;
                    self.reload(ctx);
                    if let Some(id) = location_memo() {
                        ctx.link().send_message(Msg::Select(id));
                    }
                }
                Err(e) => {
                    self.loading = false;
                    self.handle_error(e);
                }
            },
            Msg::Refresh => {
                if !self.auth && self.connection == api::ConnectionState::Connected {
                    ctx.link().send_future(async {
                        Msg::Clients(api("GET", "/api/clients", None).await)
                    });
                    ctx.link().send_future(async {
                        Msg::Refreshed(api("GET", "/api/settings", None).await)
                    });
                }
                return false;
            }
            Msg::Refreshed(r) => match r {
                Ok(mut s) => {
                    self.merge_browser(&mut s);
                    self.settings.local_ai = s.local_ai;
                    self.settings.user_ai = s.user_ai;
                    self.settings.default_ai = s.default_ai;
                    self.settings.semantic_status = s.semantic_status;
                    self.settings.providers = s.providers;
                    self.settings.provider = s.provider;
                    self.settings.demo = s.demo;
                }
                Err(e) => self.handle_error(e),
            },
            Msg::List(r, version) => {
                if version != self.search_version {
                    return false;
                }
                match r {
                    Ok(r) => {
                        self.total = r.total;
                        self.memos = r.memos;
                    }
                    Err(e) => self.handle_error(e),
                }
            }
            Msg::Files(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                match r {
                    Ok(files) => self.files = files,
                    Err(e) => self.handle_error(e),
                }
            }
            Msg::Navigate(target) => {
                // A target may carry a subpath ("coding/<id>"): the address keeps
                // it, the shell mounts the page before the slash.
                push_location(if target == "home" {
                    "/".into()
                } else {
                    format!("/{target}")
                });
                let page = target.split('/').next().unwrap_or("home").to_string();
                self.navigate(ctx, page);
            }
            Msg::History => {
                self.navigate(ctx, location_page());
                if let Some(id) = location_memo() {
                    ctx.link().send_message(Msg::Select(id));
                }
            }
            Msg::Query(q) => {
                self.query = q;
                let link = ctx.link().clone();
                self.debounce = Some(Timeout::new(250, move || link.send_message(Msg::Search)));
            }
            Msg::Filter(filter) => {
                self.filter = filter;
                ctx.link().send_message(Msg::Search);
            }
            Msg::Search => {
                self.search_version += 1;
                let version = self.search_version;
                let url = format!(
                    "/api/memos?q={}&kind={}",
                    encode(&self.query),
                    encode(&self.filter)
                );
                ctx.link()
                    .send_future(async move { Msg::List(api("GET", &url, None).await, version) });
                return false;
            }
            Msg::Prompt(prompt) => {
                self.prompt = prompt.clone();
                let link = ctx.link().clone();
                self.draft_timer = Some(Timeout::new(500, move || {
                    link.send_message(Msg::PersistDraft(prompt))
                }));
            }
            Msg::PersistDraft(draft) => {
                self.browser["draft"] = json!(draft);
                self.persist_browser();
                return false;
            }
            Msg::Think(capture) => {
                let version = self.selection_version;
                if self.busy {
                    return false;
                }
                self.error.clear();
                self.clear_batch();
                self.busy = true;
                self.stage = "Starting…".into();
                self.phase = if capture { "capture" } else { "ai" }.into();
                let req = ThinkRequest {
                    ask: false,
                    note: false,
                    library: false,
                    provider: Some(self.settings.provider.clone()),
                    model: self
                        .settings
                        .providers
                        .iter()
                        .find(|p| p.id == self.settings.provider)
                        .map(|p| p.model.clone()),
                    capture_mode: Some(self.capture_mode.clone()),
                    display: Some(self.display),
                    client_id: if self.client_id.is_empty()
                        || self.client_id == "all"
                        || self.client_id == "multiple"
                    {
                        None
                    } else {
                        Some(self.client_id.clone())
                    },
                    input_mode: if self.settings.input_mode == "image" {
                        InputMode::Image
                    } else {
                        InputMode::Text
                    },
                    prompt: self.prompt.clone(),
                    capture,
                    agent: self.agent,
                    file_ids: vec![],
                    file_id: if capture {
                        None
                    } else {
                        self.attachment.as_ref().map(|f| f.id.clone())
                    },
                    parent_id: self.selected.as_ref().map(|m| m.id.clone()),
                    room_id: None,
                    message_id: None,
                    clear_epoch: None,
                    data_epoch: Some(
                        self.browser["erased"]
                            .as_str()
                            .and_then(|v| v.parse::<i64>().ok())
                            .unwrap_or(0),
                    ),
                };
                if capture && (self.client_id == "all" || self.client_id == "multiple") {
                    let body = json!({"all":self.client_id=="all","client_ids":self.capture_ids,"request":req});
                    ctx.link().send_future(async move {
                        Msg::BatchStarted(
                            version,
                            api("POST", "/api/capture/batch", Some(body)).await,
                        )
                    });
                    return true;
                }
                ctx.link().send_future(async move {
                    Msg::Started(
                        version,
                        api(
                            "POST",
                            "/api/think",
                            Some(serde_json::to_value(req).unwrap()),
                        )
                        .await,
                    )
                });
            }
            Msg::Started(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                match r {
                    Ok(m) => {
                        self.selected = Some(m.clone());
                        self.prompt.clear();
                        self.attachment = None;
                        self.draft_timer = None;
                        ctx.link().send_message(Msg::PersistDraft(String::new()));
                        if self.page != "quiz" {
                            self.page = "library".into();
                            push_location(format!("/memos/{}", m.id));
                        }
                        self.start_stream(ctx, &m.id);
                    }
                    Err(e) => {
                        self.busy = false;
                        self.handle_error(e);
                    }
                }
            }
            Msg::BatchStarted(version, result) => {
                if version != self.selection_version {
                    return false;
                }
                match result {
                    Ok(items) => {
                        self.selected = None;
                        self.prompt.clear();
                        self.attachment = None;
                        for item in items {
                            if let Some(memo) = &item.memo {
                                let id = memo.id.clone();
                                let _ = api::subscribe(
                                    &id.clone(),
                                    ctx.link()
                                        .callback(move |event| Msg::BatchEvent(id.clone(), event)),
                                );
                            }
                            self.batch.push(BatchRun {
                                item,
                                stage: "Capturing screen…".into(),
                                cursor: 0,
                            });
                        }
                        self.busy = self
                            .batch
                            .iter()
                            .any(|r| r.item.memo.as_ref().is_some_and(|m| m.status == "running"));
                    }
                    Err(error) => {
                        self.busy = false;
                        self.handle_error(error);
                    }
                }
            }
            Msg::BatchEvent(id, event) => {
                if let Some(row) = self
                    .batch
                    .iter_mut()
                    .find(|r| r.item.memo.as_ref().is_some_and(|m| m.id == id))
                {
                    if event.id <= row.cursor {
                        return false;
                    }
                    row.cursor = event.id;
                    let memo = row.item.memo.as_mut().unwrap();
                    match event.kind.as_str() {
                        "stage" => row.stage = event.text,
                        "image" => memo.has_image = true,
                        "ocr" | "context" => memo.input = event.text,
                        "delta" => memo.answer.push_str(&event.text),
                        "done" => {
                            memo.status = "complete".into();
                            row.stage = "Saved to your library".into();
                            api::unsubscribe(&id);
                        }
                        "error" => {
                            memo.status = "error".into();
                            memo.error = Some(event.text);
                            row.stage = "Capture stopped".into();
                            api::unsubscribe(&id);
                        }
                        _ => {}
                    }
                }
                self.busy = self
                    .batch
                    .iter()
                    .any(|r| r.item.memo.as_ref().is_some_and(|m| m.status == "running"));
                if !self.busy {
                    self.reload(ctx);
                }
            }
            Msg::CaptureClientToggle(id) => {
                if self.capture_ids.contains(&id) {
                    self.capture_ids.retain(|v| v != &id);
                } else {
                    self.capture_ids.push(id);
                }
                self.browser["capture_ids"] = json!(self.capture_ids);
                self.persist_browser();
            }
            Msg::QuizKind(kind) => self.new_quiz(kind),
            Msg::NewQuiz(random) => {
                let kind = if random {
                    quiz::Kind::ALL[(js_sys::Math::random() * 4.) as usize]
                } else {
                    self.quiz.kind
                };
                self.new_quiz(kind);
            }
            Msg::RevealQuiz => self.quiz_answer = !self.quiz_answer,
            Msg::QuizCapture => {
                self.quiz_answer = false;
                self.selected = None;
                self.prompt="Solve the quiz visible on the captured screen. For coding questions, provide a complete Rust solution and explain it.".into();
                ctx.link().send_message(Msg::Think(true));
            }
            Msg::Select(id) => {
                self.selection_version += 1;
                let version = self.selection_version;
                self.close_stream();
                self.selected = None;
                self.error.clear();
                self.delete_confirm = false;
                push_location(format!("/memos/{}", encode(&id)));
                let url = format!("/api/memos/{id}");
                ctx.link().send_future(async move {
                    Msg::Selected(version, api("GET", &url, None).await)
                });
            }
            Msg::Selected(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                match r {
                    Ok(mut m) => {
                        self.busy = m.status == "running";
                        self.agent = m.kind == "agent";
                        let id = m.id.clone();
                        m.answer.clear();
                        self.selected = Some(m);
                        self.page = "library".into();
                        self.start_stream(ctx, &id);
                    }
                    Err(e) => self.handle_error(e),
                }
            }
            Msg::Stream(version, e) => {
                if version != self.selection_version || self.stream_id.is_none() {
                    return false;
                }
                if e.id <= self.last_event {
                    return false;
                }
                self.last_event = e.id;
                if e.kind == "delta" {
                    if let Some(m) = self.selected.as_mut() {
                        m.answer.push_str(&e.text);
                    }
                } else if e.kind == "phase" {
                    self.phase = e.text.clone();
                } else if e.kind == "stage" {
                    self.stage = e.text.clone();
                } else if e.kind == "mode" {
                    self.settings.input_mode = e.text.clone();
                } else if e.kind == "kind" {
                    if let Some(m) = self.selected.as_mut() {
                        m.kind = e.text.clone();
                    }
                } else if e.kind == "image" {
                    if let Some(m) = self.selected.as_mut() {
                        m.has_image = true;
                    }
                } else if (e.kind == "ocr" || e.kind == "context")
                    && let Some(m) = self.selected.as_mut()
                {
                    m.input = e.text.clone();
                }
                if e.kind == "done" || e.kind == "error" {
                    self.busy = false;
                    self.close_stream();
                    if let Some(m) = self.selected.as_mut() {
                        m.status = if e.kind == "done" {
                            "complete"
                        } else {
                            "error"
                        }
                        .into();
                        if e.kind == "error" {
                            m.error = Some(e.text.clone());
                        } else {
                            m.error = None;
                        }
                    }
                    self.stage = if e.kind == "done" {
                        "Saved to your library".into()
                    } else {
                        "Response stopped".into()
                    };
                    self.reload(ctx);
                }
                self.events.push(e);
            }
            Msg::Clients(result) => {
                if let Ok(clients) = result {
                    self.set_clients(clients);
                }
            }
            Msg::Client(id) => {
                if id == "multiple" && self.capture_ids.is_empty() {
                    self.capture_ids = self
                        .clients
                        .iter()
                        .filter(|c| c.online && c.id == self.client_id)
                        .map(|c| c.id.clone())
                        .collect();
                }
                self.client_id = id;
                self.browser["client_id"] = json!(self.client_id);
                self.browser["capture_ids"] = json!(self.capture_ids);
                self.persist_browser();
            }
            Msg::Connection(state) => {
                self.connection = state;
                if state == api::ConnectionState::Connected {
                    if !self.booted {
                        ctx.link().send_message(Msg::Boot);
                    }
                    ctx.link().send_message(Msg::Refresh);
                    ctx.link().send_message(Msg::DashboardTick);
                    if api::is_connection_error(&self.error) {
                        self.error.clear();
                    }
                    if self.booted && self.stream_id.is_none() {
                        self.reload(ctx);
                    }
                }
            }
            Msg::DashboardTick => {
                ctx.link().send_message(Msg::Boot);
                if self.connection == api::ConnectionState::Connected
                    && matches!(
                        self.page.as_str(),
                        "devices" | "home" | "chat" | "autonomous"
                    )
                    && !self.auth
                {
                    ctx.link().send_future(async {
                        Msg::DashboardLoaded(api("GET", "/api/dashboard", None).await)
                    });
                }
                return false;
            }
            Msg::DashboardLoaded(result) => {
                if let Ok(d) = result {
                    self.set_clients(d.rust_clients.clone());
                    self.dashboard = Some(d);
                }
            }
            Msg::Cancel => {
                if let Some(m) = &self.selected {
                    let url = format!("/api/memos/{}/cancel", m.id);
                    spawn_local(async move {
                        let _ = api::<Value>("POST", &url, Some(json!({}))).await;
                    });
                }
            }
            Msg::RenameNode(client) => {
                if !self.is_admin() {
                    return false;
                }
                self.rename_id = client.id;
                self.node_name = client.name;
                self.error.clear();
                self.modal = "rename".into();
            }
            Msg::NodeName(name) => self.node_name = name,
            Msg::SaveNodeName => {
                let url = format!("/api/clients/{}", encode(&self.rename_id));
                let body = json!({"name":self.node_name});
                ctx.link().send_future(async move {
                    Msg::NodeRenamed(api("PATCH", &url, Some(body)).await)
                });
            }
            Msg::NodeRenamed(result) => match result {
                Ok(client) => {
                    if let Some(c) = self.clients.iter_mut().find(|c| c.id == client.id) {
                        *c = client;
                    }
                    self.modal.clear();
                    ctx.link().send_message(Msg::DashboardTick);
                    ctx.link()
                        .send_message(Msg::Toast("Computer name saved".into()));
                }
                Err(e) => self.handle_error(e),
            },
            Msg::DeleteEverything => {
                if self.busy {
                    return false;
                }
                self.busy = true;
                self.error.clear();
                ctx.link().send_future(async {
                    Msg::DeletedEverything(api("DELETE", "/api/user/data", None).await)
                });
            }
            Msg::DeletedEverything(r) => {
                self.busy = false;
                match r {
                    Ok(value) => {
                        self.modal.clear();
                        // The same path a `data.deleted` notification takes: caches
                        // cleared, pages remounted, the shared thought started afresh.
                        ctx.link().send_message(Msg::Notification(
                            json!({"kind":"data.deleted","data":value}),
                        ));
                        ctx.link()
                            .send_message(Msg::Toast("Everything was deleted".into()));
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Modal(modal) => {
                self.mobile_menu = false;
                if matches!(modal.as_str(), "settings" | "rename") && !self.is_admin() {
                    return false;
                }
                self.modal = modal.clone();
                self.error.clear();
                if modal == "settings" {
                    ctx.link().send_message(Msg::CheckSetup);
                    self.local_ai_draft = self.settings.local_ai.clone();
                    self.default_ai_draft = backend_default(&self.settings);
                    self.settings_provider = self.default_ai_draft.provider().into();
                    self.model = self
                        .settings
                        .providers
                        .iter()
                        .find(|p| p.id == self.settings_provider)
                        .map(|p| p.model.clone())
                        .unwrap_or_default();
                }
            }
            Msg::LocalAi(field, value) => match field {
                "auto_interval_seconds" => {
                    self.local_ai_draft.auto_interval_seconds = value.parse().unwrap_or(0)
                }
                "local_interval_seconds" => {
                    self.local_ai_draft.local_interval_seconds = value.parse().unwrap_or(0)
                }
                "cloud_min_interval_seconds" => {
                    self.local_ai_draft.cloud_min_interval_seconds = value.parse().unwrap_or(0)
                }
                "local_only" => {
                    self.local_ai_draft.local_only = value == "true";
                    if self.local_ai_draft.local_only {
                        ctx.link().send_message(Msg::DefaultMode("ondevice"));
                    }
                }
                "apple_enabled" => self.local_ai_draft.apple_enabled = value == "true",
                "mlx_enabled" => self.local_ai_draft.mlx_enabled = value == "true",
                "mlx_engine" => self.local_ai_draft.mlx_engine = value,
                "mlx_url" => self.local_ai_draft.mlx_url = value,
                "speech_provider" => self.local_ai_draft.speech_provider = value,
                "speech_locale" => self.local_ai_draft.speech_locale = value,
                "mlx_speech_model" => self.local_ai_draft.mlx_speech_model = value,
                _ => {}
            },
            Msg::DefaultMode(mode) => {
                if self.local_ai_draft.local_only && mode != "ondevice" {
                    return false;
                }
                self.default_ai_draft.mode = mode.into();
                if self.settings_provider != self.default_ai_draft.provider() {
                    ctx.link().send_message(Msg::SettingProvider(
                        self.default_ai_draft.provider().into(),
                    ));
                }
            }
            Msg::DefaultCloud(provider) => self.default_ai_draft.cloud_provider = provider,
            Msg::SettingProvider(p) => {
                if self.local_ai_draft.local_only && !matches!(p.as_str(), "apple" | "mlx") {
                    return false;
                }
                if matches!(p.as_str(), "apple" | "mlx") {
                    self.default_ai_draft.local_provider = p.clone();
                } else {
                    self.default_ai_draft.cloud_provider = p.clone();
                }
                self.settings_provider = p;
                self.model = self
                    .settings
                    .providers
                    .iter()
                    .find(|p| p.id == self.settings_provider)
                    .map(|p| p.model.clone())
                    .unwrap_or_default();
            }
            Msg::ChooseLocalModel(provider, model) => {
                // Picking a model is picking its runtime: a card never leaves
                // the server pointing at a provider that is still switched off.
                match provider.as_str() {
                    "mlx" => self.local_ai_draft.mlx_enabled = true,
                    "apple" => self.local_ai_draft.apple_enabled = true,
                    _ => {}
                }
                self.default_ai_draft.local_provider = provider.clone();
                self.settings_provider = provider;
                self.model = model;
            }
            Msg::Model(v) => self.model = v,
            Msg::CaptureMode(v) => self.capture_mode = v,
            Msg::Display(v) => self.display = v.parse().unwrap_or(1),
            Msg::SaveSettings => {
                let body = json!({"provider":self.settings_provider,"default_ai":self.default_ai_draft,"local_ai":self.local_ai_draft,"model":self.model,"capture_mode":self.capture_mode,"display":self.display,"input_mode":self.settings.input_mode});
                ctx.link().send_future(async move {
                    Msg::SettingsSaved(api("PATCH", "/api/settings", Some(body)).await)
                });
            }
            Msg::SettingsSaved(result) => match result {
                Ok(settings) => {
                    self.browser["provider"] = json!(settings.provider);
                    self.browser["capture_mode"] = json!(settings.capture_mode);
                    self.browser["display"] = json!(settings.display);
                    self.settings = settings;
                    self.persist_browser();
                    ctx.link()
                        .send_message(Msg::Toast("Backend settings saved".into()));
                    ctx.link().send_message(Msg::CheckSetup);
                }
                Err(e) => self.handle_error(e),
            },
            Msg::CheckSetup => {
                self.checking_setup = true;
                ctx.link().send_future(async {
                    Msg::SetupChecked(api("GET", "/api/admin/setup/check", None).await)
                });
            }
            Msg::SetupChecked(result) => {
                self.checking_setup = false;
                match result {
                    // The panel in the dialog carries the verdict; no toast
                    // competes with "Backend settings saved".
                    Ok(report) => self.setup_report = Some(report),
                    Err(e) => self.handle_error(e),
                }
            }
            Msg::InputMode(mode) => {
                self.settings.input_mode = mode.clone();
                self.browser["input_mode"] = json!(mode);
                self.persist_browser();
            }
            Msg::NoteTitle(v) => self.note_title = v,
            Msg::NoteBody(v) => self.note_body = v,
            Msg::SaveNote => {
                let version = self.selection_version;
                let body = json!({"title":self.note_title,"content":self.note_body});
                ctx.link().send_future(async move {
                    Msg::SavedNote(version, api("POST", "/api/memos", Some(body)).await)
                });
            }
            Msg::SavedNote(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                match r {
                    Ok(m) => {
                        self.modal.clear();
                        self.note_title.clear();
                        self.note_body.clear();
                        ctx.link().send_message(Msg::Select(m.id));
                        ctx.link()
                            .send_message(Msg::Toast("Memo saved to your library".into()));
                        self.reload(ctx);
                    }
                    Err(e) => self.handle_error(e),
                }
            }
            Msg::Pin => {
                if let Some(m) = self.selected.as_mut() {
                    m.pinned = !m.pinned;
                    let body = json!({"pinned":m.pinned});
                    let url = format!("/api/memos/{}", m.id);
                    let link = ctx.link().clone();
                    spawn_local(async move {
                        match api::<Memo>("PATCH", &url, Some(body)).await {
                            Ok(_) => link.send_message(Msg::Toast("Library updated".into())),
                            Err(e) => link.send_message(Msg::Toast(e)),
                        }
                    });
                }
            }
            Msg::DeleteAsk => self.delete_confirm = !self.delete_confirm,
            Msg::Delete => {
                if let Some(m) = &self.selected {
                    let url = format!("/api/memos/{}", m.id);
                    ctx.link()
                        .send_future(async move { Msg::Deleted(api("DELETE", &url, None).await) });
                }
            }
            Msg::Deleted(r) => match r {
                Ok(_) => {
                    self.selected = None;
                    self.delete_confirm = false;
                    self.close_stream();
                    push_location("/library".into());
                    self.page = "library".into();
                    self.reload(ctx);
                    ctx.link().send_message(Msg::Toast("Memo deleted".into()));
                }
                Err(e) => self.handle_error(e),
            },
            Msg::Upload(file) => {
                let data_epoch = self.browser["erased"]
                    .as_str()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0);
                let version = self.selection_version;
                self.stage = "Uploading file…".into();
                ctx.link().send_future(async move{
                    use base64::Engine;
                    let result=async{
                        if crate::uploads::is_video(&file){
                            let name=file.name();
                            return crate::uploads::upload_video(file.into(),name,crate::uploads::VideoTarget{data_epoch:Some(data_epoch),..Default::default()},None).await;
                        }
                        if file.size()>25.0*1024.0*1024.0{return Err("Upload must be below 25 MB".into());}
                        let buffer=wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(|_|"Could not read file".to_string())?;
                        let bytes=js_sys::Uint8Array::new(&buffer).to_vec();
                        api("POST","/api/files",Some(json!({"name":file.name(),"data_base64":base64::engine::general_purpose::STANDARD.encode(bytes),"data_epoch":data_epoch}))).await
                    }.await;Msg::Uploaded(version,result)
                });
            }
            Msg::Uploaded(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                match r {
                    Ok(f) => {
                        self.attachment = Some(f);
                        self.reload(ctx);
                        ctx.link()
                            .send_message(Msg::Toast("File saved and attached".into()));
                    }
                    Err(e) => self.handle_error(e),
                }
            }
            Msg::Detach => self.attachment = None,
            Msg::Preview(f) => {
                self.selection_version += 1;
                let version = self.selection_version;
                self.preview = Some(f.clone());
                self.preview_text.clear();
                self.compile_output.clear();
                self.modal = "preview".into();
                if f.mime.starts_with("text/") || f.mime == "application/json" {
                    let url = format!("/api/files/{}/text", f.id);
                    ctx.link().send_future(async move {
                        Msg::PreviewText(version, api("GET", &url, None).await)
                    });
                }
            }
            Msg::PreviewText(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                match r {
                    Ok(v) => {
                        self.preview_text = v["text"].as_str().unwrap_or_default().into();
                        if !v["compilation"].is_null() {
                            ctx.link()
                                .send_message(Msg::Compiled(version, Ok(v["compilation"].clone())));
                        }
                    }
                    Err(e) => self.preview_text = e,
                }
            }
            Msg::Compile => {
                if let Some(f) = &self.preview {
                    self.compile_output = "Compiling…".into();
                    let url = format!("/api/files/{}/compile", f.id);
                    let version = self.selection_version;
                    ctx.link().send_future(async move {
                        Msg::Compiled(version, api("POST", &url, Some(json!({}))).await)
                    });
                }
            }
            Msg::Compiled(version, r) => {
                if version != self.selection_version {
                    return false;
                }
                self.compile_output = match r {
                    Ok(v) => format!(
                        "{}\n{}",
                        if v["success"] == true {
                            "✓ Compilation succeeded. Code was not executed."
                        } else {
                            "Compilation failed"
                        },
                        v["diagnostics"].as_str().unwrap_or("")
                    ),
                    Err(e) => e,
                }
            }
            Msg::Toast(t) => {
                self.toast = t;
                let link = ctx.link().clone();
                self.toast_timer = Some(Timeout::new(3500, move || {
                    link.send_message(Msg::ClearToast)
                }));
            }
            Msg::ClearToast => self.toast.clear(),
            Msg::ClearError => self.error.clear(),
            Msg::Token(v) => self.token = v,
            Msg::Login => {
                let body = json!({"token":self.token});
                ctx.link().send_future(async move {
                    Msg::LoggedIn(api("POST", "/api/session", Some(body)).await)
                });
            }
            Msg::LoggedIn(r) => match r {
                Ok(_) => {
                    self.auth = false;
                    self.error.clear();
                    self.token.clear();
                    ctx.link().send_message(Msg::Boot);
                }
                Err(_) => self.error = "That access token didn’t match. Try again.".into(),
            },
            Msg::ToggleMenu => self.mobile_menu = !self.mobile_menu,
            Msg::Copy => {
                if let Some(m) = &self.selected {
                    let text = m
                        .answer
                        .replace("<!--kind:reply-->", "")
                        .replace("<!--kind:code-->", "")
                        .replace("<!--kind:question-->", "")
                        .trim()
                        .to_string();
                    let link = ctx.link().clone();
                    spawn_local(async move {
                        let navigator = web_sys::window().unwrap().navigator();
                        if js_sys::Reflect::get(&navigator, &"clipboard".into())
                            .is_ok_and(|v| !v.is_undefined())
                        {
                            let result = wasm_bindgen_futures::JsFuture::from(
                                navigator.clipboard().write_text(&text),
                            )
                            .await;
                            link.send_message(Msg::Toast(
                                if result.is_ok() || clipboard::copy_on_http(&text) {
                                    "Answer copied"
                                } else {
                                    "Use Download to save this answer"
                                }
                                .into(),
                            ));
                        } else {
                            link.send_message(Msg::Toast(
                                if clipboard::copy_on_http(&text) {
                                    "Answer copied"
                                } else {
                                    "Select the answer to copy it, or use Download."
                                }
                                .into(),
                            ));
                        }
                    });
                }
            }
            Msg::Shortcut(submit) => {
                if self.auth || self.loading || !self.modal.is_empty() {
                    return false;
                }
                if submit {
                    if self.selected.is_some() || self.page == "overview" {
                        ctx.link().send_message(Msg::Think(
                            self.prompt.trim().is_empty() && self.attachment.is_none(),
                        ));
                    }
                } else {
                    self.mobile_menu = false;
                    if self.page == "library" && self.filter != "all" {
                        // Searching from the sidebar covers the whole library.
                        self.filter = "all".into();
                        self.reload(ctx);
                    }
                    if self.page == "library"
                        && let Some(el) = web_sys::window()
                            .unwrap()
                            .document()
                            .unwrap()
                            .get_element_by_id("memo-search")
                    {
                        let _ = el.dyn_ref::<HtmlInputElement>().unwrap().focus();
                    } else {
                        ctx.link().send_message(Msg::Navigate("library".into()));
                        let link = ctx.link().clone();
                        Timeout::new(50, move || link.send_message(Msg::Shortcut(false))).forget();
                    }
                }
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        if self.auth {
            if self.session["wallet_login"] == true {
                return html! {<><div class="login-language">{theme::button(self.theme, ctx.link().callback(|_|Msg::ToggleTheme))}{i18n::selector(self.language, ctx.link().callback(Msg::Language))}</div><login::Login language={self.language} session={self.session.clone()} changed={ctx.link().callback(|_|Msg::Boot)}/></>};
            }
            if self.session.is_null() {
                return connection::splash(&self.error);
            }
            return self.login_view(ctx);
        }
        html! {
            <div class="app-shell">
                {self.sidebar(ctx)}
                if self.mobile_menu {<button class="sidebar-backdrop" aria-label={t("Close navigation")} onclick={ctx.link().callback(|_|Msg::ToggleMenu)}/>}
                <div class="main-shell">
                    {self.header(ctx)}
                    <ai_activity::Indicator language={self.language} key={self.session["address"].as_str().unwrap_or("local").to_string()}/>
                    if self.page == "dashboard" {<invisible::Usage language={self.language} key={self.session["address"].as_str().unwrap_or("local").to_string()}/> }
                    <main>
                        if !matches!(self.page.as_str(), "home" | "chat") {{companion::page(&self.page, self.busy || self.loading)}}
                        if self.settings.demo{<div class="demo-banner">{icon("info")}<span>{t("Demo workspace · Responses use fixtures. Your files and memos are really saved.")}</span></div>}
                        if !self.error.is_empty(){<div class="error-banner" role="alert"><span>{t(&self.error)}</span><button aria-label={t("Dismiss error")} onclick={ctx.link().callback(|_|Msg::ClearError)}>{icon("close")}</button></div>}
                        if self.loading{<div class="loading-state"><span class="spinner"/>{t("Opening your thinking space…")}</div>}
                        else if self.page=="quiz"{{self.quiz_view(ctx)}if let Some(m)=&self.selected{{self.answer_view(ctx,m)}}else if !self.batch.is_empty(){{self.batch_view(ctx)}}}
                        else if let Some(m)=&self.selected{{self.answer_view(ctx,m)}}
                        else if !self.batch.is_empty(){{self.batch_view(ctx)}}
                        else if self.page=="chat" || self.page=="home"{<chat::Chat data_epoch={self.browser["erased"].as_str().and_then(|v|v.parse::<i64>().ok()).unwrap_or(0)} language={self.language} key={self.session["address"].as_str().unwrap_or("local").to_string()} thought={self.page=="home"} thought_epoch={self.thought_epoch} owner={self.session["address"].as_str().unwrap_or("local").to_string()} clients={self.clients.clone()} devices={self.devices.clone()} choose_device={ctx.link().callback(Msg::CaptureDevice)} settings={self.settings.clone()} ai_saved={ctx.link().callback(|_|Msg::Refresh)} connection={self.connection} admin={self.is_admin()} navigate={ctx.link().callback(Msg::Navigate)} backend_settings={ctx.link().callback(|_|Msg::Modal("settings".into()))} library_attachment={self.attachment.clone()} attachment_consumed={ctx.link().callback(|_|Msg::Detach)} copy={ctx.link().callback(Msg::CopyText)}/>}
                        else if self.page=="save"{<save_anything::SaveAnything language={self.language} data_epoch={self.browser["erased"].as_str().and_then(|v|v.parse::<i64>().ok()).unwrap_or(0)} key={format!("{}:{}",self.session["address"],self.browser["erased"])}/>}
                        else if self.page=="wiki"{<wiki::Wiki language={self.language} key={format!("{}:{}",self.session["address"],self.selection_version)}/> }
                        else if self.page=="autonomous"{<cloud::Cloud owner={self.session["address"].as_str().unwrap_or("local").to_string()} data_epoch={self.browser["erased"].as_str().and_then(|v|v.parse::<i64>().ok()).unwrap_or(0)} key={self.session["address"].to_string()} ai_saved={ctx.link().callback(|_|Msg::Refresh)} language={self.language} settings={self.settings.clone()} clients={self.clients.clone()} active_client={self.devices.active.clone()} navigate={ctx.link().callback(Msg::Navigate)}/> }
                        else if self.page=="assistance"{<assistance::Assistance language={self.language} key={self.session["address"].to_string()}/> }
                        else if self.page=="images"{<images::Images key={format!("{}:{}",self.session["address"],self.selection_version)} language={self.language}/>}
                        else if self.page=="videos"{<videos::Videos key={format!("{}:{}",self.session["address"],self.selection_version)} language={self.language}/>}
                        else if self.page=="code"{<code::Code key={format!("{}:{}",self.session["address"],self.selection_version)} language={self.language} copy={ctx.link().callback(Msg::CopyText)}/>}
                        else if self.page=="coding"{<coding::Coding key={format!("{}:{}",self.session["address"],self.selection_version)} language={self.language} owner={self.session["address"].as_str().unwrap_or("local").to_string()} settings={self.settings.clone()} ai_saved={ctx.link().callback(|_|Msg::Refresh)} copy={ctx.link().callback(Msg::CopyText)}/>}
                        else if self.page=="ai"{{ai_settings::page(self.language,self.session["address"].as_str().unwrap_or("local").to_string(),self.settings.clone(),ctx.link().callback(|_|Msg::Refresh),ctx.link().callback(Msg::Navigate))}}
                        else if self.page=="skills"{<skills::Skills key={format!("{}:{}",self.session["address"],self.selection_version)} language={self.language} owner={self.session["address"].as_str().unwrap_or("local").to_string()} settings={self.settings.clone()} ai_saved={ctx.link().callback(|_|Msg::Refresh)} navigate={ctx.link().callback(Msg::Navigate)}/>}
                        else if self.page=="files"{{self.files_view(ctx)}}
                        else if self.page=="wallet"{<wallet::Wallet key={self.session["address"].as_str().unwrap_or("local").to_string()} session={self.session.clone()} copy={ctx.link().callback(Msg::CopyText)}/>}
                        else if self.page=="dashboard"{<status::Status key={format!("{}-{}",self.session["address"],self.selection_version)} language={self.language} connection={self.connection} copy={ctx.link().callback(Msg::CopyText)}/>}
                        else if self.page=="devices"{{dashboard::view(self.dashboard.as_ref(),self.connection,self.is_admin(),self.session["address"].as_str(),dashboard::Actions { rename:ctx.link().callback(Msg::RenameNode), default:ctx.link().callback(Msg::DefaultCaptureDevice), copy:ctx.link().callback(Msg::CopyText) }, &self.devices.default)}}
                        else if self.page=="admin" && self.is_admin(){<admin::Admin language={self.language} settings={ctx.link().callback(|_|Msg::Modal("settings".into()))}/>}
                        else if self.page=="library"{{self.library_view(ctx)}}
                        else if self.page=="notes"{{self.notes_view(ctx)}}
                        else{{self.home_view(ctx)}}
                        <footer class="page-footer"><span>{brand()}{t("Made for a clearer head.")}</span><span>{t("Your space. Your devices. Your ideas.")}<span class="footer-dot"/></span></footer>
                    </main>
                </div>
                <connection::ConnectionActivity state={self.connection} language={self.language}/>
                {self.modal_view(ctx)}
                if !self.toast.is_empty(){<div class="toast" role="status">{icon("check")}{t(&self.toast)}</div>}
            </div>
        }
    }
    fn destroy(&mut self, _: &Context<Self>) {
        self.close_stream();
        self.clear_batch();
    }
}

impl App {
    fn navigate(&mut self, ctx: &Context<Self>, page: String) {
        if page == "home" {
            self.thought_epoch += 1;
        }
        self.selection_version += 1;
        self.clear_batch();
        self.close_stream();
        self.selected = None;
        self.page = page;
        self.mobile_menu = false;
        self.modal.clear();
        self.query.clear();
        self.filter = if self.page == "notes" { "notes" } else { "all" }.into();
        self.agent = false;
        self.busy = false;
        self.error.clear();
        self.reload(ctx);
        ctx.link().send_message(Msg::DashboardTick);
    }
    fn is_admin(&self) -> bool {
        self.session["is_admin"] == true
    }
    fn clear_batch(&mut self) {
        for row in &self.batch {
            if let Some(memo) = &row.item.memo {
                api::unsubscribe(&memo.id);
            }
        }
        self.batch.clear();
    }
    fn new_quiz(&mut self, kind: quiz::Kind) {
        if self.busy {
            return;
        }
        let mut next = quiz::generate(kind, random_seed());
        while next.question == self.quiz.question {
            next = quiz::generate(kind, random_seed());
        }
        self.quiz = next;
        self.quiz_answer = false;
        self.selected = None;
        self.clear_batch();
    }
    fn quiz_view(&self, ctx: &Context<Self>) -> Html {
        html! {<section class="quiz-playground"><div class="page-intro"><div><div class="eyebrow">{icon("spark")}{t("A LITTLE ROOM TO EXPERIMENT")}</div><h1>{t("Give your bot a little test.")}</h1><p>{t("Fresh numbers, useful coding challenges. Capture a question and see what your bot makes of it.")}</p></div><a class="text-button" href="/quiz" target="_blank" rel="noopener">{t("Open test page")}{icon("arrow")}</a></div>
            {self.mode_switch(ctx)}{self.client_picker(ctx)}
            <div class="quiz-toolbar"><div class="filter-tabs">{for quiz::Kind::ALL.into_iter().map(|kind|html!{<button class={classes!((self.quiz.kind==kind).then_some("selected"))} aria-pressed={(self.quiz.kind==kind).to_string()} disabled={self.busy} onclick={ctx.link().callback(move |_|Msg::QuizKind(kind))}>{t(kind.label())}</button>})}</div><button class="secondary-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::NewQuiz(true))}>{icon("spark")}{t("Random quiz")}</button></div>
            <article class={classes!("quiz-board",(self.quiz.kind==quiz::Kind::Math).then_some("math-quiz"))} data-testid="quiz-board"><div class="eyebrow">{t(self.quiz.kind.label())}{t(" · CAPTURE CHALLENGE")}</div><h2>{t(&self.quiz.title)}</h2><p class="quiz-question" data-testid="quiz-question">{&self.quiz.question}</p>if !self.quiz.code.is_empty(){<pre><code>{&self.quiz.code}</code></pre>}
            if self.quiz_answer{<div class="quiz-answer" data-testid="quiz-answer"><strong>{t("Expected result")}</strong><pre>{&self.quiz.answer}</pre></div>}
            <div class="quiz-actions"><button class="secondary-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::NewQuiz(false))}>{icon("plus")}{t("New numbers")}</button><button class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::RevealQuiz)}>{if self.quiz_answer{t("Hide answer")}else{t("Show answer")}}</button><button class="primary-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::QuizCapture)}>{icon("screen")}{if self.busy{t("Capturing / thinking…")}else{t("Capture this quiz")}}</button></div></article>
            <p class="quiz-hint">{icon("info")}{t("Open this page on the capture computer. Choose that client here or capture from another device. The question stays visible while capture runs.")}</p>
        </section>}
    }
    fn batch_view(&self, ctx: &Context<Self>) -> Html {
        html! {<section class="batch-results"><div class="page-intro"><div><div class="eyebrow">{icon("monitor")}{t("YOUR CONNECTED COMPUTERS")}</div><h1>{t("A view from every screen.")}</h1><p>{tf("{count} capture results · each saved to your library", &[("count", self.batch.len().to_string())])}</p></div></div><div class="batch-grid">{for self.batch.iter().map(|row|html!{<article class="batch-card" data-testid="capture-result"><header><h2>{&row.item.client.name}</h2><span>{t(&row.stage)}</span></header>
            if let Some(error)=&row.item.error{<div class="error-banner" role="alert">{t(error)}</div>}
            if let Some(m)=&row.item.memo{
                if m.has_image{<a href={format!("/api/memos/{}/image",m.id)} target="_blank" rel="noopener" aria-label={t("Open full screenshot")}><img decoding="async" loading="lazy" src={format!("/api/memos/{}/thumbnail",m.id)} alt={tf("Screenshot from {name}", &[("name", row.item.client.name.clone())])}/></a>}
                if !m.answer.is_empty(){<div class="markdown answer-content">{markdown(&m.answer)}</div>}
                if let Some(error)=&m.error{<div class="error-banner" role="alert">{t(error)}</div>}
                if m.status=="running"{<div class="batch-progress"><span class="spinner"/>{t("Processing this screen…")}</div>}
                else{<button class="text-button" disabled={self.busy} onclick={ctx.link().callback({let id=m.id.clone();move |_|Msg::Select(id.clone())})}>{tf("Open result from {name}", &[("name", row.item.client.name.clone())])}{icon("arrow")}</button>}
            }
        </article>})}</div></section>}
    }

    fn persist_devices(&mut self) {
        if !devices::save(
            self.session["address"].as_str().unwrap_or("local"),
            &self.devices,
        ) {
            self.error =
                "Browser storage is unavailable. Settings will last only for this session.".into();
        }
    }
    fn set_clients(&mut self, clients: Vec<CaptureClient>) {
        if self.devices.initialize(&clients) {
            self.persist_devices();
        }
        if self.client_id.is_empty()
            && let Some(scope) = self.browser["client_id"].as_str()
            && matches!(scope, "all" | "multiple")
        {
            self.client_id = scope.into();
            self.capture_ids = self.browser["capture_ids"]
                .as_array()
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
        }
        if self.client_id == "all" || self.client_id == "multiple" {
            self.capture_ids
                .retain(|id| clients.iter().any(|c| c.online && &c.id == id));
            self.clients = clients;
            return;
        }
        let preferred = if self.client_id.is_empty() {
            self.browser["client_id"].as_str().unwrap_or("")
        } else {
            &self.client_id
        };
        let active = clients
            .iter()
            .find(|c| c.online && c.id == preferred)
            .or_else(|| {
                clients
                    .iter()
                    .filter(|c| c.online && c.can_capture)
                    .min_by_key(|c| (&c.name, &c.id))
            })
            .map(|c| c.id.clone())
            .unwrap_or_default();
        self.client_id = active;
        self.clients = clients;
    }
    fn merge_browser(&self, s: &mut Settings) {
        if let Some(prefs) = &s.user_ai {
            s.provider = prefs.provider().into();
        }
        if s.user_ai.as_ref().is_none_or(|p| p.mode == "cloudai")
            && let Some(provider) = self.browser["provider"].as_str()
            && s.providers.iter().any(|p| {
                p.id == provider
                    && (!s.local_ai.local_only || matches!(provider, "apple" | "mlx"))
                    && (s.demo || p.configured || matches!(provider, "apple" | "mlx"))
            })
        {
            s.provider = provider.into();
        }
        for (key, target) in [
            ("input_mode", &mut s.input_mode),
            ("capture_mode", &mut s.capture_mode),
            ("draft", &mut s.draft),
        ] {
            if let Some(v) = self.browser[key].as_str() {
                *target = v.into();
            }
        }
        if let Some(display) = self.browser["display"].as_u64() {
            s.display = display.clamp(1, 16) as u32;
        }
        for p in &mut s.providers {
            if !matches!(p.id.as_str(), "apple" | "mlx")
                && let Some(model) = self.browser["models"][&p.id].as_str()
            {
                p.model = model.into();
            }
        }
    }
    fn persist_browser(&mut self) {
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            && storage
                .set_item(
                    &browser_key(self.session["address"].as_str().unwrap_or("local")),
                    &self.browser.to_string(),
                )
                .is_err()
        {
            self.error =
                "Browser storage is unavailable. Settings will last only for this session.".into();
        }
    }
    fn handle_error(&mut self, e: String) {
        if e == "AUTH_REQUIRED" {
            self.auth = true;
        } else {
            self.error = e;
        }
    }
    fn reload(&self, ctx: &Context<Self>) {
        ctx.link().send_message(Msg::Search);
        let version = self.selection_version;
        ctx.link()
            .send_future(async move { Msg::Files(version, api("GET", "/api/files", None).await) });
    }
    fn close_stream(&mut self) {
        if let Some(id) = self.stream_id.take() {
            api::unsubscribe(&id);
        }
    }
    fn start_stream(&mut self, ctx: &Context<Self>, id: &str) {
        self.close_stream();
        self.events.clear();
        self.last_event = 0;
        let version = self.selection_version;
        match api::subscribe(
            id,
            ctx.link()
                .callback(move |event| Msg::Stream(version, event)),
        ) {
            Ok(()) => self.stream_id = Some(id.into()),
            Err(e) => self.error = e,
        }
    }
    fn nav(
        &self,
        ctx: &Context<Self>,
        page: &str,
        label: &str,
        ico: &str,
        count: Option<usize>,
    ) -> Html {
        let p = page.to_string();
        html! {<button class={classes!("nav-item",(self.page==page&&self.selected.is_none()).then_some("active"))} aria-current={(self.page==page&&self.selected.is_none()).then_some("page")} data-nav={page.to_string()} onclick={ctx.link().callback(move |_|Msg::Navigate(p.clone()))}>{icon(ico)}<span>{t(label)}</span>{count.map(|n|html!{<span class="nav-count">{n}</span>}).unwrap_or_default()}</button>}
    }
    fn sidebar(&self, ctx: &Context<Self>) -> Html {
        // Three everyday actions: capture the screen, save anything, ask anything.
        // Optional tools, including the Wiki, are folded below.
        let library = matches!(
            self.page.as_str(),
            "autonomous"
                | "notes"
                | "wiki"
                | "library"
                | "files"
                | "images"
                | "videos"
                | "code"
                | "coding"
                | "skills"
                | "dashboard"
                | "overview"
                | "quiz"
        );
        html! {<aside class={classes!("sidebar",self.mobile_menu.then_some("open"))}>
            <button class="brand" onclick={ctx.link().callback(|_|Msg::Navigate("home".into()))}><span class="brand-mark">{brand()}</span><span>{"Invisible AI"}<small>{t("A LITTLE EVERYDAY MAGIC")}</small></span></button>
            <div class="nav-label">{t("YOUR LITTLE CORNER")}</div>
            <nav aria-label={t("Main navigation")}>
                {self.nav(ctx,"home","Capture","screen",None)}
                {self.nav(ctx,"save","Save anything","plus",None)}
                {self.nav(ctx,"chat","Ask anything","message",None)}
                {self.nav(ctx,"wallet","Wallet","wallet",None)}
            </nav>
            <details class="more-tools" open={library}><summary>{icon("book")}{t("More tools")}</summary><nav aria-label={t("Additional tools")}>
                {self.nav(ctx,"notes","Notes","file",None)}
                {self.nav(ctx,"wiki","Find anything","search",None)}
                {self.nav(ctx,"autonomous","Autonomous AI","bolt",None)}
                {self.nav(ctx,"library","Library","book",None)}
                {self.nav(ctx,"files","Files","folder",Some(self.files.len()))}
                {self.nav(ctx,"images","Images","image",None)}
                {self.nav(ctx,"videos","Videos","film",None)}
                {self.nav(ctx,"code","Code","code",None)}
                {self.nav(ctx,"coding","Coding","play",None)}
                {self.nav(ctx,"skills","Skills","spark",None)}
                {self.nav(ctx,"dashboard","Dashboard","dashboard",None)}
                {self.nav(ctx,"overview","Multi-device capture","screen",None)}
                {self.nav(ctx,"quiz","Quiz playground","quiz",None)}
            </nav></details>
            <div class="sidebar-bottom"><details class="everyday-settings" open={matches!(self.page.as_str(), "ai" | "assistance" | "admin")}><summary>{icon("settings")}{t("Settings")}</summary>
                {self.nav(ctx,"devices","Devices","monitor",None)}
                {self.nav(ctx,"ai","AI settings","spark",None)}
                {self.nav(ctx,"assistance","Assistance settings","settings",None)}
                if self.is_admin(){ {self.nav(ctx,"admin","Admin","shield",None)} }
                if self.session["address"].is_string() { <button class="settings-nav" onclick={ctx.link().callback(|_|Msg::Modal("account".into()))}>{icon("user")}<span>{t("Account details")}</span></button> }
                <button class="settings-nav danger-nav" data-nav="delete-all" onclick={ctx.link().callback(|_|Msg::Modal("delete-all".into()))}>{icon("trash")}<span>{t("Delete all")}</span></button>
                if self.session["required"]==true { <button class="settings-nav" onclick={ctx.link().callback(|_|Msg::Logout)}>{icon("arrow")}<span>{t("Sign out")}</span></button> }
                </details><div class="profile"><span class="avatar">{"Y"}</span><span><strong>{self.session["name"].as_str().map(str::to_owned).unwrap_or_else(||t("Your workspace"))}</strong><small>{t("Personal · Invisible AI")}</small></span><span class="profile-dot"/></div>
            </div>
        </aside>}
    }
    fn header(&self, ctx: &Context<Self>) -> Html {
        let label = if self.selected.is_some() {
            "Thought"
        } else {
            match self.page.as_str() {
                "save" => "Save anything",
                "wiki" => "Find anything",
                "autonomous" => "Autonomous AI",
                "assistance" => "Assistance settings",
                "library" => "Library",
                "notes" => "Notes",
                "files" => "Files",
                "chat" => "Ask anything",
                "home" => "Capture",
                "images" => "Images",
                "videos" => "Videos",
                "code" => "Code",
                "coding" => "Coding",
                "skills" => "Skills",
                "ai" => "AI settings",
                "admin" => "Admin",
                "devices" => "Devices",
                "dashboard" => "Dashboard",
                "quiz" => "Quiz playground",
                "wallet" => "Wallet",
                _ => "Multi-device capture",
            }
        };
        html! {<header class="topbar"><div class="breadcrumbs"><button class="mobile-toggle" aria-label={t("Open navigation")} aria-expanded={self.mobile_menu.to_string()} onclick={ctx.link().callback(|_|Msg::ToggleMenu)}>{icon("menu")}</button><span>{t("My space")}</span><span class="crumb-separator">{"/"}</span><strong>{t(label)}</strong></div>
            <div class="header-actions">{theme::button(self.theme, ctx.link().callback(|_|Msg::ToggleTheme))}{i18n::selector(self.language, ctx.link().callback(Msg::Language))}{connection::badge(self.connection, ctx.link().callback(|_|Msg::Navigate("devices".into())))}if self.is_admin(){<button class="icon-button" aria-label={t("Open settings")} title={t("Settings")} onclick={ctx.link().callback(|_|Msg::Modal("settings".into()))}>{icon("settings")}</button>}<span class="header-avatar">{"Y"}</span></div>
        </header>}
    }
    fn home_view(&self, ctx: &Context<Self>) -> Html {
        html! {<>
            <div class="page-intro"><div><div class="eyebrow">{icon("spark")}{if self.agent{t("YOUR CODING COMPANION")}else{t("YOUR EVERYDAY THINKING COMPANION")}}</div><h1>{if self.agent{t("Good code starts with a thought.")}else{t("A little clarity. A lot less effort.")}}</h1><p>{if self.agent{t("Read, write, and compile. A helpful pair of hands for your Rust workspace.")}else{t("From what’s on your screen to what’s on your mind. Let’s make sense of it.")}}</p></div><span class="intro-stamp">{t("CAPTURE. THINK. KEEP.")}</span></div>
            {self.mode_switch(ctx)}
            {self.client_picker(ctx)}
            <section class="hero"><img src="/assets/harbour.png" alt={t("A miniature Hong Kong harbour in lavender glass and ivory ceramic")}/><div class="hero-shade"/><div class="hero-content"><span class="hero-pill">{icon("bolt")}{t("LESS COPYING. MORE FLOW.")}</span><h2>{t("Your screen has context.")}<br/><span>{t("Give it a little thought.")}</span></h2><p>{t("Capture your screen. Get a useful answer.\nNo copying, pasting, or switching apps.")}</p><button class="primary-button think-hero" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Think(true))}>{icon("spark")}{t("Capture")}<span class="button-divider"/>{icon("screen")}</button><span class="hero-caption">{if self.settings.input_mode=="image"{t("Capture → Send image → Ask AI")}else{t("Capture → Extract text → Ask AI")}}</span></div></section>
            <section class="quick-actions" aria-label={t("Ways to think")}>
                {self.action_card(ctx,"message","The right words","Turn a comment into a thoughtful reply.","Help me write a natural, thoughtful reply to this conversation.","peach")}
                {self.action_card(ctx,"code","A fresh pair of eyes","Untangle a problem. Find the solution.","Help me understand and fix the code on my screen.","lavender")}
                {self.action_card(ctx,"book","A clearer answer","Ask a question. Connect the dots.","Explain what’s on my screen in simple terms.","sage")}
            </section>
            {self.composer(ctx,false)}
            <section class="recent-section"><div class="section-title"><div><h2>{t("Your recent thoughts")}</h2><span>{t("Good ideas deserve a place to stay.")}</span></div><button class="text-button" onclick={ctx.link().callback(|_|Msg::Navigate("library".into()))}>{t("View library")}{icon("arrow")}</button></div>
                {self.memo_grid(ctx,true)}
            </section>
        </>}
    }
    fn mode_switch(&self, ctx: &Context<Self>) -> Html {
        html! {<div class="mode-row"><div class="mode-switch" role="group" aria-label={t("Screen input mode")}>
            <button class={classes!((self.settings.input_mode!="image").then_some("selected"))} aria-pressed={(self.settings.input_mode!="image").to_string()} onclick={ctx.link().callback(|_|Msg::InputMode("text".into()))}>{icon("file")}{t("Text mode")}</button>
            <button class={classes!((self.settings.input_mode=="image").then_some("selected"))} aria-pressed={(self.settings.input_mode=="image").to_string()} onclick={ctx.link().callback(|_|Msg::InputMode("image".into()))}>{icon("screen")}{t("Image mode")}</button>
        </div><p>{if self.settings.input_mode=="image"{t("Send the screenshot directly to AI")}else{t("Extract text with Apple Vision, then ask AI")}}</p></div>}
    }
    fn client_picker(&self, ctx: &Context<Self>) -> Html {
        let online = self
            .clients
            .iter()
            .filter(|c| c.online && c.can_capture)
            .collect::<Vec<_>>();
        html! {<div class="capture-devices"><div class="client-picker">{icon("monitor")}
            if self.settings.demo && online.is_empty(){<span>{t("Demo capture client")}</span>}
            else if online.is_empty(){<span>{t("No capture client connected")}</span><button class="text-button" onclick={ctx.link().callback(|_|Msg::Navigate("devices".into()))}>{t("Connect a device")}{icon("arrow")}</button>}
            else{<label><span>{t("Capture from")}</span><select aria-label={t("Capture client")} value={self.client_id.clone()} disabled={self.busy} onchange={ctx.link().callback(|e:Event|Msg::Client(e.target_unchecked_into::<HtmlSelectElement>().value()))}>
                {for online.iter().map(|c|html!{<option value={c.id.clone()} selected={self.client_id==c.id}>{&c.name}</option>})}
                <option value="all" selected={self.client_id=="all"}>{tf("All connected clients ({count})", &[("count", online.len().to_string())])}</option>
                <option value="multiple" selected={self.client_id=="multiple"}>{t("Choose multiple…")}</option>
            </select></label><i class="online-dot"/>}
        </div>if self.client_id=="multiple"{<div class="capture-checklist">{for online.iter().map(|c|{let id=c.id.clone();html!{<label><input type="checkbox" checked={self.capture_ids.contains(&c.id)} disabled={self.busy} onchange={ctx.link().callback(move |_|Msg::CaptureClientToggle(id.clone()))}/>{&c.name}</label>}})}</div>}</div>}
    }

    fn action_card(
        &self,
        ctx: &Context<Self>,
        ico: &str,
        title: &str,
        subtitle: &str,
        prompt: &str,
        color: &str,
    ) -> Html {
        let p = t(prompt);
        html! {<button class="quick-card" onclick={ctx.link().callback(move |_|Msg::Prompt(p.clone()))}><span class={classes!("quick-icon",color.to_string())}>{icon(ico)}</span><span><strong>{t(title)}</strong><small>{t(subtitle)}</small></span><span class="quick-arrow">{icon("arrow")}</span></button>}
    }
    fn composer(&self, ctx: &Context<Self>, followup: bool) -> Html {
        html! {<>if followup{{self.mode_switch(ctx)}}<section class="composer" aria-label={t("Ask Invisible AI")}><div class="composer-topline"><span>{icon("spark")}{if followup{t("Keep the thought going")}else{t("Or, start with a thought")}}</span><span class="small-muted">{t("A question, an idea, anything.")}</span></div>
            <textarea id="question" aria-label={t("Ask a question")} placeholder={if followup{t("Ask a follow-up question…")}else{t("What’s on your mind? Ask anything, or add context before you capture…")}} value={self.prompt.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Prompt(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/>
            if let Some(f)=&self.attachment{<div class="attachment">{icon("file")}<span>{&f.name}</span><button aria-label={t("Remove attachment")} onclick={ctx.link().callback(|_|Msg::Detach)}>{icon("close")}</button></div>}
            <div class="composer-bottom"><div class="composer-options">
                {self.upload_button(ctx,false)}<span class="vertical-rule"/>
                <span class="composer-provider"><span class={classes!("provider-symbol",self.settings.provider.clone())}>{provider_symbol(&self.settings.provider)}</span><span class="small-muted">{tf("{provider} · chosen in chat’s AI & tools", &[("provider", provider_label(&self.settings.provider).to_string())])}</span></span>
            </div><div class="composer-submit"><span>{"⌘ Enter"}</span><button class="send-button" aria-label={t("Send question")} title={t("Send question")} disabled={self.busy||(self.prompt.trim().is_empty()&&self.attachment.is_none())} onclick={ctx.link().callback(|_|Msg::Think(false))}>{icon("up")}</button></div></div>
            <div class="composer-footnote">{icon("shield")}<span>{if self.settings.input_mode=="image"{t("Image mode · Screenshots are sent to your selected AI provider.")}else{t("Text mode · Apple Vision reads locally. Only text goes to your AI.")}}</span></div>
        </section></>}
    }
    fn memo_grid(&self, ctx: &Context<Self>, recent: bool) -> Html {
        if self.memos.is_empty() {
            return html! {<div class="empty-state">{icon(if self.query.is_empty(){"book"}else{"search"})}<h3>{if self.query.is_empty(){t("Your next thought belongs here.")}else{t("No thoughts found")}}</h3><p>{if self.query.is_empty(){t("Capture your screen, ask a question, or save your first memo.")}else{t("Try different words or a hashtag, like #rust.")}}</p><button class="secondary-button" onclick={ctx.link().callback(|_|Msg::Modal("note".into()))}>{icon("plus")}{t("Write a memo")}</button></div>};
        }
        html! {<div class="memo-grid">{for self.memos.iter().take(if recent{3}else{50}).map(|m|{
            let id=m.id.clone();let kind=kind_label(&m.kind);let preview=if m.answer.trim().is_empty(){&m.input}else{&m.answer}.lines().filter(|l|!l.starts_with("<!--")&&!l.starts_with("```" )).collect::<Vec<_>>().join(" ").replace(['#','*','`'],"");
            html!{<button class="memo-card" onclick={ctx.link().callback(move |_|Msg::Select(id.clone()))}><div class="memo-card-top"><span class={classes!("kind-chip",m.kind.clone())}>{icon(kind_icon(&m.kind))}{t(kind)}</span><span>{if m.pinned{icon("pin")}else{icon("arrow")}}</span></div><h3>{&m.title}</h3><p>{preview.chars().take(170).collect::<String>()}</p><div class="memo-card-bottom"><div class="tags">{for m.tags.iter().take(2).map(|t|html!{<span>{format!("#{t}")}</span>})}</div><span class="memo-date">{short_date(&m.created_at)}</span></div></button>}
        })}</div>}
    }
    fn search_bar(&self, ctx: &Context<Self>) -> Html {
        html! {<div class="search-bar">{icon("search")}<input id="memo-search" aria-label={t("Search memos")} placeholder={t("Search a thought, a phrase, or #hashtag…")} value={self.query.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Query(e.target_unchecked_into::<HtmlInputElement>().value()))}/><span class="search-mode">{if self.settings.semantic_status=="ready"{t("HYBRID SEARCH")}else{t("KEYWORD + HASHTAGS")}}</span><kbd>{"⌘ K"}</kbd></div>}
    }
    /// Text the user kept with Save anything (and tidied notes), searched with
    /// BM25 keywords fused with semantic similarity, like the Library.
    fn notes_view(&self, ctx: &Context<Self>) -> Html {
        html! {<><div class="page-intro library-intro" data-testid="notes-page"><div><div class="eyebrow">{icon("file")}{t("YOUR NOTES")}</div><h1>{t("Everything you kept, in your words.")}</h1><p>{t("Text you save with Save anything lands here. Search finds it by its words and by its meaning.")}</p></div><button class="primary-button" onclick={ctx.link().callback(|_|Msg::Navigate("save".into()))}>{icon("plus")}{t("Save anything")}</button></div>
            {self.search_bar(ctx)}
            <div class="library-toolbar"><span>{tf("{count} notes", &[("count", self.total.to_string())])}</span></div>
            {self.memo_grid(ctx,false)}
        </>}
    }
    fn library_view(&self, ctx: &Context<Self>) -> Html {
        html! {<><div class="page-intro library-intro"><div><div class="eyebrow">{icon("book")}{t("YOUR PERSONAL KNOWLEDGE SPACE")}</div><h1>{if self.filter=="pinned"{t("The thoughts you keep close.")}else{t("A home for every good thought.")}}</h1><p>{t("Find the answer you had, the idea you saved, or the words you needed.")}</p></div><button class="primary-button" onclick={ctx.link().callback(|_|Msg::Modal("note".into()))}>{icon("plus")}{t("New memo")}</button></div>
            {self.search_bar(ctx)}
            <div class="library-toolbar"><div class="filter-tabs">{for [("all","All thoughts"),("pinned","Pinned"),("reply","Replies"),("code","Code"),("question","Answers"),("memo","Memos"),("agent","Agent")].into_iter().map(|(value,label)|{let v=value.to_string();html!{<button class={classes!((self.filter==value).then_some("selected"))} onclick={ctx.link().callback(move |_|Msg::Filter(v.clone()))}>{t(label)}</button>}})}</div><span>{tf("{count} thoughts", &[("count", self.total.to_string())])}</span></div>
            {self.memo_grid(ctx,false)}
            if self.total>50{<p class="small-muted">{t("Showing the first 50 matches. Refine your search to find a specific memo.")}</p>}
        </>}
    }
    fn answer_view(&self, ctx: &Context<Self>, m: &Memo) -> Html {
        let step = match self.phase.as_str() {
            "capture" => 0,
            "ocr" | "image" => 1,
            "done" => 3,
            _ => 2,
        };
        let image_mode = self.settings.input_mode == "image";
        // What this memo's answer cost: measured wait, tokens, estimated cloud cost.
        let usage_line = m.usage.as_ref().map(|u| {
            let local = matches!(m.provider.as_str(), "apple" | "mlx");
            html! {<p class="chat-llm-log memo-usage" data-testid="memo-usage">{icon(if local {"shield"} else {"globe"})}<span>{t(if local {"Local LLM"} else {"Cloud AI"})}{" · "}{&m.provider}{" · "}{&m.model}{" · "}{u.calls}{" "}{t("AI requests")}{" · "}{t("tokens")}{" "}{u.input_tokens.map(|v|v.to_string()).unwrap_or("—".into())}{"/"}{u.output_tokens.map(|v|v.to_string()).unwrap_or("—".into())}{" · "}{if local {t("No cloud cost")} else if let Some(usd)=u.estimated_usd {format!("{} ${usd:.4}",t("est."))} else {t("Cost unknown")}}</span></p>}
        }).unwrap_or_default();
        html! {<><div class="answer-heading"><button class="text-button" onclick={ctx.link().callback(|_|Msg::Navigate("home".into()))}><span class="back-arrow">{icon("arrow")}</span>{t("Back to your space")}</button><span class="save-status">{if self.busy{html!{<span class="spinner"/>}}else{icon("check")}}{t(&self.stage)}</span></div>
            <section class="answer-card">{usage_line}<div class="answer-title-row"><span class="answer-brand">{brand()}</span><div><h1>{if self.busy{t("A thought is taking shape…")}else{t("A little more clarity.")}}</h1><p>{format!("{} · {}",t(provider_label(&m.provider)),m.model)}</p></div><span class="kind-chip">{t(kind_label(&m.kind))}</span></div>
                if m.has_image{<figure class="capture-preview" data-testid="capture-preview"><figcaption>{icon("screen")}<strong>{t("Screenshot received")}</strong><span>{t("Saved on backend")}</span></figcaption><a href={format!("/api/memos/{}/image",m.id)} target="_blank" rel="noopener" aria-label={t("Open full screenshot")}><img decoding="async" loading="lazy" src={format!("/api/memos/{}/thumbnail",m.id)} alt={t("Captured screen stored locally")}/></a></figure>}
                <details class="source-details"><summary>{icon("file")}{t("Your context")}<span>{t("View source")}</span></summary><pre>{&m.input}</pre></details>
                if self.busy{<div class="pipeline" aria-label={t("Processing progress")}>{for [("screen","Capture"),(if image_mode{"screen"}else{"file"},if image_mode{"Image"}else{"OCR"}),("spark","AI")].iter().enumerate().map(|(i,(symbol,label))|html!{<span class={classes!("pipeline-step",(i<step).then_some("complete"),(i==step).then_some("current"))} aria-current={(i==step).then_some("step")}>{icon(symbol)}{t(label)}</span>})}</div>}
                if !m.answer.is_empty(){<div class="markdown answer-content">{markdown(&m.answer)}</div>}
                if self.busy&&m.answer.is_empty(){<div class="thinking-placeholder"><span/><span/><span/><p>{t(&self.stage)}</p></div>}
                if let Some(error)=&m.error{<div class="error-banner" role="alert">{icon("info")}<span>{t(error)}</span></div>}
                if self.events.iter().any(|e|e.kind=="tool_result"&&e.text.contains("\"run_skill\"")){<p class="chat-skills" data-testid="memo-skills">{icon("spark")}<span>{t("Skills used")}{": "}</span>{for self.events.iter().filter(|e|e.kind=="tool_result").filter_map(|e|{let v:Value=serde_json::from_str(&e.text).ok()?;if v["name"]!="run_skill"{return None;}let out:Value=serde_json::from_str(v["output"].as_str()?).ok()?;Some((out["skill"].as_str()?.to_owned(),out["success"]==true))}).map(|(name,ok)|html!{<span key={name.clone()} class={classes!("chat-skill",(!ok).then_some("is-failed"))}>{name}{if ok{" ✓"}else{" ✗"}}</span>})}</p>}
                if self.events.iter().any(|e|e.kind=="tool_start"||e.kind=="tool_result"||e.kind=="compile"){
                    <details class="tool-activity" open=true><summary>{icon("code")}{t("Workspace activity")}<span>{t("Saved with this thought")}</span></summary>{for self.events.iter().filter(|e|e.kind=="tool_start"||e.kind=="tool_result"||e.kind=="compile").map(|e|{
                        let v:Value=serde_json::from_str(&e.text).unwrap_or(Value::Null);let label=if e.kind=="compile"{"Compiler".into()}else{v["name"].as_str().unwrap_or("Tool").replace('_'," ")};
                        html!{<details class="tool-event"><summary>{icon(if e.kind=="tool_start"{"bolt"}else{"check"})}<span>{t(&label)}</span><small>{if e.kind=="tool_start"{t("called")}else{t("result")}}</small></summary><pre>{if e.kind=="compile"{e.text.clone()}else if e.kind=="tool_start"{v["arguments"].as_str().unwrap_or("").into()}else{v["output"].as_str().unwrap_or("").into()}}</pre></details>}
                    })}</details>
                }
                <div class="answer-actions">if self.busy{<button class="secondary-button" onclick={ctx.link().callback(|_|Msg::Cancel)}>{icon("stop")}{t("Stop")}</button>}else{<><button class="secondary-button" onclick={ctx.link().callback(|_|Msg::Copy)}>{icon("copy")}{t("Copy answer")}</button><a class="icon-button" href={format!("/api/memos/{}/export",m.id)} aria-label={t("Download memo")} title={t("Download memo")}>{icon("download")}</a><button class={classes!("icon-button",m.pinned.then_some("is-pinned"))} aria-label={if m.pinned{t("Unpin memo")}else{t("Pin memo")}} onclick={ctx.link().callback(|_|Msg::Pin)}>{icon("pin")}</button><button class="icon-button delete-action" aria-label={t("Delete memo")} onclick={ctx.link().callback(|_|Msg::DeleteAsk)}>{icon("trash")}</button></>}
                    if self.delete_confirm{<button class="danger-button" onclick={ctx.link().callback(|_|Msg::Delete)}>{t("Confirm delete")}</button>}
                </div>
            </section>{self.composer(ctx,true)}
        </>}
    }
    fn upload_button(&self, ctx: &Context<Self>, full: bool) -> Html {
        html! {<label class={if full{"secondary-button upload-label"}else{"icon-button upload-label"}} title={t("Upload a file")}><input type="file" aria-label={t("Upload file")} onchange={ctx.link().callback(|e:Event|{let el=e.target_unchecked_into::<HtmlInputElement>();let file=el.files().and_then(|fs|fs.get(0));el.set_value("");file.map(Msg::Upload).unwrap_or(Msg::ClearToast)})}/>{icon("upload")}if full{<span>{t("Upload file")}</span>}</label>}
    }
    fn files_view(&self, ctx: &Context<Self>) -> Html {
        html! {<><div class="page-intro library-intro"><div><div class="eyebrow">{icon("folder")}{t("THE THINGS YOU’RE WORKING WITH")}</div><h1>{t("Everything, in its place.")}</h1><p>{t("Your uploads and generated files. Preview, process, download, or delete anytime.")}</p></div>{self.upload_button(ctx,true)}</div>
            <div class="file-info-strip">{icon("shield")}<span>{t("Stored on your backend · Up to 25 MB, videos up to 1 GB · Images, audio, Markdown, CSV and raw files")}</span></div>
            if self.files.is_empty(){<div class="empty-state">{icon("folder")}<h3>{t("Room for what you’re making.")}</h3><p>{t("Upload a file, or let the coding agent create one.")}</p>{self.upload_button(ctx,true)}</div>}
            <div class="files-grid">{for self.files.iter().map(|f|{let preview=f.clone();html!{<article class="file-card"><button class="file-preview" onclick={ctx.link().callback(move |_|Msg::Preview(preview.clone()))}><span class="file-icon">{icon(if f.mime.starts_with("image/"){"screen"}else if f.name.ends_with(".rs"){"code"}else{"file"})}</span><h3>{&f.name}</h3><p>{format!("{} · {}",file_size(f.size),short_date(&f.created_at))}</p></button><div class="file-actions"><button class="text-button" onclick={ctx.link().batch_callback({let f=f.clone();let version=self.selection_version;move |_|vec![Msg::Uploaded(version,Ok(f.clone())),Msg::Navigate("home".into())]})}>{icon("plus")}{t("Use in chat")}</button><a class="icon-button" href={format!("/api/files/{}/download",f.id)} aria-label={tf("Download {name}", &[("name", f.name.clone())])}>{icon("download")}</a><button class="text-button delete-action" aria-label={format!("{} {}",if self.delete_file==f.id{t("Confirm delete")}else{t("Delete")},f.name)} onclick={ctx.link().callback({let id=f.id.clone();move |_|Msg::DeleteFile(id.clone())})}>{if self.delete_file==f.id{t("Confirm delete")}else{t("Delete")}}</button></div></article>}})}</div>
            if let Some(f)=&self.attachment{<div class="file-info-strip"><span>{tf("{name} is ready to use in a chat.", &[("name", f.name.clone())])}</span><button class="text-button" onclick={ctx.link().callback(|_|Msg::Navigate("home".into()))}>{t("Ask a question")}{icon("arrow")}</button></div>}
        </>}
    }
    fn modal_view(&self, ctx: &Context<Self>) -> Html {
        if self.modal == "account" && self.session["address"].is_string() {
            return html! {<account::Account key={self.session["address"].as_str().unwrap_or_default().to_owned()} language={self.language} session={self.session.clone()} changed={ctx.link().callback(|_|Msg::Boot)} close={ctx.link().callback(|_|Msg::Modal(String::new()))}/>};
        }
        if self.modal.is_empty() {
            return Html::default();
        }
        let title = match self.modal.as_str() {
            "settings" => "Backend settings",
            "note" => "Keep a good thought.",
            "rename" => "Name this computer.",
            "delete-all" => "Delete everything?",
            _ => "A closer look.",
        };
        html! {<div class="modal-backdrop" onclick={ctx.link().callback(|_|Msg::Modal(String::new()))}><section class={classes!("modal",(self.modal=="preview").then_some("wide"))} role="dialog" aria-modal="true" aria-labelledby="modal-title" onclick={Callback::from(|e:MouseEvent|e.stop_propagation())}><div class="modal-heading"><div><div class="eyebrow">{if self.modal=="settings"{t("YOUR WORKSPACE SETTINGS")}else if self.modal=="note"{t("NEW MEMO")}else if self.modal=="rename"{t("CAPTURE COMPUTER")}else if self.modal=="delete-all"{t("RIGHT TO BE FORGOTTEN")}else{t("FILE PREVIEW")}}</div><h2 id="modal-title">{t(title)}</h2></div><button class="icon-button" aria-label={t("Close dialog")} onclick={ctx.link().callback(|_|Msg::Modal(String::new()))}>{icon("close")}</button></div>
            if !self.error.is_empty(){<div class="error-banner" role="alert">{t(&self.error)}</div>}
            if self.modal=="settings"{{self.settings_view(ctx)}}
            else if self.modal=="delete-all"{<div class="note-form delete-all-form" data-testid="delete-all-dialog"><p>{t("Permanently delete all your chats, notes, captures, images, files, and results on every session? This cannot be undone. Your login remains active.")}</p><p class="small-muted">{icon("shield")}{t("Everything you added goes: messages, memos, pictures, videos, files, Wiki tags and documents, projects, published skills, code notes, and cached thumbnails on this backend.")}</p><div class="modal-footer"><button class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Modal(String::new()))}>{t("Cancel")}</button><button class="danger-button delete-all-confirm" data-testid="delete-all-confirm" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::DeleteEverything)}>{icon("trash")}{if self.busy{t("Deleting…")}else{t("Confirm delete all")}}</button></div></div>}
            else if self.modal=="rename"{<div class="note-form"><label>{t("Computer name")}<input aria-label={t("Computer name")} value={self.node_name.clone()} maxlength="120" oninput={ctx.link().callback(|e:InputEvent|Msg::NodeName(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label><p class="small-muted">{t("Saved on the backend and synced to the Rust client when it connects.")}</p><div class="modal-footer"><button class="primary-button" disabled={self.node_name.trim().is_empty()} onclick={ctx.link().callback(|_|Msg::SaveNodeName)}>{t("Save computer name")}</button></div></div>}
            else if self.modal=="note"{<div class="note-form"><label>{t("Give it a title")}<input aria-label={t("Memo title")} placeholder={t("A thought worth keeping")} value={self.note_title.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::NoteTitle(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label><label>{t("Your memo")}<textarea aria-label={t("Memo content")} placeholder={t("Write freely. Add #hashtags to find it later.")} value={self.note_body.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::NoteBody(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/></label><div class="modal-footer"><span>{icon("shield")}{t("Saved only in your workspace")}</span><button class="primary-button" disabled={self.note_body.trim().is_empty()} onclick={ctx.link().callback(|_|Msg::SaveNote)}>{icon("check")}{t("Save memo")}</button></div></div>}
            else if let Some(f)=&self.preview{<div class="preview-body"><h3>{&f.name}</h3>if f.mime.starts_with("image/"){<a href={format!("/api/files/{}/image",f.id)} target="_blank" rel="noopener"><img decoding="async" loading="lazy" src={format!("/api/files/{}/thumbnail",f.id)} alt={f.name.clone()}/></a>}else if f.mime.starts_with("audio/"){<audio controls=true src={format!("/api/files/{}/download",f.id)}/>}<pre class="file-code">{&self.preview_text}</pre>if !self.compile_output.is_empty(){<pre class="compile-output" role="status">{&self.compile_output}</pre>}<div class="modal-footer"><button class="secondary-button" onclick={ctx.link().callback(|_|Msg::ProcessFile)}>{if f.mime.starts_with("audio/"){t("Transcribe audio")}else if f.mime.starts_with("image/"){t("Extract text")}else{t("Process file")}}</button><a class="secondary-button" href={format!("/api/files/{}/download",f.id)}>{icon("download")}{t("Download")}</a>if f.name.ends_with(".rs"){<button class="primary-button" disabled={self.compile_output=="Compiling…"} onclick={ctx.link().callback(|_|Msg::Compile)}>{icon("code")}{t("Compile Rust")}</button>}</div></div>}
        </section></div>}
    }
    fn settings_view(&self, ctx: &Context<Self>) -> Html {
        html! {<div class="settings-body"><section class="admin-ai-choice" aria-labelledby="admin-ai-title"><span class="eyebrow">{"SERVER DEFAULT"}</span><h3 id="admin-ai-title">{"Default AI mode for new users"}</h3><p>{"Applies until a user picks their own mode on the Autonomous AI page."}</p>
            <div class="ai-mode-options admin-ai-modes" role="group" aria-label="Default AI processing mode">{for [("ondevice","On-device","Private on your backend Mac","ondevice"),("hybrid","Hybrid","Local first. Cloud help when the local model asks.","hybrid"),("cloudai","Cloud AI","Use your chosen cloud provider","cloudai"),("codex","Codex","Codex command-line tool on the backend Mac","codex"),("claude_code","Claude Code","Claude Code command-line tool on the backend Mac","claude_code")].into_iter().map(|(mode,label,description,symbol)|html!{<button type="button" class={classes!("ai-mode-card",(self.default_ai_draft.mode==mode).then_some("selected"))} aria-pressed={(self.default_ai_draft.mode==mode).to_string()} disabled={self.local_ai_draft.local_only&&mode!="ondevice"} onclick={ctx.link().callback(move |_|Msg::DefaultMode(mode))}>{icon(symbol)}<strong>{label}</strong><small>{description}</small><span class="mode-check" aria-hidden="true">{if self.default_ai_draft.mode==mode{icon("check")}else{Html::default()}}</span></button>})}</div>
            <p class="admin-mode-detail" role="status">{icon(icons::mode_icon(&self.default_ai_draft.mode))}<span>{match self.default_ai_draft.mode.as_str(){"ondevice"=>"OCR, speech, and AI run locally on the backend Mac. No cloud fallback.","hybrid"=>"Start locally. Send a note to cloud AI when the local model asks for help (or ask first, per user).","codex"=>"The backend runs the `codex` command-line tool with its own login; prompts go to OpenAI under that subscription.","claude_code"=>"The backend runs the `claude` command-line tool with its own login; prompts go to Anthropic under that subscription.",_=>"Content is sent to the selected cloud provider when you request AI help."}}</span></p>
            if self.default_ai_draft.mode=="hybrid"{<label class="form-label">{"Cloud provider for approved help"}<select aria-label="Cloud provider for approved help" value={self.default_ai_draft.cloud_provider.clone()} onchange={ctx.link().callback(|e:Event|Msg::DefaultCloud(e.target_unchecked_into::<HtmlSelectElement>().value()))}>{for self.settings.providers.iter().filter(|p|!matches!(p.id.as_str(),"apple"|"mlx")).map(|p|html!{<option value={p.id.clone()} selected={p.id==self.default_ai_draft.cloud_provider}>{&p.name}</option>})}</select></label>}
            <small class="admin-mode-save">{"Changes apply after Save backend settings."}</small></section>
            <section class="local-ai-settings" aria-label="Local AI setup"><h3>{"Local runtimes & safeguards"}</h3><p class="small-muted">{"Enable an installed local runtime to use On-device or Hybrid."}</p>
            <div class="runtime-options">{for [("local_only","Local-only mode",self.local_ai_draft.local_only),("apple_enabled","Enable Apple local",self.local_ai_draft.apple_enabled),("mlx_enabled","Enable MLX local",self.local_ai_draft.mlx_enabled)].into_iter().map(|(field,label,checked)|html!{<label class="runtime-toggle"><input type="checkbox" checked={checked} aria-label={label} onchange={ctx.link().callback(move |e:Event|Msg::LocalAi(field,e.target_unchecked_into::<HtmlInputElement>().checked().to_string()))}/><span>{label}</span></label>})}</div>
            <p class="small-muted">{"Local-only blocks cloud AI, cloud transcription, and web search. Local models use Apple Vision OCR for screenshots."}</p>
            <ondevice::Picker language={self.language} admin={self.is_admin()} provider={self.settings_provider.clone()} model={self.model.clone()} engine={self.local_ai_draft.mlx_engine.clone()} onselect={ctx.link().callback(|(p,m)|Msg::ChooseLocalModel(p,m))}/>
            <label class="form-label">{"MLX engine"}<select aria-label="MLX engine" value={self.local_ai_draft.mlx_engine.clone()} onchange={ctx.link().callback(|e:Event|Msg::LocalAi("mlx_engine",e.target_unchecked_into::<HtmlSelectElement>().value()))}>{for [("builtin","Built-in Rust MLX worker (downloads above)"),("service","OpenAI-compatible MLX service on the backend")].into_iter().map(|(id,label)|html!{<option value={id} selected={self.local_ai_draft.mlx_engine==id}>{label}</option>})}</select></label>
            <label class="form-label">{"Cloud AI polling interval (seconds)"}<input type="number" min="2" max="300" aria-label="Auto interval (seconds)" value={self.local_ai_draft.auto_interval_seconds.to_string()} oninput={ctx.link().callback(|e:InputEvent|Msg::LocalAi("auto_interval_seconds",e.target_unchecked_into::<HtmlInputElement>().value()))}/></label><label class="form-label">{"On-device polling interval (seconds)"}<input type="number" min="2" max="300" aria-label="On-device polling interval (seconds)" value={self.local_ai_draft.local_interval_seconds.to_string()} oninput={ctx.link().callback(|e:InputEvent|Msg::LocalAi("local_interval_seconds",e.target_unchecked_into::<HtmlInputElement>().value()))}/></label><label class="form-label">{"Cloud AI minimum interval (seconds)"}<input type="number" min="2" max="300" aria-label="Cloud AI minimum interval (seconds)" value={self.local_ai_draft.cloud_min_interval_seconds.to_string()} oninput={ctx.link().callback(|e:InputEvent|Msg::LocalAi("cloud_min_interval_seconds",e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>if self.local_ai_draft.mlx_engine=="service"{<label class="form-label">{"MLX server URL"}<input aria-label="MLX server URL" value={self.local_ai_draft.mlx_url.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::LocalAi("mlx_url",e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>}
            <label class="form-label">{"Speech-to-text"}<select aria-label="Speech-to-text provider" value={self.local_ai_draft.speech_provider.clone()} onchange={ctx.link().callback(|e:Event|Msg::LocalAi("speech_provider",e.target_unchecked_into::<HtmlSelectElement>().value()))}>{for [("auto","Follow selected AI"),("apple","Apple Speech · local"),("mlx","MLX Whisper · local"),("grok","Grok · cloud"),("gemini","Gemini · cloud")].into_iter().map(|(id,label)|html!{<option value={id} selected={self.local_ai_draft.speech_provider==id} disabled={self.local_ai_draft.local_only&&matches!(id,"grok"|"gemini")}>{label}</option>})}</select></label>
            <label class="form-label">{"Speech language"}<input aria-label="Speech language" value={self.local_ai_draft.speech_locale.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::LocalAi("speech_locale",e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
            <label class="form-label">{"Installed MLX speech model directory"}<input aria-label="MLX speech model directory" placeholder="/path/to/installed/whisper-model" value={self.local_ai_draft.mlx_speech_model.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::LocalAi("mlx_speech_model",e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
            <p class="small-muted">{"Apple local needs the backend Apple worker and Apple Intelligence. Install speech models on the backend before transcribing; requests never download them automatically."}</p></section>
            <section class={classes!("setup-report",self.setup_report.as_ref().map(|r|if r.ok{"is-ok"}else{"has-problems"}))} aria-label="Setup check" data-testid="setup-report"><div class="setup-report-heading"><h3>{"Setup check"}</h3><button type="button" class="text-button" disabled={self.checking_setup} onclick={ctx.link().callback(|_|Msg::CheckSetup)}>{icon("refresh")}{if self.checking_setup{"Checking…"}else{"Check again"}}</button></div>
            if let Some(r)=&self.setup_report{<ul>{for r.checks.iter().map(|c|html!{<li class={classes!((!c.ok).then_some("problem"))} data-check={c.id.clone()}><span class="setup-mark" aria-hidden="true">{icon(if c.ok{"check"}else{"close"})}</span><strong>{&c.label}</strong>if !c.detail.is_empty(){<small>{&c.detail}</small>}</li>})}</ul><p class="small-muted">{if r.ok{"The backend checked its AI setup and everything is ready."}else{"The backend found problems; fix them and save again."}}</p>}
            else{<p class="small-muted">{"The backend checks its AI setup after every save."}</p>}</section><div class="settings-divider"/><p class="modal-description">{t("AI is configured on your backend. No API key is needed here.")}</p><div class="provider-grid">{for self.settings.providers.iter().map(|p|{let id=p.id.clone();html!{<button class={classes!("provider-card",(p.id==self.settings_provider).then_some("selected"))} disabled={(self.local_ai_draft.local_only && !matches!(p.id.as_str(),"apple"|"mlx")) || (!self.settings.demo && !p.configured && !matches!(p.id.as_str(),"apple"|"mlx"))} onclick={ctx.link().callback(move |_|Msg::SettingProvider(id.clone()))}><span class={classes!("provider-logo",p.id.clone())}>{provider_symbol(&p.id)}</span><strong>{&p.name}</strong><span class={classes!("provider-status",p.configured.then_some("connected"))}>{if p.configured{t("Available")}else if self.settings.demo{t("Demo")}else{t("Not configured")}}</span></button>}})}</div>
            <p class="small-muted">{t("These are server defaults. Choose your personal AI and model in the chat’s AI & tools menu; those choices stay in your browser. API keys stay on the server.")}</p><label class="form-label">{t("Model")}<input aria-label={t("Model")} value={self.model.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Model(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
            <div class="client-setup"><strong>{t("Connect your capture computer")}</strong><span>{t("Devices has the connection command for your server and wallet.")}</span><button class="text-button" onclick={ctx.link().callback(|_|Msg::Navigate("devices".into()))}>{t("Connect a device")}{icon("arrow")}</button></div><div class="settings-divider"/><h3 class="settings-section-title">{icon("screen")}{t("Screen capture")}</h3><p class="small-muted">{t("The backend runs on your Mac Studio. Start the capture client on your MacBook, then use this page from your iPad.")}</p><div class="form-row"><label class="form-label">{t("Capture mode")}<select aria-label={t("Capture mode")} value={self.capture_mode.clone()} onchange={ctx.link().callback(|e:Event|Msg::CaptureMode(e.target_unchecked_into::<HtmlSelectElement>().value()))}><option value="screen" selected={self.capture_mode=="screen"}>{t("Entire display")}</option><option value="region" selected={self.capture_mode=="region"}>{t("Select a region on the Mac")}</option></select></label><label class="form-label display-field">{t("Display")}<input type="number" min="1" max="16" aria-label={t("Display number")} value={self.display.to_string()} oninput={ctx.link().callback(|e:InputEvent|Msg::Display(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label></div>
            <div class="search-status">{icon("search")}<div><strong>{t("Search that connects the dots")}</strong><p>{t(match self.settings.semantic_status.as_str(){"ready"=>"Local semantic embeddings + BM25 + hashtags are ready.","loading"|"starting"=>"The local semantic model is loading. Keyword and hashtag search are ready.","off"=>"Keyword and hashtag search are ready. Semantic search is disabled in this session.",_=>"Keyword and hashtag search are ready. The semantic model is unavailable; check server logs."})}</p></div></div>
            <div class="modal-footer"><span>{icon("shield")}{t("API keys stay on the backend")}</span><button class="primary-button" onclick={ctx.link().callback(|_|Msg::SaveSettings)}>{t("Save backend settings")}{icon("check")}</button></div>
        </div>}
    }
    fn login_view(&self, ctx: &Context<Self>) -> Html {
        html! {<main class="login-page"><div class="login-language">{theme::button(self.theme, ctx.link().callback(|_|Msg::ToggleTheme))}{i18n::selector(self.language, ctx.link().callback(Msg::Language))}</div><section class="login-card"><span class="brand-mark">{brand()}</span><div class="eyebrow">{"INVISIBLE AI"}</div><h1>{t("Welcome to your space.")}</h1><p>{t("Enter the access token configured on your backend.")}</p><input type="password" aria-label={t("Workspace access token")} value={self.token.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Token(e.target_unchecked_into::<HtmlInputElement>().value()))} onkeydown={ctx.link().batch_callback(|e:KeyboardEvent|if e.key()=="Enter"{Some(Msg::Login)}else{None})}/>if !self.error.is_empty(){<p role="alert" class="login-error">{t(&self.error)}</p>}<button class="primary-button" onclick={ctx.link().callback(|_|Msg::Login)}>{t("Open workspace")}{icon("arrow")}</button></section></main>}
    }
}

/// Mirrors the backend: the admin default, else the shared computed policy.
fn backend_default(s: &Settings) -> UserAiPreferences {
    s.default_ai.clone().unwrap_or_else(|| {
        let providers = s
            .providers
            .iter()
            .map(|p| (p.id.clone(), p.configured))
            .collect::<Vec<_>>();
        UserAiPreferences::computed_default(s.local_ai.local_only, &s.provider, &providers)
    })
}

fn location_page() -> String {
    let path = web_sys::window()
        .unwrap()
        .location()
        .pathname()
        .unwrap_or_default();
    match path.trim_matches('/') {
        page @ ("save" | "notes" | "wiki" | "autonomous" | "assistance" | "overview" | "chat"
        | "library" | "files" | "images" | "videos" | "code" | "coding" | "skills"
        | "dashboard" | "devices" | "admin" | "quiz" | "ai" | "wallet") => page.into(),
        // Older links: the backend session page was /cloud and the Dashboard was /status.
        "cloud" => "autonomous".into(),
        "status" => "dashboard".into(),
        _ if path.starts_with("/memos/") => "library".into(),
        _ if path.starts_with("/coding/") => "coding".into(),
        _ if path.starts_with("/wiki/") => "wiki".into(),
        _ => "home".into(),
    }
}
fn location_memo() -> Option<String> {
    let path = web_sys::window()?.location().pathname().ok()?;
    let id = path.strip_prefix("/memos/")?;
    (id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then(|| id.into())
}
const HISTORY_STAMP: &str = "causewaybay.history.v1";
fn mark_history_traversal() {
    if let Some(storage) = web_sys::window().and_then(|w| w.session_storage().ok().flatten()) {
        let _ = storage.set_item(HISTORY_STAMP, &js_sys::Date::now().to_string());
    }
}
/// True within a second of a browser Back/Forward, when a freshly mounted view
/// must show the URL as it is rather than something it remembered. The stamp is
/// consumed, so a later deliberate navigation is not mistaken for a traversal.
pub fn take_history_traversal() -> bool {
    let Some(storage) = web_sys::window().and_then(|w| w.session_storage().ok().flatten()) else {
        return false;
    };
    let stamp = storage.get_item(HISTORY_STAMP).ok().flatten();
    let _ = storage.remove_item(HISTORY_STAMP);
    stamp
        .and_then(|v| v.parse::<f64>().ok())
        .is_some_and(|stamp| js_sys::Date::now() - stamp < 1000.0)
}
/// Swap the current history entry for `path` without adding one, so restoring a
/// remembered Wiki node does not break the browser's Back button.
pub fn replace_location(path: String) {
    let window = web_sys::window().unwrap();
    if window.location().pathname().ok().as_deref() != Some(&path)
        && let Ok(history) = window.history()
    {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&path));
    }
}

fn push_location(path: String) {
    let window = web_sys::window().unwrap();
    if window.location().pathname().ok().as_deref() != Some(&path)
        && let Ok(history) = window.history()
    {
        let _ = history.push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&path));
    }
}

fn markdown(text: &str) -> Html {
    let mut output = String::new();
    pulldown_cmark::html::push_html(
        &mut output,
        pulldown_cmark::Parser::new_ext(text, pulldown_cmark::Options::all()),
    );
    let mut sanitizer = ammonia::Builder::default();
    // Images are allowed only from this backend's own media routes, so a page can
    // show a capture, an uploaded picture, or a video frame but never a remote URL.
    sanitizer
        .add_tags(&["img"])
        .add_tag_attributes("img", &["src", "alt", "loading"])
        .url_relative(ammonia::UrlRelative::PassThrough)
        .attribute_filter(|element, attribute, value| {
            if element == "img" && attribute == "src" {
                let local = value.starts_with("/api/memos/") || value.starts_with("/api/files/");
                return local.then(|| value.into());
            }
            Some(value.into())
        });
    Html::from_html_unchecked(AttrValue::from(drop_sourceless_images(
        &sanitizer.clean(&output).to_string(),
    )))
}
/// Ammonia removes a disallowed `src` but keeps the `<img>` itself; an image
/// that lost its source is nothing but a broken icon, so it goes too.
fn drop_sourceless_images(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<img") {
        out.push_str(&rest[..at]);
        let tag_end = rest[at..].find('>').map_or(rest.len(), |i| at + i + 1);
        let tag = &rest[at..tag_end];
        if tag.contains(" src=\"") {
            out.push_str(tag);
        }
        rest = &rest[tag_end..];
    }
    out.push_str(rest);
    out
}
fn kind_label(s: &str) -> &str {
    match s {
        "reply" => "Reply",
        "code" => "Code",
        "memo" => "Memo",
        "agent" => "Agent",
        "coding" => "Coding",
        "capture" => "Capture",
        _ => "Answer",
    }
}
fn kind_icon(s: &str) -> &str {
    match s {
        "reply" => "message",
        "code" | "agent" | "coding" => "code",
        "memo" => "book",
        _ => "spark",
    }
}
fn provider_label(s: &str) -> &str {
    match s {
        "anthropic" => "Anthropic",
        "openai" => "OpenAI",
        "grok" => "Grok",
        "gemini" => "Gemini",
        "local" => "Local memo",
        "demo" => "Demo",
        _ => "Invisible AI",
    }
}
fn provider_symbol(s: &str) -> &str {
    match s {
        "anthropic" => "✳",
        "openai" => "◎",
        "grok" => "𝕏",
        "gemini" => "✦",
        "apple" => "",
        "mlx" => "M",
        _ => "✧",
    }
}
fn short_date(s: &str) -> String {
    s.get(..10).unwrap_or(s).into()
}
fn file_size(size: usize) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.)
    } else {
        format!("{:.1} MB", size as f64 / 1048576.)
    }
}

fn browser_key(owner: &str) -> String {
    if owner == "local" {
        "causewaybay.settings.v1".into()
    } else {
        format!("causewaybay.settings.{owner}.v1")
    }
}
fn load_browser(owner: &str) -> Value {
    let key = browser_key(owner);
    let mut v = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(&key).ok().flatten())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .unwrap_or(json!({}));
    // Migrate older browser settings: credentials are now backend-owned.
    if v.as_object_mut().unwrap().remove("keys").is_some()
        && let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
        && storage.set_item(&key, &v.to_string()).is_err()
    {
        // If storage is full, remove the old entry rather than retain legacy secrets.
        let _ = storage.remove_item(&key);
    }
    if !v["models"].is_object() {
        v["models"] = json!({});
    }
    v
}

fn random_seed() -> u64 {
    (js_sys::Math::random() * u64::MAX as f64) as u64
}
