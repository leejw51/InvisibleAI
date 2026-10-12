//! Coding: one folder per project, in Rust, Go, Python, C, Lua or TypeScript.
//! Describe what the program should do; the configured AI writes it, the
//! backend builds and runs it in its sandbox, and the output lands here. Files
//! can be read and edited in place, the program run again with any input, and
//! a finished program published as a skill the chat can run.
use crate::{
    api::{api, encode},
    i18n::{Language, t, tf},
    icons::icon,
};
use crate::{motion::FadeSwap, tween::ProgressBar};
use causewaybay_shared::{Memo, StreamEvent};
use gloo_timers::callback::Interval;
use serde_json::{Value, json};
use web_sys::{HtmlElement, HtmlInputElement, HtmlTextAreaElement};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub owner: String,
    pub settings: causewaybay_shared::Settings,
    pub ai_saved: Callback<()>,
    pub copy: Callback<String>,
}
pub struct Coding {
    overview: Value,
    /// The AI pick from the shared picker: what `ask` sends.
    provider: String,
    model: String,
    open: Option<String>,
    detail: Value,
    error: String,
    new_language: String,
    new_name: String,
    creating: bool,
    confirm_delete: bool,
    file: String,
    text: String,
    dirty: bool,
    saving: bool,
    prompt: String,
    /// The request being worked on, its live stage and workspace events.
    job: Option<String>,
    stage: String,
    events: Vec<StreamEvent>,
    answer: String,
    job_error: String,
    /// The agent's task list for the running request.
    todos: Vec<Value>,
    /// What the model is writing right now (the harness protocol, raw), so a
    /// slow on-device model shows progress; cleared at each new stage.
    draft: String,
    draft_chars: usize,
    /// Progress bookkeeping for the live bar: round and phase from the stage
    /// events, steps for the tool loop and the CLI tools, and the clock.
    round: usize,
    rounds: usize,
    steps: usize,
    phase: Phase,
    started: f64,
    draft_started: f64,
    tick: Option<Interval>,
    draft_node: NodeRef,
    stdin: String,
    args: String,
    running: bool,
    run: Option<Value>,
    testing: bool,
    tests: Option<Value>,
    suggesting: bool,
    skill_name: String,
    skill_description: String,
    publishing: bool,
}
/// Where a request is inside a round, for the bar and the stepper.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Writing,
    Building,
    Testing,
    Verifying,
    Done,
}
pub enum Msg {
    Tick,
    Load,
    Loaded(Result<Value, String>),
    Open(String),
    Detail(Result<Value, String>),
    Close,
    NewLanguage(String),
    NewName(String),
    Create,
    Created(Result<Value, String>),
    Delete,
    Deleted(Result<Value, String>),
    PickFile(String),
    FileLoaded(String, Result<Value, String>),
    Edit(String),
    Save,
    Saved(Result<Value, String>),
    Prompt(String),
    Ask,
    Asked(Result<Memo, String>),
    Event(StreamEvent),
    Stdin(String),
    Args(String),
    Suggest,
    Suggested(Result<Value, String>),
    Run,
    Ran(Result<Value, String>),
    Test,
    Tested(Result<Value, String>),
    Stop,
    Stopped(Result<Value, String>),
    SkillName(String),
    SkillDescription(String),
    Publish,
    Retire,
    Patched(Result<Value, String>),
    AiChosen(String, String),
    Revert(String),
    Reverted(Result<Value, String>),
    ClearError,
}
/// The project named in the address bar, when the page opened on one.
fn location_project() -> Option<String> {
    let path = web_sys::window()?.location().pathname().ok()?;
    let id = path.strip_prefix("/coding/")?.trim_end_matches('/');
    (id.len() == 36).then(|| id.to_owned())
}
impl Component for Coding {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        if let Some(id) = location_project() {
            ctx.link().send_message(Msg::Open(id));
        }
        let (provider, model) = crate::ai_settings::ai_choice(
            &ctx.props().settings,
            &crate::ai_settings::preferences(&ctx.props().owner),
        );
        Self {
            overview: Value::Null,
            provider,
            model,
            open: None,
            detail: Value::Null,
            error: String::new(),
            new_language: "python".into(),
            new_name: String::new(),
            creating: false,
            confirm_delete: false,
            file: String::new(),
            text: String::new(),
            dirty: false,
            saving: false,
            prompt: String::new(),
            job: None,
            stage: String::new(),
            events: vec![],
            answer: String::new(),
            job_error: String::new(),
            todos: vec![],
            draft: String::new(),
            draft_chars: 0,
            round: 0,
            rounds: 0,
            steps: 0,
            phase: Phase::Writing,
            started: 0.0,
            draft_started: 0.0,
            tick: None,
            draft_node: NodeRef::default(),
            stdin: String::new(),
            args: String::new(),
            running: false,
            run: None,
            testing: false,
            tests: None,
            suggesting: false,
            skill_name: String::new(),
            skill_description: String::new(),
            publishing: false,
        }
    }
    fn destroy(&mut self, _ctx: &Context<Self>) {
        if let Some(job) = &self.job {
            crate::api::unsubscribe(job);
        }
    }
    fn rendered(&mut self, _ctx: &Context<Self>, _first_render: bool) {
        // The draft panel follows what is being written.
        if let Some(pre) = self.draft_node.cast::<HtmlElement>() {
            pre.set_scroll_top(pre.scroll_height());
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Tick => self.job.is_some(),
            Msg::Load => {
                ctx.link()
                    .send_future(async { Msg::Loaded(api("GET", "/api/coding", None).await) });
                false
            }
            Msg::Loaded(r) => {
                match r {
                    Ok(v) => {
                        // Default to the first language this backend can actually run.
                        if self.overview.is_null()
                            && let Some(first) = v["languages"]
                                .as_array()
                                .and_then(|l| l.iter().find(|l| l["available"] == true))
                            && let Some(id) = first["id"].as_str()
                        {
                            self.new_language = id.to_owned();
                        }
                        self.overview = v;
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Open(id) => {
                self.open = Some(id.clone());
                self.detail = Value::Null;
                self.file.clear();
                self.text.clear();
                self.dirty = false;
                self.run = None;
                self.answer.clear();
                self.events.clear();
                self.stage.clear();
                self.job_error.clear();
                self.confirm_delete = false;
                // A prompt handed over by the Skills page ("Fix with the agent").
                if let Some(storage) =
                    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
                {
                    let key = format!("causewaybay.coding.prompt.{id}");
                    if let Ok(Some(prompt)) = storage.get_item(&key) {
                        self.prompt = prompt;
                        let _ = storage.remove_item(&key);
                    }
                }
                crate::push_location(format!("/coding/{id}"));
                let url = format!("/api/coding/projects/{}", encode(&id));
                ctx.link()
                    .send_future(async move { Msg::Detail(api("GET", &url, None).await) });
                true
            }
            Msg::Detail(r) => {
                match r {
                    Ok(v) => {
                        if let Some(k) = v["project"]["skill"].as_object() {
                            self.skill_name = k["name"].as_str().unwrap_or("").to_owned();
                            self.skill_description =
                                k["description"].as_str().unwrap_or("").to_owned();
                        } else if self.skill_name.is_empty() {
                            self.skill_name = v["project"]["name"]
                                .as_str()
                                .unwrap_or("")
                                .to_lowercase()
                                .chars()
                                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                                .take(40)
                                .collect();
                        }
                        // The folder's example run prefills the Run box.
                        if self.stdin.is_empty()
                            && let Some(stdin) = v["example"]["stdin"].as_str()
                        {
                            self.stdin = stdin.to_owned();
                        }
                        if self.run.is_none() && !v["project"]["last_run"].is_null() {
                            self.run = Some(v["project"]["last_run"].clone());
                        }
                        let entry = v["project"]["entry"].as_str().unwrap_or("").to_owned();
                        // A request already running (started from the Skills page, or
                        // from another device) shows live here too.
                        if self.job.is_none()
                            && let Some(m) = v["memos"].as_array().and_then(|m| m.first())
                            && m["status"] == "running"
                            && let Some(memo_id) = m["id"].as_str()
                        {
                            self.job = Some(memo_id.to_owned());
                            self.stage = t("Working…");
                            self.events.clear();
                            self.answer.clear();
                            self.started = js_sys::Date::now();
                            let link = ctx.link().clone();
                            self.tick =
                                Some(Interval::new(1000, move || link.send_message(Msg::Tick)));
                            let link = ctx.link().clone();
                            if let Err(e) = crate::api::subscribe(
                                memo_id,
                                Callback::from(move |e| link.send_message(Msg::Event(e))),
                            ) {
                                self.error = e;
                            }
                        }
                        self.detail = v;
                        if self.file.is_empty() && !entry.is_empty() {
                            ctx.link().send_message(Msg::PickFile(entry));
                        }
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Close => {
                self.open = None;
                self.detail = Value::Null;
                crate::push_location("/coding".into());
                ctx.link().send_message(Msg::Load);
                true
            }
            Msg::NewLanguage(l) => {
                self.new_language = l;
                true
            }
            Msg::NewName(n) => {
                self.new_name = n;
                false
            }
            Msg::Create => {
                if self.creating {
                    return false;
                }
                self.creating = true;
                let body = json!({"language":self.new_language,"name":self.new_name});
                ctx.link().send_future(async move {
                    Msg::Created(api("POST", "/api/coding/projects", Some(body)).await)
                });
                true
            }
            Msg::Created(r) => {
                self.creating = false;
                match r {
                    Ok(v) => {
                        self.new_name.clear();
                        if let Some(id) = v["id"].as_str() {
                            ctx.link().send_message(Msg::Open(id.to_owned()));
                        }
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Delete => {
                if !self.confirm_delete {
                    self.confirm_delete = true;
                    return true;
                }
                let Some(id) = self.open.clone() else {
                    return false;
                };
                let url = format!("/api/coding/projects/{}", encode(&id));
                ctx.link()
                    .send_future(async move { Msg::Deleted(api("DELETE", &url, None).await) });
                false
            }
            Msg::Deleted(r) => {
                match r {
                    Ok(_) => ctx.link().send_message(Msg::Close),
                    Err(e) => self.error = e,
                }
                self.confirm_delete = false;
                true
            }
            Msg::PickFile(path) => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                self.file = path.clone();
                self.dirty = false;
                let url = format!(
                    "/api/coding/projects/{}/file?path={}",
                    encode(&id),
                    encode(&path)
                );
                ctx.link().send_future(async move {
                    Msg::FileLoaded(path, api("GET", &url, None).await)
                });
                true
            }
            Msg::FileLoaded(path, r) => {
                // Typing that started before the file arrived wins over the file.
                if path != self.file || self.dirty {
                    return false;
                }
                match r {
                    Ok(v) => self.text = v["text"].as_str().unwrap_or("").to_owned(),
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Edit(text) => {
                self.text = text;
                let first = !self.dirty;
                self.dirty = true;
                // The Save button and the unsaved mark appear on the first keystroke.
                first
            }
            Msg::Save => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                if self.file.is_empty() || self.saving {
                    return false;
                }
                self.saving = true;
                let url = format!("/api/coding/projects/{}/file", encode(&id));
                let body = json!({"path":self.file,"text":self.text});
                ctx.link()
                    .send_future(async move { Msg::Saved(api("POST", &url, Some(body)).await) });
                true
            }
            Msg::Saved(r) => {
                self.saving = false;
                match r {
                    Ok(v) => {
                        self.dirty = false;
                        self.detail["files"] = v["files"].clone();
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Prompt(p) => {
                self.prompt = p;
                false
            }
            Msg::Ask => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                if self.job.is_some() || self.prompt.trim().is_empty() {
                    return false;
                }
                self.stage = t("Sending your request");
                self.events.clear();
                self.answer.clear();
                self.job_error.clear();
                self.todos.clear();
                self.draft.clear();
                self.draft_chars = 0;
                self.round = 0;
                self.rounds = 0;
                self.steps = 0;
                self.phase = Phase::Writing;
                self.started = js_sys::Date::now();
                let link = ctx.link().clone();
                self.tick = Some(Interval::new(1000, move || link.send_message(Msg::Tick)));
                self.job = Some(String::new());
                let url = format!("/api/coding/projects/{}/ask", encode(&id));
                let body =
                    json!({"prompt":self.prompt,"provider":self.provider,"model":self.model});
                ctx.link()
                    .send_future(async move { Msg::Asked(api("POST", &url, Some(body)).await) });
                true
            }
            Msg::Asked(r) => {
                match r {
                    Ok(memo) => {
                        self.prompt.clear();
                        self.job = Some(memo.id.clone());
                        let link = ctx.link().clone();
                        if let Err(e) = crate::api::subscribe(
                            &memo.id,
                            Callback::from(move |e| link.send_message(Msg::Event(e))),
                        ) {
                            self.error = e;
                        }
                    }
                    Err(e) => {
                        self.job = None;
                        self.stage.clear();
                        self.error = e;
                    }
                }
                true
            }
            Msg::Event(e) => {
                match e.kind.as_str() {
                    "stage" => {
                        self.note_stage(&e.text);
                        self.stage = e.text;
                        self.draft.clear();
                        self.draft_chars = 0;
                        self.draft_started = 0.0;
                    }
                    "draft" => {
                        if self.draft_started == 0.0 {
                            self.draft_started = js_sys::Date::now();
                        }
                        self.draft_chars += e.text.chars().count();
                        self.draft.push_str(&e.text);
                        // Keep the tail: enough to see what is being written.
                        if self.draft.len() > 4000 {
                            let cut = self.draft.len() - 3000;
                            let mut at = cut;
                            while !self.draft.is_char_boundary(at) {
                                at += 1;
                            }
                            self.draft = format!("…{}", &self.draft[at..]);
                        }
                    }
                    "progress" => {
                        if let Ok(p) = serde_json::from_str::<Value>(&e.text) {
                            self.stage = format!(
                                "{} {}",
                                p["label"].as_str().unwrap_or(""),
                                p["detail"].as_str().unwrap_or("")
                            );
                        }
                    }
                    "tool_start" | "tool_result" => self.events.push(e),
                    "todos" => self.todos = serde_json::from_str(&e.text).unwrap_or_default(),
                    "delta" => self.answer.push_str(&e.text),
                    "error" => {
                        self.job_error = e.text;
                        self.finish_job(ctx);
                    }
                    "done" => {
                        self.stage.clear();
                        self.finish_job(ctx);
                    }
                    _ => return false,
                }
                true
            }
            Msg::Stdin(s) => {
                self.stdin = s;
                false
            }
            Msg::Args(a) => {
                self.args = a;
                false
            }
            Msg::Suggest => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                if self.suggesting {
                    return false;
                }
                self.suggesting = true;
                let url = format!("/api/coding/projects/{}/example", encode(&id));
                let body = json!({"provider":self.provider,"model":self.model});
                ctx.link().send_future(async move {
                    Msg::Suggested(api("POST", &url, Some(body)).await)
                });
                true
            }
            Msg::Suggested(r) => {
                self.suggesting = false;
                match r {
                    Ok(v) => {
                        self.stdin = v["stdin"].as_str().unwrap_or("").to_owned();
                        self.args = v["args"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or_default();
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Run => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                if self.running {
                    return false;
                }
                self.running = true;
                let url = format!("/api/coding/projects/{}/run", encode(&id));
                let args: Vec<String> = self.args.split_whitespace().map(str::to_owned).collect();
                let body = json!({"stdin":self.stdin,"args":args});
                ctx.link()
                    .send_future(async move { Msg::Ran(api("POST", &url, Some(body)).await) });
                true
            }
            Msg::Ran(r) => {
                self.running = false;
                match r {
                    Ok(v) => self.run = Some(v),
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Test => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                if self.testing {
                    return false;
                }
                self.testing = true;
                let url = format!("/api/coding/projects/{}/test", encode(&id));
                ctx.link().send_future(async move {
                    Msg::Tested(api("POST", &url, Some(json!({}))).await)
                });
                true
            }
            Msg::Tested(r) => {
                self.testing = false;
                match r {
                    Ok(v) => self.tests = Some(v),
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Stop => {
                let Some(job) = self.job.clone().filter(|j| !j.is_empty()) else {
                    return false;
                };
                self.stage = t("Stopping…");
                let url = format!("/api/memos/{}/cancel", encode(&job));
                ctx.link().send_future(async move {
                    Msg::Stopped(api("POST", &url, Some(json!({}))).await)
                });
                true
            }
            Msg::Stopped(r) => {
                if let Err(e) = r {
                    self.error = e;
                }
                true
            }
            Msg::SkillName(v) => {
                self.skill_name = v;
                false
            }
            Msg::SkillDescription(v) => {
                self.skill_description = v;
                false
            }
            Msg::Publish | Msg::Retire => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                self.publishing = true;
                let skill = if matches!(msg, Msg::Publish) {
                    json!({"name":self.skill_name,"description":self.skill_description})
                } else {
                    Value::Null
                };
                let url = format!("/api/coding/projects/{}", encode(&id));
                ctx.link().send_future(async move {
                    Msg::Patched(api("PATCH", &url, Some(json!({"skill":skill}))).await)
                });
                true
            }
            Msg::Patched(r) => {
                self.publishing = false;
                match r {
                    Ok(v) => {
                        self.detail["project"] = v;
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::AiChosen(provider, model) => {
                self.provider = provider;
                self.model = model;
                true
            }
            Msg::Revert(commit) => {
                let Some(id) = self.open.clone() else {
                    return false;
                };
                let url = format!("/api/coding/projects/{}/revert", encode(&id));
                ctx.link().send_future(async move {
                    Msg::Reverted(api("POST", &url, Some(json!({"commit":commit}))).await)
                });
                true
            }
            Msg::Reverted(r) => {
                match r {
                    Ok(v) => {
                        self.detail["files"] = v["files"].clone();
                        self.detail["history"] = v["history"].clone();
                        self.dirty = false;
                        let path = self.file.clone();
                        if !path.is_empty() {
                            ctx.link().send_message(Msg::PickFile(path));
                        }
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::ClearError => {
                self.error.clear();
                true
            }
        }
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        html! {<section class="coding-page" data-testid="coding-page">
            if !self.error.is_empty(){<p class="error-banner" role="alert"><span>{t(&self.error)}</span><button aria-label={t("Dismiss error")} onclick={ctx.link().callback(|_|Msg::ClearError)}>{icon("close")}</button></p>}
            if self.open.is_some(){{self.project_view(ctx)}}else{{self.list_view(ctx)}}
        </section>}
    }
}
impl Coding {
    /// Read the stage text for the bar: which round, which phase, which step.
    fn note_stage(&mut self, text: &str) {
        if let Some(rest) = text.strip_prefix("Round ") {
            let mut parts = rest.split_whitespace();
            self.round = parts
                .next()
                .and_then(|r| r.parse().ok())
                .unwrap_or(self.round);
            if parts.next() == Some("of") {
                self.rounds = parts
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(self.rounds);
            }
            self.phase = Phase::Writing;
        } else if let Some(step) = text.strip_prefix("Step ") {
            self.steps = step
                .split_whitespace()
                .next()
                .and_then(|k| k.parse().ok())
                .unwrap_or(self.steps + 1);
            self.phase = Phase::Writing;
        } else if text.starts_with("Claude Code ·") || text.starts_with("Codex ·") {
            self.steps += 1;
            self.phase = Phase::Writing;
        } else if text.contains("Building and running") {
            self.phase = Phase::Building;
        } else if text.contains("Running the tests") || text.starts_with("Running ") {
            self.phase = Phase::Testing;
        } else if text.contains("example input") {
            self.phase = Phase::Verifying;
        }
    }
    /// 0 to 1: rounds (or steps) carry the bar, the phase and the growing
    /// draft move it inside a round, so a slow model is always seen moving.
    fn fraction(&self) -> f64 {
        if self.phase == Phase::Done {
            return 1.0;
        }
        let within = match self.phase {
            Phase::Writing => 0.1 + 0.45 * (1.0 - (-(self.draft_chars as f64) / 1500.0).exp()),
            Phase::Building => 0.7,
            Phase::Testing | Phase::Verifying => 0.85,
            Phase::Done => 1.0,
        };
        let base = if self.rounds > 0 {
            // Rounds usually end early; weight the first ones more.
            let r = self.round.max(1) as f64;
            let n = self.rounds as f64;
            let done = 1.0 - (1.0 - (r - 1.0) / n).powf(1.6);
            done + (1.0 - done) * within * 0.9
        } else if self.steps > 0 {
            1.0 - (-(self.steps as f64) / 7.0).exp()
        } else {
            0.04 + within * 0.1
        };
        base.clamp(0.02, 0.96)
    }
    fn elapsed_text(&self) -> String {
        if self.started == 0.0 {
            return String::new();
        }
        let secs = ((js_sys::Date::now() - self.started) / 1000.0).max(0.0) as u64;
        if secs >= 60 {
            format!("{}m {:02}s", secs / 60, secs % 60)
        } else {
            format!("{secs}s")
        }
    }
    fn speed_text(&self) -> String {
        if self.draft_started == 0.0 || self.draft_chars < 20 {
            return String::new();
        }
        let secs = ((js_sys::Date::now() - self.draft_started) / 1000.0).max(0.5);
        format!("{:.0} {}", self.draft_chars as f64 / secs, t("chars/s"))
    }
    fn finish_job(&mut self, ctx: &Context<Self>) {
        self.phase = Phase::Done;
        self.tick = None;
        if let Some(job) = self.job.take() {
            crate::api::unsubscribe(&job);
        }
        if let Some(id) = self.open.clone() {
            self.run = None;
            self.file.clear();
            let url = format!("/api/coding/projects/{}", encode(&id));
            ctx.link()
                .send_future(async move { Msg::Detail(api("GET", &url, None).await) });
        }
    }
    /// The shared AI picker and one line saying how that AI does the work.
    fn ai_panel(&self, ctx: &Context<Self>) -> Html {
        let settings = &ctx.props().settings;
        let local = matches!(self.provider.as_str(), "apple" | "mlx");
        let cli = causewaybay_shared::cli_provider(&self.provider);
        let mode = settings
            .user_ai
            .as_ref()
            .map(|p| p.mode.clone())
            .unwrap_or_default();
        html! {<details class="chat-options coding-ai-settings" open=true><summary>{t("AI for this work")}</summary>
            <crate::ai_settings::AiPicker language={ctx.props().language} owner={ctx.props().owner.clone()} settings={settings.clone()} prefix="Coding" disabled={self.job.is_some()} changed={ctx.link().callback(|(p,m)|Msg::AiChosen(p,m))} ai_saved={ctx.props().ai_saved.clone()}/>
            <p class="chat-llm-log coding-ai" data-testid="coding-ai">{icon(if local {"shield"} else {"globe"})}<span>
                {t(crate::ai_settings::mode_label(&mode))}{" · "}{&self.provider}{if !self.model.is_empty(){format!(" · {}",self.model)}else{String::new()}}{" · "}
                {t(if cli {"works directly in the project folder with its own tools, sandboxed"} else {"writes the source through the backend harness; the backend builds and runs it"})}
                if settings.demo {{" · "}{t("Demo responses")}}
            </span></p>
            <a class="text-button" href="/ai">{icon("spark")}{t("AI settings")}{icon("arrow")}</a>
        </details>}
    }
    fn list_view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let languages = self.overview["languages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let projects = self.overview["projects"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let chosen = languages
            .iter()
            .find(|l| l["id"] == self.new_language.as_str())
            .cloned()
            .unwrap_or(Value::Null);
        html! {<>
            <div class="page-intro"><div><div class="eyebrow">{icon("play")}{t("CODING AGENT")}</div><h1>{t("Write it, run it, see the result.")}</h1><p>{t("Each project is its own folder on the backend. Describe the program; your AI writes it, the backend builds and runs it in a sandbox, and the output appears here.")}</p></div></div>
            {self.ai_panel(ctx)}
            <form class="coding-new" data-testid="coding-new" onsubmit={link.callback(|e:SubmitEvent|{e.prevent_default();Msg::Create})}>
                <div class="coding-languages" role="group" aria-label={t("Language")}>
                    {for languages.iter().map(|l|{let id=l["id"].as_str().unwrap_or("").to_owned();let available=l["available"]==true;let active=id==self.new_language;let pick=id.clone();html!{<button type="button" key={id.clone()} class={classes!("chip-button",active.then_some("is-active"),(!available).then_some("is-unavailable"))} aria-pressed={active.to_string()} title={l["detail"].as_str().unwrap_or("").to_owned()} onclick={link.callback(move |_|Msg::NewLanguage(pick.clone()))}>{l["name"].as_str().unwrap_or("")}if !available{<small>{t("not installed")}</small>}</button>}})}
                </div>
                <div class="coding-new-row">
                    <input aria-label={t("Project name")} placeholder={t("Project name (optional)")} value={self.new_name.clone()} oninput={link.callback(|e:InputEvent|Msg::NewName(e.target_unchecked_into::<HtmlInputElement>().value()))}/>
                    <button type="submit" class="primary-button" data-testid="coding-create" disabled={self.creating||self.overview.is_null()}>{icon("plus")}{t("New project")}</button>
                </div>
                if !chosen.is_null() {<p class="small-muted coding-detail">{if chosen["available"]==true{tf("{language} runs with {detail}", &[("language",chosen["name"].as_str().unwrap_or("").into()),("detail",chosen["detail"].as_str().unwrap_or("").into())])}else{chosen["detail"].as_str().unwrap_or("").to_owned()}}</p>}
            </form>
            if self.overview.is_null(){<div class="loading-state"><span class="spinner"/>{t("Opening your projects…")}</div>}
            else if projects.is_empty(){<div class="empty-state">{icon("play")}<h3>{t("No projects yet.")}</h3><p>{t("Pick a language and create a project. Then tell the agent what to build.")}</p></div>}
            else {<div class="coding-grid">{for projects.iter().map(|p|{
                let id=p["id"].as_str().unwrap_or("").to_owned();let open=id.clone();
                let run=&p["last_run"];
                html!{<button type="button" key={id.clone()} class="coding-card" data-testid="coding-card" onclick={link.callback(move |_|Msg::Open(open.clone()))}>
                    <div class="coding-card-top"><span class="code-lang">{p["language_name"].as_str().unwrap_or("")}</span>if let Some(k)=p["skill"].as_object(){<span class="wiki-tag coding-skill-tag">{icon("spark")}{k["name"].as_str().unwrap_or("")}</span>}<time>{crate::short_date(p["updated_at"].as_str().unwrap_or(""))}</time></div>
                    <h3>{p["name"].as_str().unwrap_or("")}</h3>
                    <p>{if run.is_null(){t("Not run yet")}else if run["success"]==true{t("Last run succeeded")}else{t("Last run failed")}}</p>
                </button>}})}</div>}
        </>}
    }
    fn project_view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let project = &self.detail["project"];
        let files = self.detail["files"].as_array().cloned().unwrap_or_default();
        let memos = self.detail["memos"].as_array().cloned().unwrap_or_default();
        let busy = self.job.is_some();
        let skill = project["skill"].as_object().cloned();
        html! {<>
            <div class="answer-heading"><button class="text-button" data-testid="coding-back" onclick={link.callback(|_|Msg::Close)}><span class="back-arrow">{icon("arrow")}</span>{t("All projects")}</button>
                <span class="save-status">{if busy{html!{<span class="spinner"/>}}else{icon("check")}}{if busy{self.stage.clone()}else{t("Ready")}}</span></div>
            if self.detail.is_null(){<div class="loading-state"><span class="spinner"/>{t("Opening the project…")}</div>}
            else {<>
            <header class="coding-head"><div><span class="code-lang">{project["language_name"].as_str().unwrap_or("")}</span><h1 data-testid="coding-title">{project["name"].as_str().unwrap_or("")}</h1><p class="small-muted coding-folder"><code>{self.detail["folder"].as_str().unwrap_or("")}</code></p></div>
                <div class="coding-head-actions"><button class="text-button delete-action" data-testid="coding-delete" onclick={link.callback(|_|Msg::Delete)}>{icon("trash")}{if self.confirm_delete{t("Confirm delete")}else{t("Delete project")}}</button></div></header>
            <div class="coding-columns">
                <section class="coding-agent" aria-label={t("Ask the agent")}>
                    <h2>{icon("spark")}{t("Tell the agent what to build")}</h2>
                    {self.ai_panel(ctx)}
                    <textarea data-testid="coding-prompt" aria-label={t("What should the program do?")} placeholder={t("For example: read numbers from stdin and print their sum and average.")} rows="4" value={self.prompt.clone()} disabled={busy} oninput={link.callback(|e:InputEvent|Msg::Prompt(e.target_unchecked_into::<HtmlTextAreaElement>().value()))} onkeydown={link.batch_callback(|e:KeyboardEvent|{if e.key()=="Enter"&&(e.meta_key()||e.ctrl_key()){e.prevent_default();Some(Msg::Ask)}else{None}})}/>
                    <div class="coding-agent-row"><button class="primary-button" data-testid="coding-ask" disabled={busy} onclick={link.callback(|_|Msg::Ask)}>{if busy{html!{<span class="spinner"/>}}else{icon("send")}}{t("Build it")}</button>if busy{<button class="secondary-button" data-testid="coding-stop" onclick={link.callback(|_|Msg::Stop)}>{icon("stop")}{t("Stop")}</button>}<span class="small-muted">{t("⌘ Enter to send · the agent keeps going until it runs")}</span></div>
                    if busy || !self.events.is_empty() || !self.answer.is_empty() || !self.job_error.is_empty() {<div class="coding-live" data-testid="coding-live">
                        if busy {<div class="coding-progress" data-testid="coding-progress">
                            <ProgressBar fraction={self.fraction()} label={if self.rounds>0{tf("Round {round} of {rounds}", &[("round",self.round.max(1).to_string()),("rounds",self.rounds.to_string())])}else if self.steps>0{tf("Step {step}", &[("step",self.steps.to_string())])}else{t("Starting")}} detail={{let speed=self.speed_text();if speed.is_empty(){self.elapsed_text()}else{format!("{} · {}",self.elapsed_text(),speed)}}}/>
                            <ol class="coding-phases" aria-label={t("Progress")}>{for [(Phase::Writing,"Write","file"),(Phase::Building,"Build","bolt"),(Phase::Testing,"Run","play"),(Phase::Verifying,"Verify","check")].into_iter().map(|(phase,label,symbol)|{let order=|p:Phase|match p{Phase::Writing=>0,Phase::Building=>1,Phase::Testing=>2,Phase::Verifying=>3,Phase::Done=>4};let (here,mine)=(order(self.phase),order(phase));html!{<li key={label} class={classes!("coding-phase",(mine<here).then_some("is-done"),(mine==here).then_some("is-current"))}>{icon(symbol)}<span>{t(label)}</span></li>}})}</ol>
                            <FadeSwap identity={self.stage.clone()}><p class="coding-stage"><span class="spinner"/>{&self.stage}</p></FadeSwap>
                        </div>}
                        if busy && !self.draft.is_empty() {<details class="coding-draft" open=true data-testid="coding-draft"><summary>{icon("bolt")}{t("The model is writing")}<span>{tf("{count} characters so far", &[("count",self.draft_chars.to_string())])}</span></summary><pre ref={self.draft_node.clone()}>{&self.draft}<span class="coding-caret" aria-hidden="true"/></pre></details>}
                        if !self.todos.is_empty(){<ul class="coding-todos" data-testid="coding-todos">{for self.todos.iter().map(|t|{let status=t["status"].as_str().unwrap_or("pending");html!{<li class={classes!("coding-todo",format!("is-{status}"))}>{icon(match status{"completed"=>"check","in_progress"=>"bolt","cancelled"=>"close",_=>"clock"})}<span>{t["content"].as_str().unwrap_or("")}</span></li>}})}</ul>}
                        if !self.events.is_empty(){<details class="tool-activity" open=true><summary>{icon("code")}{t("Workspace activity")}<span>{tf("{count} steps", &[("count",self.events.len().to_string())])}</span></summary>{for self.events.iter().map(|e|{
                            let v:Value=serde_json::from_str(&e.text).unwrap_or(Value::Null);let name=v["name"].as_str().unwrap_or("step").to_owned();
                            let args:Value=serde_json::from_str(v["arguments"].as_str().unwrap_or("")).unwrap_or(Value::Null);let out:Value=serde_json::from_str(v["output"].as_str().unwrap_or("")).unwrap_or(Value::Null);
                            let title=if e.kind=="tool_start"{match name.as_str(){"write_file"=>format!("{} {}",t("Writing"),args["path"].as_str().unwrap_or("")),"edit_file"=>format!("{} {}",t("Editing"),args["path"].as_str().unwrap_or("")),"read_file"=>format!("{} {}",t("Reading"),args["path"].as_str().unwrap_or("")),"grep"=>format!("{} {}",t("Searching"),args["pattern"].as_str().or(args["text"].as_str()).unwrap_or("")),"list_files"=>t("Listing files"),"delete_file"=>format!("{} {}",t("Deleting"),args["path"].as_str().unwrap_or("")),"run_project"|"run_program"=>t("Building and running"),"run_tests"=>t("Running tests"),"run_command"=>format!("{} {} {}",t("Running"),args["program"].as_str().unwrap_or(""),args["args"].as_array().map(|a|a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default()),"glob"=>format!("{} {}",t("Finding files"),args["pattern"].as_str().unwrap_or("")),"todo_write"=>t("Updating the task list"),"publish_skill"=>format!("{} {}",t("Publishing skill"),args["name"].as_str().unwrap_or("")),"web_search"=>format!("{} {}",t("Searching the web"),args["query"].as_str().unwrap_or("")),other if other.starts_with("claude_code.")||other.starts_with("codex.")=>format!("{} · {}",if other.starts_with("codex."){"Codex"}else{"Claude Code"},other.split('.').nth(1).unwrap_or("")),other=>format!("{} {}",t(other),t("called"))}}else{match name.as_str(){"run_project"|"run_program"=>if out["success"]==true{t("Run succeeded")}else{t("Run failed")},"run_tests"=>if out["success"]==true{t("Tests passed")}else{t("Tests failed")},"run_command"=>if out["success"]==true{t("Command succeeded")}else{t("Command failed")},_=>if out["error"].is_string(){format!("{}: {}",t("Problem"),out["error"].as_str().unwrap_or(""))}else{format!("{} {}",t(&name.replace(['_', '.'], " ")),t("result"))}}};
                            let body=if e.kind=="tool_start"{if name=="write_file"{args["content"].as_str().unwrap_or("").to_owned()}else if args.is_string(){args.as_str().unwrap_or("").to_owned()}else{args.to_string()}}else if (name=="run_project"||name=="run_program"||name=="run_tests"||name=="run_command")&&out.is_object(){format!("{}\n{}",out["stdout"].as_str().unwrap_or(""),out["stderr"].as_str().unwrap_or("")).trim().to_owned()}else{v["output"].as_str().unwrap_or("").to_owned()};
                            html!{<details key={e.id} class="tool-event" open={e.kind=="tool_result"&&(name=="run_project"||name=="run_program"||name=="run_tests")}><summary>{icon(if e.kind=="tool_start"{"bolt"}else if out["error"].is_string()||(name=="run_project"&&out["success"]!=true){"close"}else{"check"})}<span>{title}</span></summary>if !body.is_empty(){<pre>{body}</pre>}</details>}
                        })}</details>}
                        if !self.job_error.is_empty(){<p class="error-banner" role="alert">{icon("info")}<span>{t(&self.job_error)}</span></p>}
                        if !self.answer.is_empty(){<FadeSwap identity={format!("answer:{}",self.answer.len()/400)}><div class="markdown coding-answer" data-testid="coding-answer">{crate::markdown(&self.answer)}</div></FadeSwap>}
                    </div>}
                    if !memos.is_empty(){<details class="coding-history"><summary>{icon("clock")}{t("Earlier requests")}<span>{memos.len()}</span></summary><ul>{for memos.iter().map(|m|html!{<li key={m["id"].as_str().unwrap_or("").to_owned()}><a href={format!("/memos/{}",m["id"].as_str().unwrap_or(""))}>{m["title"].as_str().unwrap_or("")}</a><small>{t(match m["status"].as_str(){Some("complete")=>"done",Some("error")=>"failed",_=>"running"})}{" · "}{crate::short_date(m["created_at"].as_str().unwrap_or(""))}</small></li>})}</ul></details>}
                </section>
                <section class="coding-files" aria-label={t("Files")}>
                    <h2>{icon("folder")}{t("Files")}</h2>
                    <ul class="coding-file-list" data-testid="coding-files">{for files.iter().map(|f|{let path=f["path"].as_str().unwrap_or("").to_owned();let active=path==self.file;let pick=path.clone();html!{<li key={path.clone()}><button type="button" class={classes!(active.then_some("is-active"))} onclick={link.callback(move |_|Msg::PickFile(pick.clone()))}>{icon("file")}<span>{&path}</span><small>{crate::file_size(f["size"].as_u64().unwrap_or(0) as usize)}</small></button></li>}})}</ul>
                    if !self.file.is_empty(){<div class="coding-editor">
                        <div class="coding-editor-bar"><code>{&self.file}</code>if self.dirty{<small>{t("Unsaved changes")}</small>}<button class="secondary-button" data-testid="coding-save" disabled={!self.dirty||self.saving} onclick={link.callback(|_|Msg::Save)}>{icon("check")}{if self.saving{t("Saving…")}else{t("Save")}}</button><button class="icon-button" aria-label={t("Copy code")} title={t("Copy code")} onclick={ctx.props().copy.reform({let text=self.text.clone();move |_|text.clone()})}>{icon("copy")}</button></div>
                        <textarea class="coding-source" data-testid="coding-source" aria-label={tf("Source of {name}", &[("name",self.file.clone())])} spellcheck="false" value={self.text.clone()} oninput={link.callback(|e:InputEvent|Msg::Edit(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/>
                    </div>}
                    <div class="coding-run">
                        <h2>{icon("play")}{t("Run")}</h2>
                        <label class="form-label">{t("Program input (stdin)")}<textarea data-testid="coding-stdin" rows="2" value={self.stdin.clone()} oninput={link.callback(|e:InputEvent|Msg::Stdin(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/></label>
                        <label class="form-label skill-args">{t("Arguments")}<input data-testid="coding-args" placeholder={t("optional, space-separated")} value={self.args.clone()} oninput={link.callback(|e:InputEvent|Msg::Args(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                        <div class="coding-run-row"><button class="secondary-button" data-testid="coding-run" disabled={self.running||busy} onclick={link.callback(|_|Msg::Run)}>{if self.running{html!{<span class="spinner"/>}}else{icon("play")}}{t("Build and run")}</button><button class="secondary-button" data-testid="coding-suggest" title={t("Ask the AI for a representative input; it becomes the folder's example.json")} disabled={self.suggesting||busy} onclick={link.callback(|_|Msg::Suggest)}>{if self.suggesting{html!{<span class="spinner"/>}}else{icon("spark")}}{t("Suggest input")}</button>if matches!(project["language"].as_str(),Some("rust"|"go"|"python"|"typescript")){<button class="secondary-button" data-testid="coding-test" disabled={self.testing||busy} onclick={link.callback(|_|Msg::Test)}>{if self.testing{html!{<span class="spinner"/>}}else{icon("check")}}{t("Run tests")}</button>}</div>
                        if let Some(run)=&self.tests {<div class={classes!("coding-output",if run["success"]==true{"is-ok"}else{"is-failed"})} data-testid="coding-tests">
                            <div class="coding-output-head">{icon(if run["success"]==true{"check"}else{"close"})}<strong>{if run["success"]==true{t("Tests passed")}else{t("Tests failed")}}</strong><span>{run["phase"].as_str().unwrap_or("")}{" · "}{tf("exit code {code}", &[("code",run["exit_code"].as_i64().map(|c|c.to_string()).unwrap_or("—".into()))])}</span></div>
                            if run["stdout"].as_str().is_some_and(|s|!s.trim().is_empty()){<pre class="coding-stdout">{run["stdout"].as_str().unwrap_or("")}</pre>}
                            if run["stderr"].as_str().is_some_and(|s|!s.trim().is_empty()){<pre class="coding-stderr">{run["stderr"].as_str().unwrap_or("")}</pre>}
                        </div>}
                        if let Some(run)=&self.run {<div class={classes!("coding-output",if run["success"]==true{"is-ok"}else{"is-failed"})} data-testid="coding-output">
                            <div class="coding-output-head">{icon(if run["success"]==true{"check"}else{"close"})}<strong>{if run["timed_out"]==true{t("Timed out")}else if run["success"]==true{t("Succeeded")}else{t("Failed")}}</strong><span>{run["phase"].as_str().unwrap_or("")}{" · "}{tf("exit code {code}", &[("code",run["exit_code"].as_i64().map(|c|c.to_string()).unwrap_or("—".into()))])}{" · "}{run["duration_ms"].as_u64().unwrap_or(0)}{" ms"}</span></div>
                            if run["stdout"].as_str().is_some_and(|s|!s.trim().is_empty()){<pre class="coding-stdout" data-testid="coding-stdout">{run["stdout"].as_str().unwrap_or("")}</pre>}
                            if run["stderr"].as_str().is_some_and(|s|!s.trim().is_empty()){<pre class="coding-stderr">{run["stderr"].as_str().unwrap_or("")}</pre>}
                            if run["stdout"].as_str().is_none_or(|s|s.trim().is_empty())&&run["stderr"].as_str().is_none_or(|s|s.trim().is_empty()){<p class="small-muted">{t("No output")}</p>}
                        </div>}
                    </div>
                    if let Some(history)=self.detail["history"].as_array().filter(|h|!h.is_empty()) {<details class="coding-history coding-checkpoints" data-testid="coding-checkpoints"><summary>{icon("clock")}{t("Checkpoints")}<span>{history.len()}</span></summary>
                        <ul>{for history.iter().enumerate().map(|(i,h)|{let commit=h["commit"].as_str().unwrap_or("").to_owned();html!{<li key={commit.clone()}><span>{h["message"].as_str().unwrap_or("")}</span><small>{crate::short_date(h["at"].as_str().unwrap_or(""))}</small>if i>0{<button class="text-button" data-testid="coding-revert" disabled={busy} onclick={link.callback(move |_|Msg::Revert(commit.clone()))}>{icon("refresh")}{t("Revert to this")}</button>}else{<small>{t("current")}</small>}</li>}})}</ul>
                        <p class="small-muted">{t("Every agent request and every save is a checkpoint, so nothing the agent does is final.")}</p></details>}
                    <div class="coding-skill" data-testid="coding-skill">
                        <h2>{icon("spark")}{t("Skill")}</h2>
                        if let Some(k)=&skill {<p>{tf("Published as {name}: your chats can run it, and it is listed under Skills.", &[("name",k["name"].as_str().unwrap_or("").to_owned())])}</p><p class="small-muted">{k["description"].as_str().unwrap_or("")}</p><button class="secondary-button" data-testid="coding-retire" disabled={self.publishing} onclick={link.callback(|_|Msg::Retire)}>{icon("close")}{t("Retire skill")}</button>}
                        else {<p class="small-muted">{t("Publish this program as a skill: the chat AI can run it with text on stdin, and it appears under Skills.")}</p>
                            <label class="form-label">{t("Skill name")}<input data-testid="coding-skill-name" value={self.skill_name.clone()} placeholder="word-count" oninput={link.callback(|e:InputEvent|Msg::SkillName(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                            <label class="form-label">{t("What it does and what input it reads")}<input data-testid="coding-skill-description" value={self.skill_description.clone()} placeholder={t("Counts the words in the text it is given")} oninput={link.callback(|e:InputEvent|Msg::SkillDescription(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                            <button class="secondary-button" data-testid="coding-publish" disabled={self.publishing} onclick={link.callback(|_|Msg::Publish)}>{icon("spark")}{t("Publish skill")}</button>}
                    </div>
                </section>
            </div>
            </>}
        </>}
    }
}
