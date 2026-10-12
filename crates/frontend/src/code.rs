//! Code: everything kept with fenced code, split into its functions. Each card
//! is one function: Input (its parameters), Output (what it returns), the code
//! to copy, and a note the user writes. Cards link back to the Wiki message
//! they came from; search covers names, signatures, notes, code and tags.
use crate::{
    api::{api, encode},
    i18n::{Language, t, tf},
    icons::icon,
};
use gloo_timers::callback::Timeout;
use serde_json::{Value, json};
use web_sys::{HtmlInputElement, HtmlTextAreaElement};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub copy: Callback<String>,
}
pub struct Code {
    data: Value,
    error: String,
    query: String,
    lang: String,
    offset: usize,
    /// Notes being edited, by function id, until they are saved on blur.
    drafts: std::collections::HashMap<String, String>,
    /// Ids whose note was just saved (a short "Saved" beside the box).
    saved: std::collections::HashSet<String>,
    epoch: u64,
    debounce: Option<Timeout>,
}
pub enum Msg {
    Load,
    Loaded(u64, Result<Value, String>),
    Query(String),
    ClearQuery,
    Lang(String),
    Offset(usize),
    Note(String, String),
    SaveNote(String),
    Saved(String, Result<Value, String>),
}
impl Component for Code {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        Self {
            data: Value::Null,
            error: String::new(),
            query: String::new(),
            lang: String::new(),
            offset: 0,
            drafts: Default::default(),
            saved: Default::default(),
            epoch: 0,
            debounce: None,
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Load => {
                self.epoch += 1;
                let epoch = self.epoch;
                let url = format!(
                    "/api/code?q={}&lang={}&offset={}",
                    encode(&self.query),
                    encode(&self.lang),
                    self.offset
                );
                ctx.link()
                    .send_future(async move { Msg::Loaded(epoch, api("GET", &url, None).await) });
                false
            }
            Msg::Loaded(epoch, r) => {
                if epoch != self.epoch {
                    return false;
                }
                match r {
                    Ok(v) => {
                        self.data = v;
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Query(q) => {
                self.query = q;
                self.offset = 0;
                let link = ctx.link().clone();
                self.debounce = Some(Timeout::new(200, move || link.send_message(Msg::Load)));
                true
            }
            Msg::ClearQuery => {
                self.query.clear();
                self.offset = 0;
                ctx.link().send_message(Msg::Load);
                true
            }
            Msg::Lang(lang) => {
                self.lang = lang;
                self.offset = 0;
                ctx.link().send_message(Msg::Load);
                true
            }
            Msg::Offset(o) => {
                self.offset = o;
                ctx.link().send_message(Msg::Load);
                true
            }
            Msg::Note(id, text) => {
                self.saved.remove(&id);
                self.drafts.insert(id, text);
                true
            }
            Msg::SaveNote(id) => {
                let Some(text) = self.drafts.get(&id).cloned() else {
                    return false;
                };
                let url = format!("/api/code/{id}");
                ctx.link().send_future(async move {
                    Msg::Saved(
                        id.clone(),
                        api("PATCH", &url, Some(json!({"description":text}))).await,
                    )
                });
                false
            }
            Msg::Saved(id, r) => {
                match r {
                    Ok(v) => {
                        // The listing carries the note from now on; the draft has done its job.
                        if let Some(items) = self.data["items"].as_array_mut()
                            && let Some(item) = items.iter_mut().find(|i| i["id"] == id.as_str())
                        {
                            item["description"] = v["description"].clone();
                        }
                        self.drafts.remove(&id);
                        self.saved.insert(id);
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
                true
            }
        }
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let items = self.data["items"].as_array().cloned().unwrap_or_default();
        let total = self.data["total"].as_u64().unwrap_or(0) as usize;
        let size = self.data["page_size"].as_u64().unwrap_or(60) as usize;
        let languages = self.data["languages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let searching = !self.query.trim().is_empty() || !self.lang.is_empty();
        html! {<section class="code-page" data-testid="code-page">
            <div class="page-intro"><div><div class="eyebrow">{icon("code")}{t("REUSABLE CODE, FUNCTION BY FUNCTION")}</div><h1>{t("Every function, ready to take.")}</h1><p>{t("Code you keep is split into its functions. Each shows what goes in, what comes out, and the code to copy. Add a note; everything stays on this computer.")}</p></div></div>
            <div class="code-bar">
                <div class="wiki-search code-search">{icon("search")}<input type="search" aria-label={t("Search code")} placeholder={t("Function, type, note, #tag")} value={self.query.clone()} oninput={link.callback(|e:InputEvent|Msg::Query(e.target_unchecked_into::<HtmlInputElement>().value()))}/>if !self.query.is_empty(){<button class="wiki-clear" aria-label={t("Clear code search")} onclick={link.callback(|_|Msg::ClearQuery)}>{icon("close")}</button>}</div>
                if !languages.is_empty() {<div class="code-languages" role="group" aria-label={t("Language")}>
                    <button type="button" class={classes!("chip-button",self.lang.is_empty().then_some("is-active"))} aria-pressed={self.lang.is_empty().to_string()} onclick={link.callback(|_|Msg::Lang(String::new()))}>{t("All")}</button>
                    {for languages.iter().map(|l|{let name=l["language"].as_str().unwrap_or("").to_owned();let active=name==self.lang;let pick=name.clone();html!{<button type="button" key={name.clone()} class={classes!("chip-button",active.then_some("is-active"))} aria-pressed={active.to_string()} onclick={link.callback(move |_|Msg::Lang(pick.clone()))}>{name}<small>{l["count"].as_u64().unwrap_or(0)}</small></button>}})}
                </div>}
                <span class="small-muted" data-testid="code-count">{tf("{count} functions", &[("count",total.to_string())])}</span>
            </div>
            if !self.error.is_empty(){<p class="error-banner" role="alert">{t(&self.error)}</p>}
            if self.data.is_null(){<div class="loading-state"><span class="spinner"/>{t("Reading your code…")}</div>}
            else if items.is_empty(){<div class="empty-state">{icon("code")}<h3>{if searching{t("No function matches.")}else{t("No code kept yet.")}}</h3><p>{if searching{t("Try the function's name, a type, or a #tag.")}else{t("Paste code into New thought or a chat. Every function in it appears here, ready to copy.")}}</p>if !searching{<a class="secondary-button" href="/">{t("Keep some code")}{icon("arrow")}</a>}</div>}
            else {<div class="code-grid">{for items.iter().map(|item| self.card(ctx,item))}</div>}
            if total>size {<div class="wiki-pagination"><button disabled={self.offset==0} onclick={link.callback({let o=self.offset.saturating_sub(size);move |_|Msg::Offset(o)})}>{t("Previous")}</button><span>{tf("{start}–{end} of {total} functions", &[("start",(self.offset+1).to_string()),("end",(self.offset+items.len()).min(total).to_string()),("total",total.to_string())])}</span><button disabled={self.offset+size>=total} onclick={link.callback({let o=self.offset+size;move |_|Msg::Offset(o)})}>{t("Next")}</button></div>}
        </section>}
    }
}
impl Code {
    /// One function: name, Input, Output, the code, a note, and where it came from.
    fn card(&self, ctx: &Context<Self>, item: &Value) -> Html {
        let id = item["id"].as_str().unwrap_or("").to_owned();
        let name = item["name"].as_str().unwrap_or("").to_owned();
        let language = item["language"].as_str().unwrap_or("").to_owned();
        let signature = item["signature"].as_str().unwrap_or("").to_owned();
        let returns = item["returns"].as_str().unwrap_or("").to_owned();
        let code = item["code"].as_str().unwrap_or("").to_owned();
        let params = item["params"].as_array().cloned().unwrap_or_default();
        let tags = item["tags"].as_array().cloned().unwrap_or_default();
        let note = self
            .drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| item["description"].as_str().unwrap_or("").to_owned());
        let lines = code.lines().count();
        let copy = ctx.props().copy.reform({
            let code = code.clone();
            move |_| code.clone()
        });
        let (note_id, save_id) = (id.clone(), id.clone());
        html! {<article key={id.clone()} class="code-card" data-testid="code-card" aria-label={name.clone()}>
            <header class="code-head"><h3>{icon("code")}{&name}</h3><span class="code-lang">{&language}</span><time datetime={item["created_at"].as_str().unwrap_or("").to_owned()}>{crate::short_date(item["created_at"].as_str().unwrap_or(""))}</time></header>
            if !signature.is_empty() {<p class="code-signature"><code>{signature}</code></p>}
            <div class="code-io">
                <section data-testid="code-input"><h4>{icon("arrow")}{t("Input")}</h4>
                    if params.is_empty(){<p class="small-muted">{t("No parameters")}</p>}
                    else {<ul>{for params.iter().map(|p|{let n=p["name"].as_str().unwrap_or("").to_owned();let ty=p["type"].as_str().unwrap_or("").to_owned();html!{<li key={n.clone()}><code>{n}</code>if !ty.is_empty(){<span>{ty}</span>}</li>}})}</ul>}
                </section>
                <section data-testid="code-output"><h4>{icon("arrow")}{t("Output")}</h4><p>if returns.is_empty(){<span class="small-muted">{t("Nothing returned")}</span>}else{<code>{returns}</code>}</p></section>
            </div>
            <details class="code-source" open={lines<=24}><summary>{icon("file")}{t("Code")}<span>{tf("{count} lines", &[("count",lines.to_string())])}</span></summary><pre><code>{code}</code></pre></details>
            <label class="code-note">{t("Your note")}if self.saved.contains(&id){<small data-testid="code-note-saved">{icon("check")}{t("Saved")}</small>}<textarea aria-label={tf("Note for {name}", &[("name",name.clone())])} placeholder={t("What it is for, how to call it…")} value={note} rows="2" oninput={ctx.link().callback(move |e:InputEvent|Msg::Note(note_id.clone(),e.target_unchecked_into::<HtmlTextAreaElement>().value()))} onblur={ctx.link().callback(move |_|Msg::SaveNote(save_id.clone()))}/></label>
            <footer class="code-foot">
                <button type="button" class="secondary-button" data-testid="code-copy" onclick={copy}>{icon("copy")}{t("Copy code")}</button>
                <a class="text-button code-from" href={item["memo_url"].as_str().unwrap_or("/wiki").to_owned()} data-testid="code-from">{icon("book")}<span>{item["memo_title"].as_str().unwrap_or("")}</span>{icon("arrow")}</a>
                <span class="code-tags">{for tags.iter().filter_map(|v|v.as_str()).map(|tag|html!{<span key={tag.to_owned()} class="wiki-tag">{"#"}{tag}</span>})}</span>
            </footer>
        </article>}
    }
}
