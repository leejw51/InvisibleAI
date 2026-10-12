//! Skills: the user's own command-line programs, managed in one place. Make a
//! new one from a description (the coding agent builds it in its folder and
//! it goes live when it runs cleanly), add code from an existing project as a
//! skill, rename or re-describe it, run it with any input, open its code in
//! the workbench, retire it, or delete it with its code. The chat AI can run
//! every live skill.
use crate::{
    api::{api, encode},
    i18n::{Language, t, tf},
    icons::icon,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use web_sys::{HtmlInputElement, HtmlTextAreaElement};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub owner: String,
    pub settings: causewaybay_shared::Settings,
    pub ai_saved: Callback<()>,
    pub navigate: Callback<String>,
}
pub struct Skills {
    data: Value,
    loaded: bool,
    error: String,
    /// New skill form.
    form_open: bool,
    name: String,
    description: String,
    new_language: String,
    prompt: String,
    example_stdin: String,
    example_args: String,
    provider: String,
    model: String,
    creating: bool,
    /// Per-skill run input and output.
    inputs: HashMap<String, String>,
    args: HashMap<String, String>,
    running: Option<String>,
    outputs: HashMap<String, Value>,
    /// A skill being renamed or re-described, and a project being added.
    editing: Option<(String, String, String)>,
    adding: Option<(String, String, String)>,
    saving: bool,
    confirm_delete: String,
    poll: Option<gloo_timers::callback::Timeout>,
}
pub enum Msg {
    Load,
    Loaded(Result<Value, String>),
    ToggleForm,
    Name(String),
    Description(String),
    NewLanguage(String),
    Prompt(String),
    ExampleStdin(String),
    ExampleArgs(String),
    AiChosen(String, String),
    Create,
    Created(Result<Value, String>),
    Input(String, String),
    Args(String, String),
    Run(String),
    Ran(String, Result<Value, String>),
    Edit(String),
    EditName(String),
    EditDescription(String),
    CancelEdit,
    SaveEdit,
    Add(String),
    AddName(String),
    AddDescription(String),
    CancelAdd,
    SaveAdd,
    Retire(String),
    Delete(String),
    Saved(Result<Value, String>),
    Open(String),
    /// Open the workbench with a prompt that quotes the last failure.
    Fix(String),
}
fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(40)
        .collect()
}
impl Component for Skills {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        let (provider, model) = crate::ai_settings::ai_choice(
            &ctx.props().settings,
            &crate::ai_settings::preferences(&ctx.props().owner),
        );
        Self {
            data: Value::Null,
            loaded: false,
            error: String::new(),
            form_open: false,
            name: String::new(),
            description: String::new(),
            new_language: "python".into(),
            prompt: String::new(),
            example_stdin: String::new(),
            example_args: String::new(),
            provider,
            model,
            creating: false,
            inputs: HashMap::new(),
            args: HashMap::new(),
            running: None,
            outputs: HashMap::new(),
            editing: None,
            adding: None,
            saving: false,
            confirm_delete: String::new(),
            poll: None,
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Load => {
                ctx.link()
                    .send_future(async { Msg::Loaded(api("GET", "/api/skills", None).await) });
                false
            }
            Msg::Loaded(r) => {
                self.loaded = true;
                match r {
                    Ok(v) => {
                        if self.data.is_null()
                            && let Some(first) = v["languages"]
                                .as_array()
                                .and_then(|l| l.iter().find(|l| l["available"] == true))
                            && let Some(id) = first["id"].as_str()
                        {
                            self.new_language = id.to_owned();
                        }
                        // A skill still being built refreshes the list until it settles.
                        let building = v["skills"]
                            .as_array()
                            .is_some_and(|s| s.iter().any(|k| k["building"] == true));
                        self.poll = building.then(|| {
                            let link = ctx.link().clone();
                            gloo_timers::callback::Timeout::new(3000, move || {
                                link.send_message(Msg::Load)
                            })
                        });
                        self.data = v;
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::ToggleForm => {
                self.form_open = !self.form_open;
                true
            }
            Msg::Name(v) => {
                self.name = v;
                true
            }
            Msg::Description(v) => {
                self.description = v;
                false
            }
            Msg::NewLanguage(v) => {
                self.new_language = v;
                true
            }
            Msg::Prompt(v) => {
                self.prompt = v;
                false
            }
            Msg::ExampleStdin(v) => {
                self.example_stdin = v;
                false
            }
            Msg::ExampleArgs(v) => {
                self.example_args = v;
                false
            }
            Msg::AiChosen(p, m) => {
                self.provider = p;
                self.model = m;
                true
            }
            Msg::Create => {
                if self.creating {
                    return false;
                }
                self.creating = true;
                self.error.clear();
                let args: Vec<String> = self
                    .example_args
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                let body = json!({"name":self.name,"description":self.description,"language":self.new_language,"prompt":self.prompt,"example_stdin":self.example_stdin,"example_args":args,"provider":self.provider,"model":self.model});
                ctx.link().send_future(async move {
                    Msg::Created(api("POST", "/api/skills", Some(body)).await)
                });
                true
            }
            Msg::Created(r) => {
                self.creating = false;
                match r {
                    Ok(v) => {
                        self.name.clear();
                        self.description.clear();
                        self.prompt.clear();
                        self.example_stdin.clear();
                        self.example_args.clear();
                        self.form_open = false;
                        if let Some(id) = v["project"]["id"].as_str() {
                            // Watch the agent build it in the workbench.
                            ctx.props().navigate.emit(format!("coding/{id}"));
                        }
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Input(id, v) => {
                self.inputs.insert(id, v);
                false
            }
            Msg::Args(id, v) => {
                self.args.insert(id, v);
                false
            }
            Msg::Run(id) => {
                if self.running.is_some() {
                    return false;
                }
                self.running = Some(id.clone());
                let url = format!("/api/skills/{}/run", encode(&id));
                let args: Vec<String> = self
                    .args
                    .get(&id)
                    .map(|a| a.split_whitespace().map(str::to_owned).collect())
                    .unwrap_or_default();
                let body =
                    json!({"input":self.inputs.get(&id).cloned().unwrap_or_default(),"args":args});
                ctx.link().send_future(async move {
                    Msg::Ran(id.clone(), api("POST", &url, Some(body)).await)
                });
                true
            }
            Msg::Ran(id, r) => {
                self.running = None;
                match r {
                    Ok(v) => {
                        self.outputs.insert(id, v);
                        self.error.clear();
                        // The run is in the ledger now; the card's numbers follow.
                        ctx.link().send_message(Msg::Load);
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Edit(id) => {
                let item = self.skill(&id);
                self.editing = Some((
                    id,
                    item["name"].as_str().unwrap_or("").to_owned(),
                    item["description"].as_str().unwrap_or("").to_owned(),
                ));
                self.adding = None;
                true
            }
            Msg::EditName(v) => {
                if let Some(e) = &mut self.editing {
                    e.1 = v;
                }
                false
            }
            Msg::EditDescription(v) => {
                if let Some(e) = &mut self.editing {
                    e.2 = v;
                }
                false
            }
            Msg::CancelEdit => {
                self.editing = None;
                true
            }
            Msg::SaveEdit => {
                let Some((id, name, description)) = self.editing.clone() else {
                    return false;
                };
                self.patch(
                    ctx,
                    &id,
                    json!({"skill":{"name":name,"description":description}}),
                );
                true
            }
            Msg::Add(id) => {
                let name = self.data["projects"]
                    .as_array()
                    .and_then(|p| p.iter().find(|p| p["id"] == id.as_str()))
                    .and_then(|p| p["name"].as_str())
                    .map(slug)
                    .unwrap_or_default();
                self.adding = Some((id, name, String::new()));
                self.editing = None;
                true
            }
            Msg::AddName(v) => {
                if let Some(a) = &mut self.adding {
                    a.1 = v;
                }
                false
            }
            Msg::AddDescription(v) => {
                if let Some(a) = &mut self.adding {
                    a.2 = v;
                }
                false
            }
            Msg::CancelAdd => {
                self.adding = None;
                true
            }
            Msg::SaveAdd => {
                let Some((id, name, description)) = self.adding.clone() else {
                    return false;
                };
                self.patch(
                    ctx,
                    &id,
                    json!({"skill":{"name":name,"description":description}}),
                );
                true
            }
            Msg::Retire(id) => {
                self.patch(ctx, &id, json!({"skill":null}));
                true
            }
            Msg::Delete(id) => {
                if self.confirm_delete != id {
                    self.confirm_delete = id;
                    return true;
                }
                self.saving = true;
                let url = format!("/api/coding/projects/{}", encode(&id));
                ctx.link()
                    .send_future(async move { Msg::Saved(api("DELETE", &url, None).await) });
                true
            }
            Msg::Saved(r) => {
                self.saving = false;
                self.confirm_delete.clear();
                match r {
                    Ok(_) => {
                        self.editing = None;
                        self.adding = None;
                        self.error.clear();
                        ctx.link().send_message(Msg::Load);
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Open(id) => {
                ctx.props().navigate.emit(format!("coding/{id}"));
                false
            }
            Msg::Fix(id) => {
                let item = self.skill(&id);
                let error = item["stats"]["last_error"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                let prompt = format!(
                    "The skill `{}` failed when it was used. Last failure:\n{}\n\nFix the cause, keep the command-line contract (stdin and arguments in, stdout out, exit 0), and run it on that kind of input before finishing.",
                    item["name"].as_str().unwrap_or(""),
                    error.chars().take(3000).collect::<String>()
                );
                if let Some(storage) =
                    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
                {
                    let _ = storage.set_item(&format!("causewaybay.coding.prompt.{id}"), &prompt);
                }
                ctx.props().navigate.emit(format!("coding/{id}"));
                false
            }
        }
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let items = self.data["skills"].as_array().cloned().unwrap_or_default();
        let projects = self.data["projects"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        html! {<section class="skills-page" data-testid="skills-page">
            <div class="page-intro"><div><div class="eyebrow">{icon("spark")}{t("YOUR SKILLS")}</div><h1>{t("Small command-line programs, yours to add.")}</h1><p>{t("A skill is a command-line program you made: input on stdin, result on stdout. Describe one and the agent builds it; it goes live when it runs cleanly. Run it here with any input, and the chat AI can run it too.")}</p></div>
                <button class="primary-button" data-testid="skills-new" onclick={link.callback(|_|Msg::ToggleForm)}>{icon("plus")}{t("New skill")}</button></div>
            if !self.error.is_empty(){<p class="error-banner" role="alert">{icon("info")}<span>{t(&self.error)}</span></p>}
            if self.form_open {{self.form(ctx)}}
            if !self.loaded{<div class="loading-state"><span class="spinner"/>{t("Reading your skills…")}</div>}
            else if items.is_empty(){<div class="empty-state">{icon("spark")}<h3>{t("No skills yet.")}</h3><p>{t("Describe your first skill above, or add the code of a project below.")}</p></div>}
            else {<div class="skills-grid">{for items.iter().map(|k| self.card(ctx, k))}</div>}
            if !projects.is_empty() {<section class="skills-drafts" data-testid="skills-projects"><h2>{icon("folder")}{t("Projects not published as skills")}</h2>
                <ul>{for projects.iter().map(|p|{
                    let id=p["id"].as_str().unwrap_or("").to_owned();let (add,open)=(id.clone(),id.clone());
                    html!{<li key={id.clone()}><span class="code-lang">{p["language_name"].as_str().unwrap_or("")}</span><strong>{p["name"].as_str().unwrap_or("")}</strong><small>{if p["ready"]==true{t("runs cleanly")}else{t("not run cleanly yet")}}</small>
                        <button class="text-button" onclick={link.callback(move |_|Msg::Open(open.clone()))}>{icon("code")}{t("Open code")}</button>
                        <button class="secondary-button" data-testid="skills-add" onclick={link.callback(move |_|Msg::Add(add.clone()))}>{icon("plus")}{t("Add as skill")}</button>
                        if self.adding.as_ref().is_some_and(|a|a.0==id) {{self.add_form(ctx)}}
                    </li>}})}</ul></section>}
        </section>}
    }
}
impl Skills {
    fn skill(&self, id: &str) -> Value {
        self.data["skills"]
            .as_array()
            .and_then(|s| s.iter().find(|k| k["id"] == id).cloned())
            .unwrap_or(Value::Null)
    }
    fn patch(&mut self, ctx: &Context<Self>, id: &str, body: Value) {
        self.saving = true;
        let url = format!("/api/coding/projects/{}", encode(id));
        ctx.link()
            .send_future(async move { Msg::Saved(api("PATCH", &url, Some(body)).await) });
    }
    fn form(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let languages = self.data["languages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let settings = &ctx.props().settings;
        html! {<form class="coding-new skills-form" data-testid="skills-form" onsubmit={link.callback(|e:SubmitEvent|{e.prevent_default();Msg::Create})}>
            <div class="coding-new-row">
                <input data-testid="skills-name" aria-label={t("Skill name")} placeholder={t("skill name, e.g. word-count")} value={self.name.clone()} oninput={link.callback(|e:InputEvent|Msg::Name(e.target_unchecked_into::<HtmlInputElement>().value()))}/>
                <input data-testid="skills-description" aria-label={t("What it does and what input it reads")} placeholder={t("What it does and what input it reads from stdin")} value={self.description.clone()} oninput={link.callback(|e:InputEvent|Msg::Description(e.target_unchecked_into::<HtmlInputElement>().value()))}/>
            </div>
            <div class="coding-languages" role="group" aria-label={t("Language")}>
                {for languages.iter().map(|l|{let id=l["id"].as_str().unwrap_or("").to_owned();let available=l["available"]==true;let active=id==self.new_language;let pick=id.clone();html!{<button type="button" key={id.clone()} class={classes!("chip-button",active.then_some("is-active"),(!available).then_some("is-unavailable"))} aria-pressed={active.to_string()} title={l["detail"].as_str().unwrap_or("").to_owned()} disabled={!available} onclick={link.callback(move |_|Msg::NewLanguage(pick.clone()))}>{l["name"].as_str().unwrap_or("")}</button>}})}
            </div>
            <textarea data-testid="skills-prompt" aria-label={t("Details for the agent (optional)")} placeholder={t("Details for the agent, if the description is not enough: formats, edge cases, examples.")} rows="3" value={self.prompt.clone()} oninput={link.callback(|e:InputEvent|Msg::Prompt(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/>
            <div class="coding-new-row skills-example">
                <label class="form-label">{t("Example input (stdin)")}<textarea data-testid="skills-example-stdin" rows="2" placeholder={t("What the skill will typically be given")} value={self.example_stdin.clone()} oninput={link.callback(|e:InputEvent|Msg::ExampleStdin(e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/></label>
                <label class="form-label">{t("Example arguments")}<input data-testid="skills-example-args" placeholder={t("optional, space-separated")} value={self.example_args.clone()} oninput={link.callback(|e:InputEvent|Msg::ExampleArgs(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
            </div>
            <p class="small-muted">{t("The skill counts as working when it runs cleanly on this example; the agent writes one itself if you leave it empty.")}</p>
            <details class="chat-options coding-ai-settings"><summary>{t("AI for this work")}</summary>
                <crate::ai_settings::AiPicker language={ctx.props().language} owner={ctx.props().owner.clone()} settings={settings.clone()} prefix="Skills" changed={link.callback(|(p,m)|Msg::AiChosen(p,m))} ai_saved={ctx.props().ai_saved.clone()}/>
            </details>
            <div class="coding-new-row"><button type="submit" class="primary-button" data-testid="skills-create" disabled={self.creating||self.name.trim().is_empty()}>{if self.creating{html!{<span class="spinner"/>}}else{icon("spark")}}{t("Build this skill")}</button><span class="small-muted">{t("The agent writes it in its own folder; you can watch, edit and run it there.")}</span></div>
        </form>}
    }
    fn add_form(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let (_, name, description) = self.adding.clone().unwrap_or_default();
        html! {<div class="skills-inline-form" data-testid="skills-add-form">
            <input aria-label={t("Skill name")} value={name} oninput={link.callback(|e:InputEvent|Msg::AddName(e.target_unchecked_into::<HtmlInputElement>().value()))}/>
            <input aria-label={t("What it does and what input it reads")} placeholder={t("What it does and what input it reads from stdin")} value={description} oninput={link.callback(|e:InputEvent|Msg::AddDescription(e.target_unchecked_into::<HtmlInputElement>().value()))}/>
            <button class="primary-button" data-testid="skills-add-save" disabled={self.saving} onclick={link.callback(|_|Msg::SaveAdd)}>{icon("check")}{t("Publish skill")}</button>
            <button class="text-button" onclick={link.callback(|_|Msg::CancelAdd)}>{t("Cancel")}</button>
        </div>}
    }
    fn card(&self, ctx: &Context<Self>, k: &Value) -> Html {
        let link = ctx.link();
        let id = k["id"].as_str().unwrap_or("").to_owned();
        let name = k["name"].as_str().unwrap_or("").to_owned();
        let ready = k["ready"] == true;
        let building = k["building"] == true;
        let available = k["available"] == true;
        let running = self.running.as_deref() == Some(id.as_str());
        let output = self.outputs.get(&id);
        let editing = self.editing.as_ref().filter(|e| e.0 == id).cloned();
        let (run, edit, retire, delete, open, inp, arg, fix) = (
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
        );
        let stats = &k["stats"];
        let runs = stats["runs"].as_i64().unwrap_or(0);
        let failures = stats["failures"].as_i64().unwrap_or(0);
        let chat_runs = stats["chat_runs"].as_i64().unwrap_or(0);
        html! {<article key={id.clone()} class={classes!("skill-card",(!ready).then_some("is-draft"))} data-testid="skill-card" aria-label={name.clone()}>
            if let Some((_, ename, edesc)) = editing {<div class="skills-inline-form" data-testid="skills-edit-form">
                <input aria-label={t("Skill name")} value={ename} oninput={link.callback(|e:InputEvent|Msg::EditName(e.target_unchecked_into::<HtmlInputElement>().value()))}/>
                <input aria-label={t("What it does and what input it reads")} value={edesc} oninput={link.callback(|e:InputEvent|Msg::EditDescription(e.target_unchecked_into::<HtmlInputElement>().value()))}/>
                <button class="primary-button" data-testid="skills-edit-save" disabled={self.saving} onclick={link.callback(|_|Msg::SaveEdit)}>{icon("check")}{t("Save")}</button>
                <button class="text-button" onclick={link.callback(|_|Msg::CancelEdit)}>{t("Cancel")}</button>
            </div>}
            else {<>
            <header><h3>{icon("spark")}{&name}</h3><span class="code-lang">{k["language_name"].as_str().unwrap_or("")}</span>
                <span class={classes!("skill-state",if building{"is-building"}else if ready{"is-live"}else{"is-draft"})} data-testid="skill-state">{if building{html!{<><span class="spinner"/>{t("Building…")}</>}}else if ready{html!{<>{icon("check")}{t("Live")}</>}}else{html!{<>{icon("info")}{t("Not running yet")}</>}}}</span></header>
            <p class="skill-description">{k["description"].as_str().unwrap_or("")}</p>
            <p class="small-muted skill-stats" data-testid="skill-stats">{if runs==0{t("Not used yet")}else{tf("{runs} runs · {failures} failed · last used {when}", &[("runs",runs.to_string()),("failures",failures.to_string()),("when",crate::short_date(stats["last_used_at"].as_str().unwrap_or("")))])}}{if chat_runs>0{format!(" · {}",tf("used by the chat AI {count} times", &[("count",chat_runs.to_string())]))}else{String::new()}}</p>
            if let Some(error)=stats["last_error"].as_str().filter(|e|!e.is_empty()) {<details class="skill-failure" data-testid="skill-failure"><summary>{icon("info")}{t("Last failure")}</summary><pre>{error}</pre><button class="secondary-button" data-testid="skill-fix" onclick={link.callback(move |_|Msg::Fix(fix.clone()))}>{icon("spark")}{t("Fix with the agent")}</button></details>}
            if !ready && !building {<p class="small-muted">{if k["memo"]["error"].is_string(){format!("{}: {}",t("Last attempt failed"),k["memo"]["error"].as_str().unwrap_or(""))}else{t("Open the code to fix or run it; it goes live once it runs cleanly.")}}</p>}
            <label class="form-label">{t("Input (stdin)")}<textarea data-testid="skill-input" rows="2" value={self.inputs.get(&id).cloned().unwrap_or_default()} oninput={link.callback(move |e:InputEvent|Msg::Input(inp.clone(),e.target_unchecked_into::<HtmlTextAreaElement>().value()))}/></label>
            <label class="form-label skill-args">{t("Arguments")}<input data-testid="skill-args" placeholder={t("optional, space-separated")} value={self.args.get(&id).cloned().unwrap_or_default()} oninput={link.callback(move |e:InputEvent|Msg::Args(arg.clone(),e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
            <div class="skill-actions">
                <button class="primary-button" data-testid="skill-run" disabled={running||!available||building} onclick={link.callback(move |_|Msg::Run(run.clone()))}>{if running{html!{<span class="spinner"/>}}else{icon("play")}}{t("Run skill")}</button>
                <button class="text-button" data-testid="skill-open" onclick={link.callback(move |_|Msg::Open(open.clone()))}>{icon("code")}{t("Open code")}</button>
                <button class="text-button" data-testid="skill-edit" onclick={link.callback(move |_|Msg::Edit(edit.clone()))}>{icon("settings")}{t("Rename")}</button>
                <button class="text-button" data-testid="skill-retire" disabled={self.saving} onclick={link.callback(move |_|Msg::Retire(retire.clone()))}>{icon("close")}{t("Retire skill")}</button>
                <button class="text-button delete-action" data-testid="skill-delete" disabled={self.saving} onclick={link.callback(move |_|Msg::Delete(delete.clone()))}>{icon("trash")}{if self.confirm_delete==id{t("Confirm delete")}else{t("Delete with code")}}</button>
            </div>
            if !available{<p class="small-muted">{t("This language's toolchain is not installed on the backend.")}</p>}
            if let Some(out)=output {<div class={classes!("coding-output",if out["success"]==true{"is-ok"}else{"is-failed"})} data-testid="skill-output">
                <div class="coding-output-head">{icon(if out["success"]==true{"check"}else{"close"})}<strong>{if out["timed_out"]==true{t("Timed out")}else if out["success"]==true{t("Succeeded")}else{t("Failed")}}</strong><span>{out["phase"].as_str().unwrap_or("")}{" · "}{tf("exit code {code}", &[("code",out["exit_code"].as_i64().map(|c|c.to_string()).unwrap_or("—".into()))])}{" · "}{out["duration_ms"].as_u64().unwrap_or(0)}{" ms"}</span></div>
                if out["stdout"].as_str().is_some_and(|s|!s.trim().is_empty()){<pre class="coding-stdout">{out["stdout"].as_str().unwrap_or("")}</pre>}
                if out["stderr"].as_str().is_some_and(|s|!s.trim().is_empty()){<pre class="coding-stderr">{out["stderr"].as_str().unwrap_or("")}</pre>}
            </div>}
            </>}
        </article>}
    }
}
