//! Account-owned backend sessions. Navigating away never stops a worker.
use crate::{
    api::api,
    i18n::{Language, t},
    icons::icon,
};
use causewaybay_shared::{
    ActivityEntry, ActivitySummary, AssistancePreferences, CaptureClient, ContinuousRequest,
    CreateInvisibleSession, InferenceRecord, InputMode, InvisibleDecision, InvisibleSession,
    MessageSync, Settings, TextEditAvailability, ThinkRequest, UserAiPreferences,
};
use gloo_timers::callback::Interval;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use web_sys::{HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub settings: Settings,
    pub clients: Vec<CaptureClient>,
    /// The workspace's active capture device; the new-session form starts from it.
    #[prop_or_default]
    pub active_client: String,
    pub data_epoch: i64,
    pub navigate: Callback<String>,
    pub ai_saved: Callback<()>,
    /// The signed-in wallet, so the browser's remembered AI pick follows this page's choice.
    #[prop_or_default]
    pub owner: String,
}

pub struct Cloud {
    sessions: Vec<InvisibleSession>,
    stats: Option<ActivitySummary>,
    assistance: Option<AssistancePreferences>,
    preferences: Option<UserAiPreferences>,
    saving_mode: bool,
    mode_error: String,
    loaded: bool,
    polling: bool,
    busy: bool,
    revision: u64,
    error: String,
    load_error: String,
    form: bool,
    title: String,
    client: String,
    /// Watched app for the new session: textedit, notes, or none.
    form_app: String,
    /// Unsent remote prompts per session card.
    prompt_drafts: HashMap<String, String>,
    capture_drafts: HashMap<String, CaptureDraft>,
    /// Add-on sections the user opened; polling keeps its section open regardless.
    capture_open: HashSet<String>,
    /// Browser time of the last successful poll; live counters add the delta since then.
    loaded_at: f64,
    _poll: Interval,
    _tick: Interval,
}

#[derive(Clone)]
struct CaptureDraft {
    client: String,
    interval: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CaptureAction {
    Now,
    Start,
    Stop,
}

/// Everything one poll of the Cloud page fetches.
type PollData = (
    Vec<InvisibleSession>,
    AssistancePreferences,
    Option<UserAiPreferences>,
    ActivitySummary,
);

pub enum Msg {
    Poll,
    Tick,
    Loaded(u64, Box<Result<PollData, String>>),
    Form(bool),
    Title(String),
    Client(String),
    FormApp(String),
    SetApp(String, String),
    ReadScreen(String, bool),
    HybridConfirm(bool),
    LocalGate(bool),
    PromptDraft(String, String),
    QuickPrompt(String, &'static str),
    SendPrompt(String),
    CaptureClient(String, String),
    CaptureInterval(String, String),
    Capture(String, CaptureAction),
    CaptureOpen(String),
    Create(bool),
    Toggle(String, bool),
    SetInterval(String, String),
    Assist(&'static str, String),
    AssistSaved(Result<AssistancePreferences, String>),
    Decide(String, String, bool),
    Done(Box<Result<InvisibleSession, String>>, bool),
    Mode(&'static str),
    ModeSaved(Result<UserAiPreferences, String>),
}

fn capture_mac(client: &CaptureClient) -> bool {
    client.can_capture
}
fn ready_mac(client: &CaptureClient) -> bool {
    client.textedit == TextEditAvailability::Ready
}
fn needs_setup(client: &CaptureClient) -> bool {
    client.textedit == TextEditAvailability::NeedsEnrollment
}
/// A friendly suggested title. It is user content, so it is never translated.
/// Watched app choices; user-facing labels are translated, ids are protocol values.
fn app_options() -> [(&'static str, &'static str); 5] {
    [
        ("auto", "Automatic (front app)"),
        ("textedit", "TextEdit note"),
        ("notes", "Apple Notes (메모)"),
        ("terminal", "Terminal (ai- …)"),
        ("none", "Remote prompts only"),
    ]
}
fn random_title() -> String {
    const FIRST: [&str; 12] = [
        "Quiet", "Bright", "Calm", "Swift", "Golden", "Misty", "Sunny", "Amber", "Velvet", "Cedar",
        "Coral", "Ivory",
    ];
    const SECOND: [&str; 12] = [
        "Harbor", "Meadow", "Summit", "Lantern", "Orchard", "Canyon", "Garden", "Beacon",
        "Harvest", "Lagoon", "Prairie", "Workshop",
    ];
    let pick = |n: usize| (js_sys::Math::random() * n as f64) as usize % n;
    format!(
        "{} {} {:02}",
        FIRST[pick(FIRST.len())],
        SECOND[pick(SECOND.len())],
        pick(100)
    )
}
fn duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {:02}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}
fn clock(iso: &str) -> String {
    let date = js_sys::Date::new(&iso.into());
    if date.get_time().is_nan() {
        return String::new();
    }
    format!(
        "{:02}:{:02}:{:02}",
        date.get_hours(),
        date.get_minutes(),
        date.get_seconds()
    )
}
fn usd(value: f64) -> String {
    if value < 0.01 {
        format!("${value:.4}")
    } else {
        format!("${value:.2}")
    }
}
fn tokens(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
}

impl Component for Cloud {
    type Message = Msg;
    type Properties = Props;

    fn create(ctx: &Context<Self>) -> Self {
        let link = ctx.link().clone();
        link.send_message(Msg::Poll);
        Self {
            sessions: vec![],
            stats: None,
            assistance: None,
            preferences: None,
            saving_mode: false,
            mode_error: String::new(),
            loaded: false,
            polling: false,
            busy: false,
            revision: 0,
            error: String::new(),
            load_error: String::new(),
            form: false,
            title: String::new(),
            client: String::new(),
            form_app: "auto".into(),
            prompt_drafts: HashMap::new(),
            capture_drafts: HashMap::new(),
            capture_open: HashSet::new(),
            loaded_at: js_sys::Date::now(),
            _poll: Interval::new(2000, move || link.send_message(Msg::Poll)),
            _tick: Interval::new(1000, {
                let link = ctx.link().clone();
                move || link.send_message(Msg::Tick)
            }),
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Poll => {
                if !self.polling && !self.busy {
                    self.polling = true;
                    let revision = self.revision;
                    ctx.link().send_future(async move {
                        Msg::Loaded(
                            revision,
                            Box::new(futures_util::try_join!(
                                api("GET", "/api/invisible/sessions", None),
                                api("GET", "/api/assistance/preferences", None),
                                api("GET", "/api/ai/preferences", None),
                                api("GET", "/api/activity/summary", None)
                            )),
                        )
                    });
                }
                return false;
            }
            Msg::Tick => {
                // Only live counters need a repaint between polls.
                return self.sessions.iter().any(|s| {
                    s.activity.thinking.is_some() || s.capture.capturing || s.status.active
                });
            }
            Msg::Loaded(revision, result) => {
                self.polling = false;
                if revision != self.revision || self.busy {
                    return false;
                }
                match *result {
                    Ok((sessions, assistance, preferences, stats)) => {
                        self.loaded_at = js_sys::Date::now();
                        self.stats = Some(stats);
                        let preferences = preferences
                            .unwrap_or_else(|| crate::backend_default(&ctx.props().settings));
                        if self.preferences.as_ref().is_some_and(|p| p != &preferences) {
                            ctx.props().ai_saved.emit(());
                        }
                        self.preferences = Some(preferences);
                        self.sessions = sessions;
                        self.assistance = Some(assistance);
                        self.loaded = true;
                        self.load_error.clear();
                    }
                    Err(e) => self.load_error = e,
                }
            }
            Msg::Form(open) => {
                if self.busy {
                    return false;
                }
                self.form = open;
                if open && self.title.trim().is_empty() {
                    self.title = random_title();
                }
                if self.client.is_empty() {
                    let clients = &ctx.props().clients;
                    self.client = clients
                        .iter()
                        .find(|c| c.id == ctx.props().active_client && capture_mac(c))
                        .or_else(|| clients.iter().find(|c| ready_mac(c)))
                        .map(|c| c.id.clone())
                        .unwrap_or_default();
                }
            }
            Msg::Title(v) => self.title = v,
            Msg::Client(v) => self.client = v,
            Msg::FormApp(v) => self.form_app = v,
            Msg::SetApp(id, app) => {
                if self.busy || self.sessions.iter().any(|s| s.id == id && s.status.active) {
                    return false;
                }
                self.begin();
                ctx.link().send_future(async move {
                    Msg::Done(
                        Box::new(
                            api(
                                "PATCH",
                                &format!("/api/invisible/sessions/{id}"),
                                Some(json!({"app":app})),
                            )
                            .await,
                        ),
                        false,
                    )
                });
            }
            Msg::HybridConfirm(confirm) => {
                if self.busy {
                    return false;
                }
                // Show the new state at once; the saved value replaces it on return.
                if let Some(assistance) = &mut self.assistance {
                    assistance.hybrid_confirm = confirm;
                }
                self.begin();
                ctx.link().send_future(async move {
                    Msg::AssistSaved(
                        api(
                            "PATCH",
                            "/api/assistance/preferences",
                            Some(json!({"hybrid_confirm":confirm})),
                        )
                        .await,
                    )
                });
            }
            Msg::LocalGate(gate) => {
                if self.busy {
                    return false;
                }
                if let Some(assistance) = &mut self.assistance {
                    assistance.local_gate = gate;
                }
                self.begin();
                ctx.link().send_future(async move {
                    Msg::AssistSaved(
                        api(
                            "PATCH",
                            "/api/assistance/preferences",
                            Some(json!({"local_gate":gate})),
                        )
                        .await,
                    )
                });
            }
            Msg::ReadScreen(id, read) => {
                if self.busy {
                    return false;
                }
                self.begin();
                ctx.link().send_future(async move {
                    Msg::Done(
                        Box::new(
                            api(
                                "PATCH",
                                &format!("/api/invisible/sessions/{id}"),
                                Some(json!({"read_screen":read})),
                            )
                            .await,
                        ),
                        false,
                    )
                });
            }
            Msg::PromptDraft(id, text) => {
                self.prompt_drafts.insert(id, text);
            }
            Msg::QuickPrompt(id, prefix) => {
                // A chip fills the verb; the user adds the address, words, or numbers.
                let draft = self.prompt_drafts.entry(id).or_default();
                if !draft.starts_with(prefix) {
                    *draft = format!("{prefix}{}", draft.trim_start());
                }
            }
            Msg::SendPrompt(id) => {
                let text = self
                    .prompt_drafts
                    .get(&id)
                    .map(|t| t.trim().to_string())
                    .unwrap_or_default();
                let running = self.sessions.iter().any(|s| s.id == id && s.status.active);
                if self.busy || text.is_empty() || text.len() > 4000 || !running {
                    return false;
                }
                self.prompt_drafts.remove(&id);
                self.begin();
                ctx.link().send_future(async move {
                    Msg::Done(
                        Box::new(
                            api(
                                "POST",
                                &format!("/api/invisible/sessions/{id}/prompt"),
                                Some(json!({"text":text})),
                            )
                            .await,
                        ),
                        false,
                    )
                });
            }
            Msg::CaptureClient(id, client) => {
                if let Some(session) = self.sessions.iter().find(|s| s.id == id)
                    && session.client_id.is_empty()
                    && !self.busy
                {
                    let mut draft = self.capture_draft(ctx, session);
                    draft.client = client;
                    self.capture_drafts.insert(id, draft);
                }
            }
            Msg::CaptureInterval(id, value) => {
                if let Some(session) = self.sessions.iter().find(|s| s.id == id)
                    && !session.capture.active
                    && !self.busy
                {
                    let mut draft = self.capture_draft(ctx, session);
                    draft.interval = value;
                    self.capture_drafts.insert(id, draft);
                }
            }
            Msg::CaptureOpen(id) => {
                if !self.capture_open.remove(&id) {
                    self.capture_open.insert(id);
                }
            }
            Msg::Capture(id, action) => {
                let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
                    return false;
                };
                self.capture_open.insert(id.clone());
                let draft = self.capture_draft(ctx, session);
                let active = session.capture.active;
                if self.busy
                    || (action != CaptureAction::Stop
                        && (!self.online(ctx, &draft.client) || !self.ai_ready(ctx)))
                    || (action == CaptureAction::Start && (active || session.capture.capturing))
                    || (action == CaptureAction::Now
                        && (session.capture.capture_queued
                            || (!active && session.capture.capturing)))
                {
                    return false;
                }
                let interval = if action == CaptureAction::Start {
                    let Some(interval) = draft
                        .interval
                        .parse::<u64>()
                        .ok()
                        .filter(|n| (2..=300).contains(n))
                    else {
                        self.error = "Choose an interval from 2 to 300 seconds".into();
                        return true;
                    };
                    interval
                } else {
                    0
                };
                let provider = self.preferences(ctx).provider().to_owned();
                let mut request = ThinkRequest {
                    input_mode: if causewaybay_shared::text_only_provider(&provider)
                        || ctx.props().settings.input_mode == "text"
                    {
                        InputMode::Text
                    } else {
                        InputMode::Image
                    },
                    provider: Some(provider),
                    capture: true,
                    capture_mode: Some("screen".into()),
                    display: Some(ctx.props().settings.display),
                    client_id: Some(draft.client.clone()),
                    room_id: Some(id.clone()),
                    message_id: Some(crate::chat::guid()),
                    data_epoch: Some(ctx.props().data_epoch),
                    ..Default::default()
                };
                self.begin();
                ctx.link().send_future(async move {
                    let result = async {
                        let path = format!("/api/rooms/{id}/capture");
                        if action == CaptureAction::Stop {
                            api::<Value>("DELETE", &path, None).await?;
                        } else {
                            let sync: MessageSync =
                                api("GET", &format!("/api/rooms/{id}/sync"), None).await?;
                            request.clear_epoch = Some(sync.cleared_through);
                            let (path, body) = if action == CaptureAction::Start {
                                (
                                    path,
                                    json!(ContinuousRequest {
                                        client_ids: vec![draft.client],
                                        all: false,
                                        interval_seconds: interval,
                                        request,
                                    }),
                                )
                            } else {
                                (
                                    if active {
                                        format!("{path}/now")
                                    } else {
                                        "/api/think".into()
                                    },
                                    json!(request),
                                )
                            };
                            api::<Value>("POST", &path, Some(body)).await?;
                        }
                        api("GET", &format!("/api/invisible/sessions/{id}"), None).await
                    }
                    .await;
                    Msg::Done(Box::new(result), false)
                });
            }
            Msg::Create(start) => {
                if self.busy
                    || self.title.trim().is_empty()
                    || !self.can_create(ctx)
                    || (start && !self.can_start_new(ctx))
                {
                    return false;
                }
                self.begin();
                let value = CreateInvisibleSession {
                    title: Some(self.title.trim().into()),
                    client_id: self.client.clone(),
                    room_id: None,
                    start,
                    app: Some(self.form_app.clone()),
                };
                ctx.link().send_future(async move {
                    Msg::Done(
                        Box::new(api("POST", "/api/invisible/sessions", Some(json!(value))).await),
                        true,
                    )
                });
            }
            Msg::Toggle(id, active) => {
                if self.busy {
                    return false;
                }
                self.begin();
                ctx.link().send_future(async move {
                    Msg::Done(
                        Box::new(
                            api(
                                "POST",
                                &format!(
                                    "/api/invisible/sessions/{id}/{}",
                                    if active { "stop" } else { "start" }
                                ),
                                None,
                            )
                            .await,
                        ),
                        false,
                    )
                });
            }
            Msg::SetInterval(id, value) => {
                let Ok(seconds) = value.trim().parse::<u64>() else {
                    self.error = "Choose an interval from 2 to 300 seconds".into();
                    return true;
                };
                if self.busy || (seconds != 0 && !(2..=300).contains(&seconds)) {
                    self.error = "Choose an interval from 2 to 300 seconds".into();
                    return true;
                }
                self.begin();
                ctx.link().send_future(async move {
                    Msg::Done(
                        Box::new(
                            api(
                                "PATCH",
                                &format!("/api/invisible/sessions/{id}"),
                                Some(json!({"interval_seconds":seconds})),
                            )
                            .await,
                        ),
                        false,
                    )
                });
            }
            Msg::Assist(key, value) => {
                if self.busy {
                    return false;
                }
                let body = if key == "textedit_settle_ms" {
                    let Some(ms) = value
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|s| s.is_finite() && (0.5..=10.0).contains(s))
                        .map(|s| (s * 1000.0).round() as u64)
                    else {
                        self.error = "Choose a typing pause from 0.5 to 10 seconds".into();
                        return true;
                    };
                    json!({ key: ms })
                } else {
                    json!({ key: value })
                };
                self.begin();
                ctx.link().send_future(async move {
                    Msg::AssistSaved(api("PATCH", "/api/assistance/preferences", Some(body)).await)
                });
            }
            Msg::AssistSaved(result) => {
                self.busy = false;
                match result {
                    Ok(p) => self.assistance = Some(p),
                    Err(e) => self.error = e,
                }
                ctx.link().send_message(Msg::Poll);
            }
            Msg::Decide(session, id, approve) => {
                if self.busy {
                    return false;
                }
                self.begin();
                ctx.link().send_future(async move {
                    Msg::Done(
                        Box::new(
                            api(
                                "POST",
                                &format!("/api/invisible/sessions/{session}/approval"),
                                Some(json!(InvisibleDecision { id, approve })),
                            )
                            .await,
                        ),
                        false,
                    )
                });
            }
            Msg::Done(result, created) => {
                self.busy = false;
                match *result {
                    Ok(session) => {
                        self.loaded_at = js_sys::Date::now();
                        if let Some(old) = self.sessions.iter_mut().find(|s| s.id == session.id) {
                            *old = session;
                        } else {
                            self.sessions.insert(0, session);
                        }
                        if created {
                            self.form = false;
                            self.title.clear();
                        }
                    }
                    Err(e) => self.error = e,
                }
                ctx.link().send_message(Msg::Poll);
            }
            Msg::Mode(mode) => {
                if self.busy
                    || !self.loaded
                    || (ctx.props().settings.local_ai.local_only && mode != "ondevice")
                {
                    return false;
                }
                let mut preferences = self.preferences(ctx);
                if preferences.mode == mode {
                    return false;
                }
                preferences.mode = mode.into();
                self.begin();
                self.saving_mode = true;
                self.mode_error.clear();
                ctx.link().send_future(async move {
                    Msg::ModeSaved(
                        api("PATCH", "/api/ai/preferences", Some(json!(preferences))).await,
                    )
                });
            }
            Msg::ModeSaved(result) => {
                self.busy = false;
                self.saving_mode = false;
                match result {
                    Ok(preferences) => {
                        crate::ai_settings::remember(
                            &ctx.props().owner,
                            preferences.provider(),
                            None,
                        );
                        self.preferences = Some(preferences);
                        // The backend revokes pending approvals whenever its policy changes.
                        for session in &mut self.sessions {
                            session.status.approval = None;
                        }
                        ctx.props().ai_saved.emit(());
                    }
                    Err(error) => self.mode_error = error,
                }
                ctx.link().send_message(Msg::Poll);
            }
        }
        true
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let running = self
            .sessions
            .iter()
            .filter(|s| s.status.active || s.capture.active || s.capture.capturing)
            .count();
        let approvals = self
            .sessions
            .iter()
            .filter(|s| s.status.approval.is_some())
            .count();
        let preferences = self.preferences(ctx);
        let mode = preferences.mode.as_str();
        let mode_label = match mode {
            "ondevice" => "On-device",
            "hybrid" => "Hybrid",
            "codex" => "Codex",
            "claude_code" => "Claude Code",
            _ => "Cloud AI",
        };
        let enabled = self.assistance.as_ref().is_some_and(|p| p.textedit_enabled);
        let error = if self.error.is_empty() {
            &self.load_error
        } else {
            &self.error
        };
        html! {<section class="cloud-page" aria-label={t("Backend sessions")}>
            <details class="simple-ai-settings"><summary>{t("AI settings & usage")}{" · "}{t(mode_label)}</summary>
            {self.cost_banner(ctx)}
            <section class="cloud-ai-settings" aria-labelledby="cloud-ai-heading">
                <div class="cloud-ai-heading"><div><h2 id="cloud-ai-heading">{t("AI processing location")}</h2><p>{t("Applies to screen questions and TextEdit replies. Changing this does not start Auto.")}</p></div><span class="cloud-ai-save" role="status">{t(if self.saving_mode {"Saving mode…"} else if !self.mode_error.is_empty() {"Mode not saved"} else if !self.load_error.is_empty() {"Waiting for backend"} else if self.loaded {"Synced with backend"} else {"Loading…"})}</span></div>
                <div class="ai-mode-options" role="group" aria-label={t("AI processing mode")}>
                    {for [("ondevice","On-device","Process privately on your backend Mac.","ondevice"),("hybrid","Hybrid","Local first. Cloud help when the local model asks for it.","hybrid"),("cloudai","Cloud AI","Use your selected cloud provider through its API key.","cloudai"),("codex","Codex","Run the Codex command-line tool signed in on the backend Mac.","codex"),("claude_code","Claude Code","Run the Claude Code command-line tool signed in on the backend Mac.","claude_code")].into_iter().map(|(value,label,description,symbol)|html!{
                        <button type="button" class={classes!("ai-mode-card",(mode==value).then_some("selected"))} aria-pressed={(mode==value).to_string()} disabled={self.busy||!self.loaded||(ctx.props().settings.local_ai.local_only&&value!="ondevice")} onclick={ctx.link().callback(move |_|Msg::Mode(value))}>
                            {icon(symbol)}<strong>{t(label)}</strong><small>{t(description)}</small><span class="mode-check" aria-hidden="true">{if mode==value {icon("check")}else{Html::default()}}</span>
                        </button>
                    })}
                </div>
                if ctx.props().settings.local_ai.local_only {<p class="cloud-mode-hint">{icon("shield")}{t("Your admin has limited this backend to On-device mode.")}</p>}
                if matches!(mode,"codex"|"claude_code") {<p class="cloud-mode-hint">{icon("globe")}{t("Your notes go to that tool's provider through its own login on the backend Mac, billed to that subscription rather than an API key.")}</p>}
                if matches!(mode,"cloudai"|"codex"|"claude_code") {<label class="cloud-mode-hint cloud-check"><span><input type="checkbox" aria-label={t("Let the local LLM decide when cloud AI is needed")} checked={self.assistance.as_ref().is_none_or(|p|p.local_gate)} disabled={self.busy||self.assistance.is_none()} onchange={ctx.link().callback(|e:Event|Msg::LocalGate(e.target_unchecked_into::<HtmlInputElement>().checked()))}/>{t("Let the local LLM decide when cloud AI is needed")}</span><small>{t("Unchanged notes never call any model. When a note changes, the local model first declines non-requests and answers simple ones itself; only harder requests, or an explicit @cloud, @codex or @claude in the note, reach cloud AI, Codex, or Claude Code.")}</small></label>}
                if mode=="hybrid" {<label class="cloud-mode-hint cloud-check"><span><input type="checkbox" aria-label={t("Ask before each cloud request")} checked={self.assistance.as_ref().is_some_and(|p|p.hybrid_confirm)} disabled={self.busy||self.assistance.is_none()} onchange={ctx.link().callback(|e:Event|Msg::HybridConfirm(e.target_unchecked_into::<HtmlInputElement>().checked()))}/>{t("Ask before each cloud request")}</span><small>{t("Off: a note goes to cloud AI as soon as the local model asks for help. On: approve each request first.")}</small></label>}
                if !self.mode_error.is_empty() {<p class="notes-error" role="alert">{t(&self.mode_error)}</p>}
            </section>
            <div class="cloud-summary" aria-label={t("Session summary")}>
                {for [("Backend sessions",self.sessions.len(),"cloud"),("Running",running,"bolt"),("Awaiting approval",approvals,"shield")].into_iter().map(|(label,count,symbol)|html!{<div>{icon(symbol)}<span>{t(label)}</span><strong>{if self.loaded {count.to_string()} else {"—".into()}}</strong></div>})}
            </div>
            <p class="cloud-policy">{icon("shield")}{t("Changing modes cancels pending cloud approvals. Start sessions separately when you are ready.")}</p>
            {self.dashboard(ctx)}
            </details>
            <div class="cloud-toolbar"><h2>{t("Your helpers")}</h2><button class="primary-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Form(true))}>{icon("plus")}{t("Add a helper")}</button></div>
            if !error.is_empty() {<div class="error-banner" role="alert"><span>{t(error)}</span><button class="text-button" disabled={self.busy||self.polling} onclick={ctx.link().callback(|_|Msg::Poll)}>{t("Refresh")}</button></div>}
            if self.form {<form class="cloud-create" aria-label={t("New session")} onsubmit={ctx.link().callback(|e:SubmitEvent|{e.prevent_default();Msg::Create(false)})}>
                if !ctx.props().clients.iter().any(ready_mac) {<div class="helper-setup-note"><p>{t("First, connect the Mac you’d like help on.")}</p><a class="secondary-button" href="/devices#device-setup">{t("Connect my Mac")}{icon("arrow")}</a></div>}
                if !self.ai_ready(ctx) {<div class="helper-setup-note"><p>{t("Choose an AI before starting your helper.")}</p><a class="secondary-button" href="/ai">{t("Choose an AI")}{icon("arrow")}</a></div>}
                if !enabled && self.assistance.is_some() {<div class="helper-setup-note"><p>{t("Turn on help in your apps to get started.")}</p><a class="secondary-button" href="/assistance">{t("Set up app help")}{icon("arrow")}</a></div>}
                <div class="cloud-fields"><label>{t("Name your helper")}<input required=true maxlength="100" value={self.title.clone()} disabled={self.busy} placeholder={t("For example, Studio notes")} oninput={ctx.link().callback(|e:InputEvent|Msg::Title(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                <label>{t("Which Mac?")}<select value={self.client.clone()} disabled={self.busy} onchange={ctx.link().callback(|e:Event|Msg::Client(e.target_unchecked_into::<HtmlSelectElement>().value()))}><option value="" selected={self.client.is_empty()}>{t("Memos and chat only")}</option>{for ctx.props().clients.iter().filter(|c|capture_mac(c)).map(|c|html!{<option value={c.id.clone()} selected={c.id==self.client} disabled={!c.online||needs_setup(c)}>{format!("{}{}",c.name,if !c.online {format!(" · {}",t("Offline"))} else if needs_setup(c) {format!(" · {}",t("Account setup required"))} else {String::new()})}</option>})}</select></label></div>
                <label class="cloud-app-field">{t("Where should I help?")}<select value={self.form_app.clone()} disabled={self.busy} onchange={ctx.link().callback(|e:Event|Msg::FormApp(e.target_unchecked_into::<HtmlSelectElement>().value()))}>{for app_options().into_iter().map(|(value,label)|html!{<option value={value} selected={self.form_app==value}>{t(label)}</option>})}</select></label>
                <p>{t("Choose the Mac and app you want help with.")}</p>
                if let Some(hint)=self.selected_client(ctx).filter(|c|!ready_mac(c)).and_then(|c|crate::invisible::textedit_hint(Some(c))) {<p class="cloud-start-hint">{t(hint)}if self.selected_client(ctx).is_some_and(needs_setup) {<a class="text-button" href="/devices#device-setup">{t("Set up this Mac")}{icon("arrow")}</a>}</p>}
                <p>{t("Start helping lets AI read and reply in the chosen app. You can stop it anytime.")}</p>
                <div class="cloud-form-actions"><button type="button" class="secondary-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Form(false))}>{t("Cancel")}</button><button type="button" class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Title(random_title()))}>{icon("refresh")}{t("Suggest a name")}</button><button type="submit" class="secondary-button" disabled={self.busy||self.title.trim().is_empty()||!self.can_create(ctx)}>{t("Save for later")}</button><button type="button" class="primary-button" disabled={self.busy||self.title.trim().is_empty()||!self.can_start_new(ctx)} onclick={ctx.link().callback(|_|Msg::Create(true))}>{icon("bolt")}{t("Start helping")}</button></div>
            </form>}

            if !self.loaded {<div class="cloud-empty" role="status">{t("Loading sessions…")}</div>}
            else if self.sessions.iter().all(|s|s.client_id.is_empty() && !s.status.active && !s.capture.active && !s.capture.capturing) {<div class="cloud-empty">{icon("cloud")}<h3>{t("Your first helper is one click away.")}</h3><p>{t("Tap Add a helper to choose what you’d like help with.")}</p></div>}
            else {<div class="cloud-session-list">{for self.sessions.iter().filter(|s|!s.client_id.is_empty() || s.status.active || s.capture.active || s.capture.capturing).map(|s|self.session(ctx,s,enabled))}</div>}
            if self.sessions.iter().any(|s|s.client_id.is_empty() && !s.status.active && !s.capture.active && !s.capture.capturing) {<details class="saved-chats"><summary>{t("Saved chats")}</summary><p>{t("These are conversations, not running helpers.")}</p>{for self.sessions.iter().filter(|s|s.client_id.is_empty() && !s.status.active && !s.capture.active && !s.capture.capturing).map(|s|html!{<a href={format!("/chat?room={}",s.id)}>{icon("message")}{&s.title}{icon("arrow")}</a>})}</details>}
        </section>}
    }
}

impl Cloud {
    fn capture_draft(&self, ctx: &Context<Self>, session: &InvisibleSession) -> CaptureDraft {
        let mut draft = self
            .capture_drafts
            .get(&session.id)
            .cloned()
            .unwrap_or(CaptureDraft {
                client: String::new(),
                interval: if session.capture.interval_seconds > 0 {
                    session.capture.interval_seconds
                } else {
                    ctx.props().settings.local_ai.auto_interval_seconds
                }
                .clamp(2, 300)
                .to_string(),
            });
        if !session.client_id.is_empty() {
            draft.client = session.client_id.clone();
        }
        if session.capture.active {
            draft.interval = session.capture.interval_seconds.to_string();
        }
        draft
    }
    fn selected_client<'a>(&self, ctx: &'a Context<Self>) -> Option<&'a CaptureClient> {
        ctx.props().clients.iter().find(|c| c.id == self.client)
    }
    /// A session may be created without a Mac, or with a connected Mac enrolled to this account.
    fn can_create(&self, ctx: &Context<Self>) -> bool {
        self.client.is_empty()
            || (self.online(ctx, &self.client)
                && self.selected_client(ctx).is_some_and(|c| !needs_setup(c)))
    }
    /// Same conditions the session card applies before its Start button is enabled.
    fn can_start_new(&self, ctx: &Context<Self>) -> bool {
        let client = ctx.props().clients.iter().find(|c| c.id == self.client);
        !self.client.is_empty()
            && self.online(ctx, &self.client)
            && client.is_some_and(ready_mac)
            && self.ai_ready(ctx)
            && self.assistance.as_ref().is_some_and(|p| p.textedit_enabled)
            && !self
                .sessions
                .iter()
                .any(|s| s.client_id == self.client && s.status.active)
    }
    fn provider_name<'a>(&self, ctx: &'a Context<Self>, id: &'a str) -> &'a str {
        ctx.props()
            .settings
            .providers
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
            .unwrap_or(id)
    }
    fn location_badge(&self, location: &str) -> Html {
        match location {
            "local" => {
                html! {<span class="ai-location is-local">{icon("shield")}{t("Local LLM")}</span>}
            }
            "cloud" => {
                html! {<span class="ai-location is-cloud">{icon("globe")}{t("Cloud AI")}</span>}
            }
            _ => Html::default(),
        }
    }
    fn llm_label(&self, ctx: &Context<Self>, provider: &str, model: &str) -> Html {
        if provider.is_empty() {
            return Html::default();
        }
        html! {<span class="cloud-llm">{self.provider_name(ctx, provider)}if !model.is_empty() {<small>{" · "}{model}</small>}</span>}
    }
    fn timeline_entry(&self, ctx: &Context<Self>, entry: &ActivityEntry) -> Html {
        let source = match entry.source.as_str() {
            "textedit" => ("file", "TextEdit"),
            "notes" => ("file", "Notes"),
            "terminal" => ("code", "Terminal"),
            "remote" => ("spark", "Remote"),
            "capture" => ("screen", "Capture"),
            _ => ("message", "Chat"),
        };
        html! {<li><time>{clock(&entry.at)}</time><span class={classes!("cloud-source",format!("is-{}",entry.source))}>{icon(source.0)}{t(source.1)}</span><span class="cloud-step">{t(&entry.stage)}</span>{self.llm_label(ctx,&entry.provider,&entry.model)}{self.location_badge(&entry.location)}</li>}
    }
    fn inference_row(&self, ctx: &Context<Self>, row: &InferenceRecord) -> Html {
        let status = match row.status.as_str() {
            "running" => "Running",
            "complete" => "Complete",
            "interrupted" => "Interrupted",
            _ => "Error",
        };
        let cost = if row.location != "cloud" {
            t("No cloud cost")
        } else if let Some(usd_value) = row.estimated_usd {
            usd(usd_value)
        } else {
            t("Unknown")
        };
        html! {<tr class={classes!(format!("is-{}",row.status))}>
            <td>{clock(&row.created_at)}</td>
            <td>{self.llm_label(ctx,&row.provider,&row.model)}{self.location_badge(&row.location)}</td>
            <td>{if row.status=="running" {duration(self.live_wait(row.elapsed_ms))} else {duration(row.elapsed_ms)}}if let Some(first)=row.first_token_ms {<small>{t("first output")}{" "}{duration(first)}</small>}</td>
            <td>{tokens(row.input_tokens)}{" / "}{tokens(row.output_tokens)}</td>
            <td>{cost}</td>
            <td><span class={classes!("cloud-run-state",format!("is-{}",row.status))}>{t(status)}</span></td>
        </tr>}
    }
    fn live_wait(&self, waited_ms: u64) -> u64 {
        waited_ms + (js_sys::Date::now() - self.loaded_at).max(0.0) as u64
    }
    fn activity(&self, ctx: &Context<Self>, session: &InvisibleSession) -> Html {
        let activity = &session.activity;
        let status = &session.status;
        let thinking = activity.thinking.as_ref();
        let running = thinking.is_some() || session.capture.capturing || status.active;
        let (headline, detail, llm, location) = if let Some(now) = thinking {
            (
                t("Waiting for the AI"),
                format!(
                    "{} {}",
                    t("Waited"),
                    duration(self.live_wait(now.waited_ms))
                ),
                self.llm_label(ctx, &now.provider, &now.model),
                now.location.as_str(),
            )
        } else if session.capture.capturing {
            (
                t("Capturing your screen"),
                String::new(),
                Html::default(),
                "",
            )
        } else if status.active {
            (
                t(&status.stage),
                t(match (status.app.as_str(), status.resolved_app.as_str()) {
                    ("none", _) => "Waiting for your device",
                    (_, "terminal") => "Watching the terminal",
                    ("terminal", _) => "Watching the terminal",
                    (_, "notes") => "Watching Notes",
                    (_, "textedit") => "Watching TextEdit",
                    ("notes", _) => "Watching Notes",
                    ("textedit", _) => "Watching TextEdit",
                    _ => "Watching your note",
                }),
                self.llm_label(ctx, &status.provider, &status.model),
                causewaybay_shared::ai_location(&status.provider),
            )
        } else if let Some(last) = activity.inferences.first() {
            (
                t("Idle"),
                format!("{} {}", t("Last reply took"), duration(last.elapsed_ms)),
                self.llm_label(ctx, &last.provider, &last.model),
                last.location.as_str(),
            )
        } else {
            (t("Idle"), t("No AI activity yet"), Html::default(), "")
        };
        let totals = &activity.totals;
        let cost = match totals.estimated_usd {
            Some(value) => usd(value),
            None if totals.unpriced_cloud_requests > 0 => t("Unknown"),
            None => t("No cloud cost"),
        };
        let shown = 8;
        html! {<section class="cloud-activity" aria-label={t("Progress")} data-testid="session-progress">
            <h4>{icon("bolt")}{t("Progress")}</h4>
            <div class={classes!("cloud-now",running.then_some("is-running"),thinking.map(|_|"is-thinking"))} role="status" aria-live="polite">
                <span class="cloud-pulse" aria-hidden="true"><i/><i/><i/></span>
                <div class="cloud-now-text"><strong>{headline}</strong>if !detail.is_empty() {<small data-testid="llm-wait">{detail}</small>}</div>
                <div class="cloud-now-llm">{llm}{self.location_badge(location)}</div>
            </div>
            <div class="cloud-progress-track" aria-hidden="true"><i/></div>
            <details class="helper-history"><summary>{t("Activity & usage")}</summary><dl class="cloud-totals">
                <div><dt>{t("AI requests")}</dt><dd>{totals.requests}</dd></div>
                <div><dt>{t("Total wait")}</dt><dd>{duration(totals.elapsed_ms)}</dd></div>
                <div><dt>{t("Tokens in / out")}</dt><dd>{tokens(totals.input_tokens)}{" / "}{tokens(totals.output_tokens)}</dd></div>
                <div><dt>{t("Estimated cloud cost")}</dt><dd>{cost}</dd></div>
            </dl>
            if !activity.price_date.is_empty() {<p class="cloud-price-note">{t("Estimated from list prices dated")}{" "}{&activity.price_date}{". "}{t("Not a provider bill.")}</p>}
            else if totals.unpriced_cloud_requests>0 {<p class="cloud-price-note">{t("Cloud cost is unknown when the provider reports no tokens or the model has no listed price.")}</p>}
            if activity.entries.is_empty() {<p class="cloud-activity-empty">{t("Steps appear here as the backend works.")}</p>}
            else {
                <ol class="cloud-timeline" aria-label={t("Recent steps")}>{for activity.entries.iter().take(shown).map(|e|self.timeline_entry(ctx,e))}</ol>
                if activity.entries.len()>shown {<details class="cloud-timeline-more"><summary>{t("Earlier steps")}{" · "}{activity.entries.len()-shown}</summary><ol class="cloud-timeline">{for activity.entries.iter().skip(shown).map(|e|self.timeline_entry(ctx,e))}</ol></details>}
            }
            if !activity.inferences.is_empty() {<div class="cloud-inferences"><table aria-label={t("Recent AI requests")}>
                <thead><tr><th>{t("Time")}</th><th>{t("LLM")}</th><th>{t("Wait")}</th><th>{t("Tokens in / out")}</th><th>{t("Cost")}</th><th>{t("State")}</th></tr></thead>
                <tbody>{for activity.inferences.iter().map(|r|self.inference_row(ctx,r))}</tbody>
            </table></div>}
            </details>
        </section>}
    }
    /// The first thing on the page: what cloud AI cost, and how much never reached it.
    fn cost_banner(&self, ctx: &Context<Self>) -> Html {
        let Some(stats) = &self.stats else {
            return Html::default();
        };
        let preferences = self.preferences(ctx);
        let mode = preferences.mode.as_str();
        let cost = |b: &causewaybay_shared::UsageBucket| -> String {
            match b.estimated_usd {
                Some(v) => usd(v),
                None if b.requests > 0 => t("Unknown"),
                None => usd(0.0),
            }
        };
        let flow: [(&str, &str); 3] = match mode {
            "ondevice" => [
                ("shield", "On-device AI polls and answers"),
                ("check", "No cloud calls"),
                ("shield", "Nothing leaves the Mac"),
            ],
            "hybrid" => [
                ("shield", "On-device AI polls and answers first"),
                ("spark", "Cloud only when the local model asks"),
                ("globe", "Cloud AI"),
            ],
            "codex" => [
                ("shield", "On-device AI polls and judges"),
                ("spark", "Simple requests answered locally"),
                ("code", "Codex only when needed"),
            ],
            "claude_code" => [
                ("shield", "On-device AI polls and judges"),
                ("spark", "Simple requests answered locally"),
                ("code", "Claude Code only when needed"),
            ],
            _ => [
                ("shield", "On-device AI polls and judges"),
                ("spark", "Simple requests answered locally"),
                ("globe", "Cloud AI only when needed"),
            ],
        };
        let avoided = |p: &causewaybay_shared::ProcessingStats| {
            p.no_request + p.answered_locally + p.skipped_unchanged
        };
        html! {<section class="cloud-cost" aria-label={t("Cost at a glance")} data-testid="cost-banner">
            <div class="cloud-cost-flow" aria-label={t("How this mode works")}>
                {for flow.iter().enumerate().map(|(i,(symbol,label))|html!{<><span class="cloud-cost-step">{icon(symbol)}{t(label)}</span>if i<2 {<i class="cloud-cost-arrow" aria-hidden="true">{"→"}</i>}</>})}
            </div>
            <div class="cloud-cost-grid">
                <div class="cloud-cost-card is-cloud" data-testid="cost-today"><span>{icon("globe")}{t("Cloud spend today")}</span><strong>{cost(&stats.day.cloud)}</strong><small>{stats.day.cloud.requests}{" "}{t("cloud calls")}{" · "}{t("tokens")}{" "}{tokens(stats.day.cloud.input_tokens)}{" / "}{tokens(stats.day.cloud.output_tokens)}</small></div>
                <div class="cloud-cost-card is-cloud" data-testid="cost-total"><span>{icon("globe")}{t("Cloud spend all time")}</span><strong>{cost(&stats.total.cloud)}</strong><small>{stats.total.cloud.requests}{" "}{t("cloud calls")}</small></div>
                <div class="cloud-cost-card is-local" data-testid="cost-local"><span>{icon("shield")}{t("Handled on-device today")}</span><strong>{stats.day.local.requests}</strong><small>{t("local LLM calls · no cloud cost")}</small></div>
                <div class="cloud-cost-card is-saved" data-testid="cost-avoided"><span>{icon("check")}{t("Cloud calls avoided today")}</span><strong>{avoided(&stats.day)}</strong><small>{stats.day.no_request}{" "}{t("no request")}{" · "}{stats.day.answered_locally}{" "}{t("answered locally")}{" · "}{stats.day.skipped_unchanged}{" "}{t("unchanged")}</small></div>
            </div>
            <p class="cloud-price-note">{t(if matches!(mode,"codex"|"claude_code") {"Codex and Claude Code bill your subscription, so their calls show $0 here; tokens are still counted."} else {"Estimates use dated list prices and provider-reported tokens. Not a provider bill."})}</p>
        </section>}
    }
    /// Wallet-wide processing numbers; the poll refreshes them after every AI job.
    fn dashboard(&self, ctx: &Context<Self>) -> Html {
        let Some(stats) = &self.stats else {
            return html! {<section class="cloud-dashboard" aria-label={t("Processing stats")} data-testid="processing-stats"><p class="cloud-activity-empty">{t("Loading…")}</p></section>};
        };
        let busy_now = stats.total.local.running + stats.total.cloud.running;
        let bucket = |b: &causewaybay_shared::UsageBucket| -> Html {
            html! {<>
                <strong>{b.requests}</strong>
                <small>{t("wait")}{" "}{duration(b.elapsed_ms)}if b.errors>0 {{format!(" · {} {}",b.errors,t("errors"))}}</small>
                if !b.providers.is_empty() {<small class="cloud-dash-providers">{for b.providers.iter().map(|(p,n)|html!{<span key={p.clone()}>{self.provider_name(ctx,p)}{" "}{n}</span>})}</small>}
            </>}
        };
        let cost = |b: &causewaybay_shared::UsageBucket| -> String {
            match b.estimated_usd {
                Some(v) => usd(v),
                None if b.requests > 0 => t("Unknown"),
                None => "—".into(),
            }
        };
        html! {<section class="cloud-dashboard" aria-label={t("Processing stats")} data-testid="processing-stats">
            <div class="cloud-dashboard-heading"><h2>{icon("bolt")}{t("Processing stats")}</h2><span class={classes!("cloud-dash-live",(busy_now>0).then_some("is-running"))} role="status"><i/>{if busy_now>0 {format!("{} {}",busy_now,t("running"))} else if stats.last_at.is_empty() {t("No AI processing yet")} else {format!("{} {}",t("Updated"),clock(&stats.last_at))}}</span></div>
            {for [("day",&stats.day,"Last 24 hours"),("total",&stats.total,"All time")].into_iter().map(|(key,p,label)|html!{
                <div class="cloud-dash-window" data-window={key}><h3>{t(label)}</h3>
                <div class="cloud-dash-grid">
                    <div class="cloud-dash-card"><span>{icon("screen")}{t("OCR runs")}</span><strong>{p.ocr_runs}</strong><small>{t("Apple Vision on the backend")}</small></div>
                    <div class="cloud-dash-card is-local" data-testid={format!("stats-local-{key}")}><span>{icon("shield")}{t("Local LLM")}</span>{bucket(&p.local)}</div>
                    <div class="cloud-dash-card is-cloud" data-testid={format!("stats-cloud-{key}")}><span>{icon("globe")}{t("Cloud AI")}</span>{bucket(&p.cloud)}<small>{t("tokens")}{" "}{tokens(p.cloud.input_tokens)}{" / "}{tokens(p.cloud.output_tokens)}{" · "}{t("est. cost")}{" "}{cost(&p.cloud)}</small></div>
                    <div class="cloud-dash-card"><span>{icon("check")}{t("Skipped unchanged screens")}</span><strong>{p.skipped_unchanged}</strong><small>{t("No model ran")}</small></div>
                </div></div>
            })}
            if !stats.price_date.is_empty() {<p class="cloud-price-note">{t("Estimated from list prices dated")}{" "}{&stats.price_date}{". "}{t("Not a provider bill.")}</p>}
        </section>}
    }
    fn ai_ready(&self, ctx: &Context<Self>) -> bool {
        let preferences = self.preferences(ctx);
        ctx.props().settings.demo
            || ctx
                .props()
                .settings
                .providers
                .iter()
                .any(|p| p.id == preferences.provider() && p.configured)
    }
    fn preferences(&self, ctx: &Context<Self>) -> UserAiPreferences {
        self.preferences
            .clone()
            .or_else(|| ctx.props().settings.user_ai.clone())
            .unwrap_or_else(|| crate::backend_default(&ctx.props().settings))
    }
    fn begin(&mut self) {
        self.busy = true;
        self.error.clear();
        self.revision += 1;
    }
    fn online(&self, ctx: &Context<Self>, id: &str) -> bool {
        ctx.props()
            .clients
            .iter()
            .any(|c| c.id == id && c.online && c.can_capture)
    }
    fn capture_controls(&self, ctx: &Context<Self>, session: &InvisibleSession) -> Html {
        let draft = self.capture_draft(ctx, session);
        let status = &session.capture;
        let ready = self.online(ctx, &draft.client) && self.ai_ready(ctx);
        let hint = if draft.client.is_empty() {
            Some("Choose one capture computer for this session.")
        } else if !self.online(ctx, &draft.client) {
            Some("Reconnect this capture computer to continue.")
        } else if !self.ai_ready(ctx) {
            Some("Enable the selected AI provider in Backend settings before starting.")
        } else {
            None
        };
        let id = &session.id;
        let action = if status.active {
            CaptureAction::Stop
        } else {
            CaptureAction::Start
        };
        let open = status.active || status.capturing || self.capture_open.contains(id);
        html! {<details class="cloud-capture" open={open} data-testid="manual-capture">
            <summary onclick={ctx.link().callback({let id=id.clone();move |e:MouseEvent|{e.prevent_default();Msg::CaptureOpen(id.clone())}})}>{icon("screen")}<span>{t("Manual capture (add-on)")}</span><small>{t(if status.active {"Polling"} else {"Off"})}</small></summary>
            <p>{t("Capture now saves one screenshot of the capture Mac to this chatroom. Unchanged screens are discarded before any AI runs.")}</p>
            if session.client_id.is_empty() {
                <label class="cloud-capture-device">{t("Capture computer")}
                    <select value={draft.client.clone()} disabled={self.busy} onchange={ctx.link().callback({let id=id.clone();move |e:Event|Msg::CaptureClient(id.clone(),e.target_unchecked_into::<HtmlSelectElement>().value())})}>
                        <option value="" selected={draft.client.is_empty()} disabled=true>{t("Choose a device")}</option>
                        {for ctx.props().clients.iter().filter(|c|c.can_capture).map(|c|html!{<option value={c.id.clone()} selected={c.id==draft.client} disabled={!c.online}>{format!("{} · {}",c.name,t(if c.online {"Connected"}else{"Offline"}))}</option>})}
                    </select>
                </label>
            }
            <div class="cloud-capture-actions">
                <button class="primary-button" disabled={self.busy||!ready||status.capture_queued||(!status.active&&status.capturing)} onclick={ctx.link().callback({let id=id.clone();move |_|Msg::Capture(id.clone(),CaptureAction::Now)})}>{icon("screen")}{t(if status.capture_queued {"Capture queued"}else{"Capture now"})}</button>
            </div>
            <div class="cloud-polling">
                <h5>{icon("clock")}{t("Optional polling")}</h5>
                <p>{t("Repeats Capture now on a timer while this session runs in the background.")}</p>
                <div class="cloud-capture-actions">
                    <label>{t("Pause between responses")}<span><input type="number" aria-label={t("Capture interval")} min="2" max="300" value={draft.interval.clone()} disabled={self.busy||status.active} oninput={ctx.link().callback({let id=id.clone();move |e:InputEvent|Msg::CaptureInterval(id.clone(),e.target_unchecked_into::<HtmlInputElement>().value())})}/>{t("seconds")}</span></label>
                    <button class="secondary-button" disabled={self.busy||(!status.active&&(!ready||status.capturing))} onclick={ctx.link().callback({let id=id.clone();move |_|Msg::Capture(id.clone(),action)})}>{icon(if status.active {"stop"}else{"clock"})}{t(if status.active {"Stop polling"}else{"Start polling"})}</button>
                </div>
            </div>
            if status.active||status.capturing {<div class="cloud-capture-status" role="status"><span>{t(if status.capture_queued {"Capture queued"}else if status.capturing {"Capturing your screen"}else{"Continuous capture is running"})}</span>
                if let Some(next)=&status.next_capture_at {<span>{t("Next capture in")}{" "}{((js_sys::Date::parse(next)-js_sys::Date::now())/1000.0).ceil().max(0.0) as u64}{"s"}</span>}
                if status.unchanged_count>0 {<span>{status.unchanged_count}{" · "}{t("Unchanged screenshots discarded")}</span>}
            </div>}
            if let Some(hint)=hint {<p class="cloud-capture-hint">{t(hint)}</p>}
            if let Some(error)=&status.error {<p class="notes-error" role="alert">{t(error)}</p>}
        </details>}
    }
    /// The session's main job: watch one TextEdit note and answer below it.
    fn autonomous(
        &self,
        ctx: &Context<Self>,
        session: &InvisibleSession,
        enabled: bool,
        client: Option<&CaptureClient>,
    ) -> Html {
        let status = &session.status;
        let settings = &ctx.props().settings;
        let ai_ready = self.ai_ready(ctx);
        let start_hint = if status.active {
            None
        } else if !enabled {
            Some("TextEdit assistance is disabled. Enable it to start a session.")
        } else if let Some(hint) = crate::invisible::textedit_hint(client) {
            Some(hint)
        } else if !ai_ready {
            Some("Enable the selected AI provider in Backend settings before starting.")
        } else if self
            .sessions
            .iter()
            .any(|s| s.id != session.id && s.client_id == session.client_id && s.status.active)
        {
            Some("Another session is running on this Mac. Stop it first.")
        } else {
            None
        };
        let id = session.id.clone();
        let active = status.active;
        let label = if status.approval.is_some() {
            "Awaiting approval"
        } else if active {
            "Running"
        } else {
            "Stopped"
        };
        // Cloud AI polls on the slow admin interval; every other mode polls with the
        // on-device model on the short one.
        let cloud_mode = self.preferences(ctx).mode == "cloudai";
        let admin_default = if cloud_mode {
            settings.local_ai.auto_interval_seconds.max(60)
        } else {
            settings.local_ai.local_interval_seconds
        };
        // Cloud AI mode never polls more often than once a minute.
        let effective = if status.interval_seconds > 0 {
            status.interval_seconds
        } else {
            admin_default
        };
        let effective = if cloud_mode {
            effective.max(60)
        } else {
            effective
        };
        let app = causewaybay_shared::note_app(&status.app);
        let connecting_for = (status.active && status.stage.starts_with("Connecting to "))
            .then(|| {
                ((js_sys::Date::now() - js_sys::Date::parse(&session.updated_at)) / 1000.0)
                    .floor()
                    .max(0.0) as u64
            })
            .filter(|_| !session.updated_at.is_empty());
        let pause = self
            .assistance
            .as_ref()
            .map(|p| (p.textedit_settle_ms as f64 / 1000.0).to_string())
            .unwrap_or_else(|| "1.2".into());
        let style = self
            .assistance
            .as_ref()
            .map(|p| p.textedit_style.clone())
            .unwrap_or_else(|| "concise".into());
        html! {<section class="cloud-autonomous" aria-label={t("Autonomous AI run")} data-testid="autonomous-ai">
            <div class="cloud-section-heading"><h4>{icon("spark")}{t("Autonomous AI run")}</h4><span class={classes!("cloud-section-state",active.then_some("is-active"),status.approval.is_some().then_some("needs-approval"))} data-testid="autonomous-state"><i/>{t(label)}</span></div>
            <p class="cloud-section-copy">{t(match app {
                "auto" => "I’ll read the note you’re using in TextEdit or Notes and add a reply below it.",
                "terminal" => "In Terminal, type ai- followed by your question. I’ll show an answer without running commands.",
                "notes" => "I’ll read your selected Apple note and add a reply below it.",
                "none" => "Send a request here. I won’t read your notes.",
                _ => "Write in TextEdit. I’ll read your note and add a reply below it.",
            })}</p>
            <div class="cloud-session-detail"><span>{icon("file")}{if !status.document.is_empty(){status.document.clone()}else{t(match app {"notes"=>"Apple Notes","terminal"=>"Terminal","none"=>"Remote prompts only","auto"=>"TextEdit, Apple Notes or Terminal",_=>"TextEdit notes"})}}</span><p role="status">{t(&status.stage)}if let Some(seconds)=connecting_for {<span class="cloud-connecting">{" · "}{seconds}{"s"}</span>}</p>
            if connecting_for.is_some_and(|s|s>=8) {<p class="cloud-capture-hint">{t(if app=="notes"||status.resolved_app=="notes" {"Notes is slow to respond. Close any dialog in Notes on the capture Mac; the connection gives up after 30 seconds."} else {"TextEdit is slow to respond. Close any dialog in TextEdit on the capture Mac; the connection gives up after 30 seconds."})}</p>}</div>
            if let Some(error)=&status.error {<p class="notes-error" role="alert">{t(error)}</p>}
            if let Some(approval)=&status.approval {<section class="cloud-approval" aria-label={t("Cloud approval")}><div class="eyebrow">{icon("shield")}{t("Awaiting approval")}</div><h3>{t("Let cloud AI help with this note?")}</h3><p>{&approval.reason}</p><p>{t("Only this note will be sent, once, to:")}{" "}<strong>{&approval.provider}</strong></p><pre>{&approval.text}</pre><div><button class="secondary-button" disabled={self.busy} onclick={ctx.link().callback({let session=id.clone();let id=approval.id.clone();move |_|Msg::Decide(session.clone(),id.clone(),false)})}>{t("Keep it local")}</button><button class="primary-button" disabled={self.busy} onclick={ctx.link().callback({let session=id.clone();let id=approval.id.clone();move |_|Msg::Decide(session.clone(),id.clone(),true)})}>{t("Approve once")}</button></div></section>}
            <div class="cloud-settings" role="group" aria-label={t("Autonomous AI settings")}>
                <label>{t("Watched app")}<select aria-label={t("Watched app")} value={app.to_string()} disabled={self.busy||active} onchange={ctx.link().callback({let id=id.clone();move |e:Event|Msg::SetApp(id.clone(),e.target_unchecked_into::<HtmlSelectElement>().value())})}>{for app_options().into_iter().map(|(value,label)|html!{<option value={value} selected={app==value}>{t(label)}</option>})}</select><small>{t("Automatic follows the front app; change it while stopped.")}</small></label>
                <details class="helper-options"><summary>{t("More options")}</summary><label class="cloud-check"><span><input type="checkbox" aria-label={t("Read from the screen")} checked={status.read_screen} disabled={self.busy} onchange={ctx.link().callback({let id=id.clone();move |e:Event|Msg::ReadScreen(id.clone(),e.target_unchecked_into::<HtmlInputElement>().checked())})}/>{t("Read from the screen")}</span><small>{t("A screenshot of the Mac accompanies each new note or request: read locally with Apple Vision for local AI, sent as an image to cloud AI.")}</small></label>
                <label>{t("Checks every")}<span><input type="number" aria-label={t("Check interval")} min="2" max="300" value={effective.to_string()} disabled={self.busy} onchange={ctx.link().callback({let id=id.clone();move |e:Event|Msg::SetInterval(id.clone(),e.target_unchecked_into::<HtmlInputElement>().value())})}/>{t("seconds")}</span>
                    <small>if cloud_mode {{t("Cloud AI mode: at least 60 seconds, and an unchanged screen never reaches cloud AI.")}} else if status.interval_seconds>0 {<button type="button" class="text-button" disabled={self.busy} onclick={ctx.link().callback({let id=id.clone();move |_|Msg::SetInterval(id.clone(),"0".into())})}>{t("Use admin default")}{" · "}{admin_default}{"s"}</button>} else {{t("Admin default · this session only when changed")}}</small></label>
                <label>{t("Typing pause")}<span><input type="number" aria-label={t("Typing pause")} min="0.5" max="10" step="0.1" value={pause} disabled={self.busy||self.assistance.is_none()} onchange={ctx.link().callback(|e:Event|Msg::Assist("textedit_settle_ms",e.target_unchecked_into::<HtmlInputElement>().value()))}/>{t("seconds")}</span><small>{t("Wait after typing stops before answering")}</small></label>
                <label>{t("Answer length")}<select aria-label={t("Answer length")} value={style.clone()} disabled={self.busy||self.assistance.is_none()} onchange={ctx.link().callback(|e:Event|Msg::Assist("textedit_style",e.target_unchecked_into::<HtmlSelectElement>().value()))}><option value="concise" selected={style=="concise"}>{t("Short and practical")}</option><option value="detailed" selected={style=="detailed"}>{t("Detailed with examples")}</option></select><small>{t("Typing pause and answer length apply to all your sessions; changing them stops a running session.")}</small></label>
                </details>
            </div>
            if status.read_screen && active {<p class="helper-screen-notice">{icon("screen")}{t("Screen sharing is on")}</p>}
            if let Some(hint)=start_hint {<p class="cloud-start-hint" id={format!("start-hint-{}",session.id)}>{t(hint)}</p>if client.is_none_or(|c| !ready_mac(c)){<a class="text-button" href="/devices#device-setup">{t("Set up this Mac")}{icon("arrow")}</a>} }
            <div class="cloud-session-footer"><span>{icon("clock")}{t("Checks every")}{" "}{effective}{"s"}</span><button class={classes!("cloud-auto-button",if active {"secondary-button"}else{"primary-button"})} data-testid="autonomous-toggle" disabled={self.busy||start_hint.is_some()} aria-describedby={start_hint.map(|_|format!("start-hint-{}",session.id))} onclick={ctx.link().callback({let id=id.clone();move |_|Msg::Toggle(id.clone(),active)})}>{icon(if active {"stop"}else{"bolt"})}{t(if active {"Stop helping"}else{"Start helping"})}</button></div>
            if active {{self.remote_control(ctx, session)}}
        </section>}
    }
    /// Requests typed here (on an iPhone or iPad) run on the Mac through the backend.
    fn remote_control(&self, ctx: &Context<Self>, session: &InvisibleSession) -> Html {
        let status = &session.status;
        let id = session.id.clone();
        let draft = self.prompt_drafts.get(&id).cloned().unwrap_or_default();
        let active = status.active;
        let can_send = active && !self.busy && !draft.trim().is_empty() && draft.len() <= 4000;
        let chips: [(&str, &str, &str); 3] = [
            ("globe", "Open Safari", "open "),
            ("search", "Search the web", "search "),
            ("bolt", "Calculate", "calc "),
        ];
        html! {<section class="cloud-remote" aria-label={t("Remote control")} data-testid="remote-control">
            <h5>{icon("phone")}{t("Remote control")}<small>{t("From this device to the Mac")}</small></h5>
            <p>{t("Type a request here. The Mac opens Safari, searches the web, calculates, or answers in chat and in the watched note.")}</p>
            <div class="cloud-remote-chips" role="group" aria-label={t("Quick requests")}>
                {for chips.into_iter().map(|(symbol,label,prefix)|html!{<button type="button" class="chip-button" disabled={self.busy||!active} onclick={ctx.link().callback({let id=id.clone();move |_|Msg::QuickPrompt(id.clone(),prefix)})}>{icon(symbol)}{t(label)}</button>})}
            </div>
            <form class="cloud-remote-form" onsubmit={ctx.link().callback({let id=id.clone();move |e:SubmitEvent|{e.prevent_default();Msg::SendPrompt(id.clone())}})}>
                <textarea aria-label={t("Request for the Mac")} rows="2" maxlength="4000" placeholder={t("For example: open youtube.com · search bitcoin price · calc 12*34 · explain this")} value={draft.clone()} disabled={self.busy||!active} oninput={ctx.link().callback({let id=id.clone();move |e:InputEvent|Msg::PromptDraft(id.clone(),e.target_unchecked_into::<HtmlTextAreaElement>().value())})} onkeydown={ctx.link().batch_callback({let id=id.clone();move |e:KeyboardEvent|{if e.key()=="Enter"&&!e.shift_key(){e.prevent_default();Some(Msg::SendPrompt(id.clone()))}else{None}}})}/>
                <button type="submit" class="primary-button" data-testid="send-remote-prompt" disabled={!can_send}>{icon("send")}{t("Send to Mac")}</button>
            </form>
            if !active {<p class="cloud-capture-hint">{t("Start the autonomous AI run to send requests from this device.")}</p>}
            if status.queued_prompts>0 {<p class="cloud-remote-queue" role="status">{status.queued_prompts}{" · "}{t("requests waiting")}</p>}
            if !status.last_prompt.is_empty() {<div class="cloud-remote-last" data-testid="remote-last">
                <div><span>{t("Last request")}</span><p>{&status.last_prompt}</p></div>
                if !status.last_action.is_empty() {<div><span>{t("Mac action")}</span><p data-testid="remote-action">{&status.last_action}</p></div>}
                if !status.last_reply.is_empty() && status.last_reply!=status.last_action {<div><span>{t("Reply")}</span><p data-testid="remote-reply">{&status.last_reply}</p></div>}
            </div>}
        </section>}
    }
    fn session(&self, ctx: &Context<Self>, session: &InvisibleSession, enabled: bool) -> Html {
        let status = &session.status;
        let client = ctx
            .props()
            .clients
            .iter()
            .find(|c| c.id == session.client_id);
        let online = client.is_some_and(|c| c.online);
        let device = client
            .map(|c| c.name.as_str())
            .unwrap_or(if session.client_id.is_empty() {
                "—"
            } else {
                &session.client_id
            });
        let label = if status.approval.is_some() {
            "Awaiting approval"
        } else if status.active || session.capture.active || session.capture.capturing {
            "Running"
        } else {
            "Stopped"
        };
        html! {<article key={session.id.clone()} class="cloud-session" data-session-id={session.id.clone()} aria-label={session.title.clone()}>
            <div class="cloud-session-heading"><div class="cloud-session-name"><span class="cloud-device-icon">{icon("monitor")}</span><div><h3>{&session.title}</h3><p>{device}<span>{" · "}{t(if online {"Connected"} else {"Offline"})}</span></p></div></div><span class={classes!("cloud-session-state",(status.active||session.capture.active||session.capture.capturing).then_some("is-active"),status.approval.is_some().then_some("needs-approval"))}><i/>{t(label)}</span></div>
            <p class="cloud-session-history">{session.capture_count}{" · "}{t("Captures")}{" / "}{session.memo_count}{" · "}{t("Memos")}</p>
            if !session.recent_captures.is_empty() {<div class="cloud-capture-thumbnails">{for session.recent_captures.iter().map(|id|html!{<a href={format!("/chat?room={}",session.id)} aria-label={t("Open chat history")}><img src={format!("/api/memos/{id}/thumbnail")} alt={t("Captured screen")} loading="lazy" decoding="async"/></a>})}</div>}
            <a class="secondary-button cloud-open-chat" href={format!("/chat?room={}",session.id)}>{icon("message")}{t("Open chat history")}</a>
            {self.autonomous(ctx, session, enabled, client)}
            {self.activity(ctx, session)}
            {self.capture_controls(ctx, session)}
        </article>}
    }
}
