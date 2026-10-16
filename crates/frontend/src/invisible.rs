//! The quiet, opt-in notes workspace. All policy and Auto state come from the backend.
use crate::{
    api::api,
    i18n::{t, tf},
    icons::icon,
};
use causewaybay_shared::{
    AssistancePreferences, CaptureClient, CreateInvisibleSession, InvisibleDecision,
    InvisibleSession, InvisibleStatus, Settings, TextEditAvailability, UserAiPreferences,
};
use gloo_timers::callback::Interval;
use serde_json::{Value, json};
use web_sys::HtmlSelectElement;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: crate::i18n::Language,
    pub settings: Settings,
    pub client: String,
    pub room: String,
    pub choose_room: Callback<String>,
    pub clients: Vec<CaptureClient>,
    pub saved: Callback<()>,
    pub setup: Callback<()>,
    pub navigate: Callback<String>,
    pub admin: bool,
}
pub struct Workspace {
    status: InvisibleStatus,
    textedit_enabled: Option<bool>,
    sessions: Vec<InvisibleSession>,
    selected: String,
    revision: u64,
    polling: bool,
    busy: bool,
    error: String,
    /// Once a run or an approval has opened the panel, it stays open when the
    /// run ends: stopping Auto must not fold the controls away. The reader
    /// can still fold it by hand, which Yew leaves alone.
    shown_open: std::cell::Cell<bool>,
    _poll: Interval,
}
pub enum Msg {
    Poll,
    Sessions(
        u64,
        Result<(Vec<InvisibleSession>, AssistancePreferences), String>,
    ),
    Toggle,
    Decision(String, bool),
    Done(u64, Box<Result<InvisibleSession, String>>),
}
fn empty_status() -> InvisibleStatus {
    InvisibleStatus {
        textedit_enabled: true,
        ..Default::default()
    }
}
pub fn textedit_hint(client: Option<&CaptureClient>) -> Option<&'static str> {
    use TextEditAvailability::*;
    Some(match client.map(|c| c.textedit) {
        Some(Ready) => return None,
        Some(NeedsEnrollment) => {
            "This Mac is shared for screen capture. Connect it to your account to enable TextEdit replies."
        }
        Some(Offline) => "Reconnect this capture Mac to start the session.",
        Some(UpdateRequired) => "Reconnect the updated Mac CLI to use TextEdit Auto",
        _ => "Choose a connected Mac and an enabled AI provider to start.",
    })
}
fn preferences(s: &Settings) -> UserAiPreferences {
    s.user_ai
        .clone()
        .unwrap_or_else(|| crate::backend_default(s))
}
impl Component for Workspace {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        let link = ctx.link().clone();
        link.send_message(Msg::Poll);
        Self {
            status: empty_status(),
            textedit_enabled: None,
            sessions: vec![],
            selected: ctx.props().room.clone(),
            revision: 0,
            polling: false,
            busy: false,
            error: String::new(),
            shown_open: std::cell::Cell::new(false),
            _poll: Interval::new(2000, move || link.send_message(Msg::Poll)),
        }
    }
    fn changed(&mut self, ctx: &Context<Self>, old: &Props) -> bool {
        if ctx.props().room != old.room {
            self.revision += 1;
            self.busy = false;
            self.selected = ctx.props().room.clone();
            self.status = empty_status();
            self.error.clear();
            self.choose_session(ctx);
            ctx.link().send_message(Msg::Poll);
        }
        true
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Poll => {
                if !self.busy && !self.polling {
                    self.polling = true;
                    let revision = self.revision;
                    ctx.link().send_future(async move {
                        Msg::Sessions(
                            revision,
                            futures_util::try_join!(
                                api("GET", "/api/invisible/sessions", None),
                                api("GET", "/api/assistance/preferences", None)
                            ),
                        )
                    });
                }
                return false;
            }
            Msg::Sessions(revision, result) => {
                self.polling = false;
                if revision != self.revision || self.busy {
                    return false;
                }
                match result {
                    Ok((sessions, assistance)) => {
                        // Permission is account-wide, including before the first
                        // session exists. An empty session list carries no policy.
                        self.textedit_enabled = Some(assistance.textedit_enabled);
                        self.sessions = sessions;
                        self.choose_session(ctx);
                        if crate::api::is_connection_error(&self.error) {
                            self.error.clear();
                        }
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Toggle => {
                if self.busy || self.selected.is_empty() {
                    return false;
                }
                self.busy = true;
                self.error.clear();
                self.revision += 1;
                let revision = self.revision;
                let selected = self.selected.clone();
                let active = self.status.active;
                let client = if self.status.client_id.is_empty() {
                    ctx.props().client.clone()
                } else {
                    self.status.client_id.clone()
                };
                ctx.link().send_future(async move {
                    let result = async {
                        let id = if !active {
                            let session: InvisibleSession = api(
                                "POST",
                                "/api/invisible/sessions",
                                Some(json!(CreateInvisibleSession {
                                    client_id: client,
                                    title: None,
                                    room_id: Some(selected.clone()),
                                    start: false,
                                    app: None
                                })),
                            )
                            .await?;
                            session.id
                        } else {
                            selected
                        };
                        api(
                            "POST",
                            &format!(
                                "/api/invisible/sessions/{id}/{}",
                                if active { "stop" } else { "start" }
                            ),
                            None,
                        )
                        .await
                    }
                    .await;
                    Msg::Done(revision, Box::new(result))
                });
            }
            Msg::Decision(id, approve) => {
                if self.busy || self.selected.is_empty() {
                    return false;
                }
                self.busy = true;
                self.error.clear();
                self.revision += 1;
                let revision = self.revision;
                let session = self.selected.clone();
                ctx.link().send_future(async move {
                    Msg::Done(
                        revision,
                        Box::new(
                            api(
                                "POST",
                                &format!("/api/invisible/sessions/{session}/approval"),
                                Some(json!(InvisibleDecision { id, approve })),
                            )
                            .await,
                        ),
                    )
                });
            }
            Msg::Done(revision, result) => {
                if revision != self.revision {
                    return false;
                }
                self.busy = false;
                match *result {
                    Ok(session) => {
                        self.textedit_enabled = Some(session.status.textedit_enabled);
                        self.selected = session.id.clone();
                        self.status = session.status.clone();
                        self.sessions.retain(|s| s.id != self.selected);
                        self.sessions.insert(0, session);
                    }
                    Err(e) => self.error = e,
                }
                ctx.link().send_message(Msg::Poll);
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let p = ctx.props();
        let prefs = preferences(&p.settings);
        let active = self.status.active;
        let target = if self.status.client_id.is_empty() {
            &p.client
        } else {
            &self.status.client_id
        };
        let device = p.clients.iter().find(|c| c.id == *target);
        let ai_ready = p.settings.demo
            || p.settings
                .providers
                .iter()
                .any(|provider| provider.id == prefs.provider() && provider.configured);
        let start_hint = if active {
            None
        } else if self.textedit_enabled == Some(false) {
            Some("TextEdit assistance is disabled. Enable it in assistance settings to use Auto.")
        } else if self.textedit_enabled.is_none() || p.room.is_empty() {
            Some("Loading…")
        } else if let Some(hint) = textedit_hint(device) {
            Some(hint)
        } else if !ai_ready {
            Some("Enable the selected AI provider in Backend settings before starting.")
        } else if self
            .sessions
            .iter()
            .any(|s| s.id != self.selected && s.client_id == *target && s.status.active)
        {
            Some("Another session is running on this Mac. Stop it first.")
        } else {
            None
        };
        let device_name = |id: &str| {
            p.clients
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| id.to_owned())
        };
        let mode_label = match prefs.mode.as_str() {
            "ondevice" => "On-device",
            "hybrid" => "Hybrid",
            "codex" => "Codex",
            "claude_code" => "Claude Code",
            _ => "Cloud AI",
        };
        let auto_label = if active {
            "Stop autonomous AI"
        } else {
            "Start autonomous AI"
        };
        if active || self.status.approval.is_some() {
            self.shown_open.set(true);
        }
        html! {<details class="everyday-workspace" open={self.shown_open.get()}><summary>{icon("spark")}<span>{t("Your Mac & AI")}</span><small>{t(mode_label)}{" · "}{t(if self.status.approval.is_some() {"Approval needed"} else if active {"Running"} else {"Ready when you are"})}</small></summary><section class="invisible-workspace" aria-label={t("Invisible AI workspace")}>
            <div class="workspace-ai-settings" data-testid="processing-settings">
                <span class="processing-symbol">{icon(crate::icons::mode_icon(&prefs.mode))}</span><span class="processing-summary"><small>{t("AI processing location")}</small><strong>{t(mode_label)}<span class="selection-check" aria-hidden="true">{icon("check")}</span></strong><span>{t(match prefs.mode.as_str(){"ondevice"=>"Processing stays on your backend Mac","hybrid"=>"Local first. Cloud help follows your settings.",_=>"Cloud AI · your selected provider"})}</span></span><button type="button" class="processing-change" onclick={p.navigate.reform(|_|"autonomous".to_string())}>{t("Change")}{icon("arrow")}</button>
            </div>
            <div class="invisible-console" aria-labelledby="textedit-auto-heading">
                <div class="invisible-control-heading"><div class="notes-heading-copy"><span class="eyebrow">{t("REPLIES IN YOUR NOTE")}</span><div class="notes-title-row"><h2 id="textedit-auto-heading">{t("Autonomous AI run")}</h2><span class={classes!("auto-state",active.then_some("is-active"))}>if active {<span class="auto-check" aria-hidden="true">{icon("check")}</span>}else{<i aria-hidden="true"/>}{t(if active{"Running"}else{"Off"})}</span></div></div><button class={classes!("auto-button",active.then_some("is-active"))} data-testid="textedit-auto-toggle" aria-label={t(auto_label)} aria-pressed={active.to_string()} disabled={self.busy||start_hint.is_some()} aria-describedby={start_hint.map(|_|"notes-start-hint")} onclick={ctx.link().callback(|_|Msg::Toggle)}>{icon(if active{"stop"}else{"spark"})}<strong>{t(auto_label)}</strong></button></div>
                <p class="notes-target">{icon("monitor")}{if target.is_empty(){t("Choose a connected Mac")}else{tf("TextEdit on {device}", &[("device",device_name(target))])}}</p>

                if let Some(hint)=start_hint {<div class="notes-hint notes-start-hint" id="notes-start-hint"><p>{t(hint)}</p>if device.is_none_or(|c| c.textedit != TextEditAvailability::Ready){<a class="text-button" href="/devices#device-setup">{t("Set up this Mac")}{icon("arrow")}</a>}else if p.admin && !ai_ready{<button class="text-button" onclick={p.setup.reform(|_|())}>{t("Set up AI")}{icon("arrow")}</button>}</div>}
                if active {<div class="invisible-status" role="status"><span>{icon("file")}{if self.status.document.is_empty(){t("Answers appear in TextEdit")}else{self.status.document.clone()}}</span><span>{tf("Checks TextEdit every {seconds}s", &[("seconds", p.settings.local_ai.auto_interval_seconds.to_string())])}</span></div>}
                if !self.selected.is_empty() {<p class="notes-progress">{t(&self.status.stage)}</p>}

                if !self.sessions.is_empty() {<div class="invisible-sessions"><label for="invisible-session">{t("Session")}<select id="invisible-session" aria-label={t("Invisible AI session")} value={self.selected.clone()} disabled={self.busy||p.room.is_empty()} onchange={p.choose_room.reform(|e:Event|e.target_unchecked_into::<HtmlSelectElement>().value())}><option value="" selected={self.selected.is_empty()} disabled=true>{t("Choose a session")}</option>{for self.sessions.iter().map(|s|html!{<option value={s.id.clone()} selected={s.id==self.selected}>{format!("{} · {} · {}",&s.title,device_name(&s.client_id),t(if s.status.active{"Running"}else{"Stopped"}))}</option>})}</select></label></div>}
                <p class="notes-hint">{if active{t("Keeps running when this browser closes. Use Stop to end automatic replies.")}else if self.selected.is_empty(){t("Start creates a session automatically. Screen capture works without a session.")}else{t("This session is stopped. Start it to resume replies in TextEdit.")}}</p>
                <div class="notes-links"><a href="/autonomous">{t("Manage sessions")}{icon("arrow")}</a><a href="/assistance">{t("Customize Wiki & TextEdit")}</a></div>
            if let Some(approval)=&self.status.approval {<section class="cloud-approval" aria-label={t("Cloud approval")}><div class="eyebrow">{icon("shield")}{t("YOUR PERMISSION FIRST")} </div><h3>{t("Let cloud AI help with this note?")} </h3><p>{&approval.reason}</p><p>{tf("Only the note below will be sent to {provider}. Approving allows this request once.", &[("provider", approval.provider.clone())])}</p><pre>{&approval.text}</pre><div><button class="secondary-button" disabled={self.busy} onclick={ctx.link().callback({let id=approval.id.clone();move |_|Msg::Decision(id.clone(),false)})}>{t("Keep it local")} </button><button class="primary-button" disabled={self.busy} onclick={ctx.link().callback({let id=approval.id.clone();move |_|Msg::Decision(id.clone(),true)})}>{t("Approve once")} </button></div></section>}
            if !self.error.is_empty() {<p class="notes-error" role="alert">{t(&self.error)}</p>}else if let Some(e)=self.status.error.as_deref(){<p class="notes-error" role="alert">{t(e)}</p>}
            </div>
        </section></details>}
    }
}

impl Workspace {
    fn choose_session(&mut self, ctx: &Context<Self>) {
        self.selected = ctx.props().room.clone();
        self.status = self
            .sessions
            .iter()
            .find(|s| s.id == self.selected)
            .map(|s| s.status.clone())
            .unwrap_or_else(empty_status);
    }
}

pub struct Usage {
    value: Value,
    _poll: Interval,
}
pub enum UsageMsg {
    Poll,
    Loaded(Result<Value, String>),
}
impl Component for Usage {
    type Message = UsageMsg;
    type Properties = crate::i18n::Props;
    fn create(ctx: &Context<Self>) -> Self {
        let link = ctx.link().clone();
        link.send_message(UsageMsg::Poll);
        Self {
            value: json!({}),
            _poll: Interval::new(5000, move || link.send_message(UsageMsg::Poll)),
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: UsageMsg) -> bool {
        match msg {
            UsageMsg::Poll => {
                ctx.link()
                    .send_future(async { UsageMsg::Loaded(api("GET", "/api/usage", None).await) });
                false
            }
            UsageMsg::Loaded(r) => {
                if let Ok(v) = r {
                    self.value = v;
                }
                true
            }
        }
    }
    fn view(&self, _: &Context<Self>) -> Html {
        let mut millis = 0;
        let mut tokens = 0;
        let mut unreported = 0;
        let mut has_cloud = false;
        for p in self.value["providers"].as_array().into_iter().flatten() {
            if matches!(p["provider"].as_str(), Some("apple" | "mlx")) {
                millis += p["elapsed_ms"].as_u64().unwrap_or(0);
            } else {
                has_cloud = true;
                tokens += p["input_tokens"].as_u64().unwrap_or(0)
                    + p["output_tokens"].as_u64().unwrap_or(0);
                unreported += p["unreported_requests"].as_u64().unwrap_or(0);
            }
        }
        html! {<details class="usage-strip"><summary aria-label={t("AI usage")}><span>{icon("spark")}{t("AI usage")} </span><span>{if self.value["demo"]==true{t("Demo · no AI charges")}else{tf("Local {seconds}s · Cloud {tokens}", &[("seconds", format!("{:.1}",millis as f64/1000.)), ("tokens", if has_cloud{tf("{count} reported tokens", &[("count", tokens.to_string())])}else{"—".into()})])}}</span><span class="usage-credit">{t("Credit: unavailable")} </span></summary><p>{t("Usage recorded by this workspace. Local time includes prompt processing and generation; Apple does not report token counts. Cloud tokens are provider-reported. ")} {if unreported>0{tf("{count} cloud requests did not report complete token counts. ", &[("count",unreported.to_string())])}else{String::new()}}{t("Provider credit balances and estimated USD savings are not connected yet.")} </p></details>}
    }
}
