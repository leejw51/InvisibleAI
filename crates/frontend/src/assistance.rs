use crate::{api::api, i18n::t, icons::icon};
use causewaybay_shared::AssistancePreferences;
use gloo_timers::callback::Interval;
use serde_json::{Value, json};
use web_sys::{HtmlInputElement, HtmlSelectElement};
use yew::prelude::*;

pub struct Assistance {
    prefs: Option<AssistancePreferences>,
    original: Option<AssistancePreferences>,
    patch: Value,
    pause: String,
    busy: bool,
    error: String,
    saved: bool,
    epoch: u64,
    _poll: Interval,
}
pub enum Msg {
    Load,
    Loaded(u64, Result<AssistancePreferences, String>),
    Change(&'static str, Value),
    Pause(String),
    Discard,
    Save,
    Saved(Result<AssistancePreferences, String>),
}
impl Component for Assistance {
    type Message = Msg;
    type Properties = crate::i18n::Props;
    fn create(ctx: &Context<Self>) -> Self {
        let link = ctx.link().clone();
        link.send_message(Msg::Load);
        Self {
            prefs: None,
            original: None,
            patch: json!({}),
            pause: String::new(),
            busy: false,
            error: String::new(),
            saved: false,
            epoch: 0,
            _poll: Interval::new(5000, move || link.send_message(Msg::Load)),
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Load => {
                if self.busy || self.patch.as_object().is_some_and(|p| !p.is_empty()) {
                    return false;
                }
                self.epoch += 1;
                let epoch = self.epoch;
                ctx.link().send_future(async move {
                    Msg::Loaded(epoch, api("GET", "/api/assistance/preferences", None).await)
                });
                return false;
            }
            Msg::Loaded(epoch, result) => {
                if epoch != self.epoch
                    || self.busy
                    || self.patch.as_object().is_some_and(|p| !p.is_empty())
                {
                    return false;
                }
                match result {
                    Ok(p) => {
                        self.pause = seconds(p.textedit_settle_ms);
                        self.original = Some(p.clone());
                        self.prefs = Some(p);
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Change(key, value) => {
                if self.busy {
                    return false;
                }
                self.set_patch(key, value.clone());
                if let Some(p) = &self.prefs {
                    let mut v = json!(p);
                    v[key] = value;
                    if let Ok(p) = serde_json::from_value(v) {
                        self.prefs = Some(p);
                    }
                }
                self.saved = false;
                self.error.clear();
            }
            Msg::Pause(value) => {
                if self.busy {
                    return false;
                }
                self.set_patch("textedit_settle_ms", json!(milliseconds(&value)));
                self.pause = value;
                self.saved = false;
                self.error.clear();
            }
            Msg::Discard => {
                if self.busy {
                    return false;
                }
                if let Some(p) = &self.original {
                    self.pause = seconds(p.textedit_settle_ms);
                    self.prefs = Some(p.clone());
                }
                self.patch = json!({});
                self.error.clear();
                self.saved = false;
                ctx.link().send_message(Msg::Load);
            }
            Msg::Save => {
                if self.busy {
                    return false;
                }
                if milliseconds(&self.pause).is_none() {
                    self.error = "Choose a typing pause between 0.5 and 10 seconds.".into();
                    return true;
                }
                self.busy = true;
                self.epoch += 1;
                let patch = self.patch.clone();
                ctx.link().send_future(async move {
                    Msg::Saved(api("PATCH", "/api/assistance/preferences", Some(patch)).await)
                });
            }
            Msg::Saved(r) => {
                self.busy = false;
                match r {
                    Ok(p) => {
                        self.pause = seconds(p.textedit_settle_ms);
                        self.original = Some(p.clone());
                        self.prefs = Some(p);
                        self.patch = json!({});
                        self.error.clear();
                        self.saved = true;
                    }
                    Err(e) => self.error = e,
                }
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let Some(p) = &self.prefs else {
            return html! {<p role="status">{if self.error.is_empty(){t("Loading assistance settings…")} else{t(&self.error)}}</p>};
        };
        let check = |key| {
            ctx.link().callback(move |e: Event| {
                Msg::Change(
                    key,
                    json!(e.target_unchecked_into::<HtmlInputElement>().checked()),
                )
            })
        };
        let select = |key| {
            ctx.link().callback(move |e: Event| {
                Msg::Change(
                    key,
                    json!(e.target_unchecked_into::<HtmlSelectElement>().value()),
                )
            })
        };
        let dirty = self.patch.as_object().is_some_and(|p| !p.is_empty());
        html! {<section class="assistance-page">
            <header class="feature-intro"><div><span class="eyebrow">{t("MADE FOR THE WAY YOU THINK")} </span><h1>{t("A little help, ")} <em>{t("your way.")} </em></h1><p>{t("Choose what to remember. Choose how to be helped.")} </p><span class="feature-trust">{icon("shield")}{t("Your settings, saved across devices")} </span></div><img src="/assets/invisible-ai-hero.png" alt="" width="1536" height="1024"/></header>
            <nav class="assistance-jumps" aria-label={t("Assistance sections")}><a href="#wiki-settings">{icon("book")}<span>{t("Wiki")} </span><small>{if p.wiki_enabled{t("On")} else{t("Off")} }</small>{icon("down")}</a><a href="#textedit-settings">{icon("file")}<span>{t("TextEdit")} </span><small>{if p.textedit_enabled{t("On")} else{t("Off")} }</small>{icon("down")}</a></nav>
            <div class="assistance-grid">
                <section id="wiki-settings" tabindex="-1" class="assistance-card" aria-label={t("Wiki settings")} data-enabled={p.wiki_enabled.to_string()}>
                    <div class="assistance-card-heading"><span class="feature-mark">{icon("book")}</span><label class="setting-switch"><span>{if p.wiki_enabled{t("On")} else{t("Off")} }</span><span class="switch-control"><input type="checkbox" role="switch" aria-label={t("Enable Wiki")} checked={p.wiki_enabled} disabled={self.busy} onchange={check("wiki_enabled")}/><span class="switch-track" aria-hidden="true"/></span></label></div>
                    <h2>{t("Your personal Wiki")} </h2><p class="card-description">{t("A home for the things you learn. Organized, searchable, and linked to their sources.")} </p>
                    <div class="assistance-fields">
                        <label class="setting-row"><span><strong>{t("Update Wiki automatically")} </strong><small>{t("Keep new notes and captures organized.")} </small></span><span class="switch-control"><input type="checkbox" role="switch" checked={p.wiki_auto_update} disabled={self.busy} onchange={check("wiki_auto_update")}/><span class="switch-track" aria-hidden="true"/></span></label>
                        <label class="setting-field"><span>{t("Organize Wiki by")} </span><select value={p.wiki_group_by.clone()} disabled={self.busy} onchange={select("wiki_group_by")}><option value="topic" selected={p.wiki_group_by=="topic"}>{t("Topics from hashtags")} </option><option value="day" selected={p.wiki_group_by=="day"}>{t("Day captured")} </option></select></label>
                        <label class="setting-row"><span><strong>{t("Include AI answers")} </strong><small>{t("Shown separately from your original text.")} </small></span><span class="switch-control"><input type="checkbox" role="switch" checked={p.wiki_include_answers} disabled={self.busy} onchange={check("wiki_include_answers")}/><span class="switch-track" aria-hidden="true"/></span></label>
                    </div>
                    <div class="card-reassurance">{icon("leaf")}<span>{t("Organized locally. No extra AI calls.")} </span></div>
                    <details class="feature-details"><summary>{t("How your Wiki works")} {icon("down")}</summary><p>{t("Untagged sources appear in Inbox. Turning updates off keeps your pages. Source edits and deletions always carry through.")} </p></details>
                    <a class="feature-link" href="/wiki">{t("Open Wiki")} {icon("arrow")}</a>
                </section>
                <section id="textedit-settings" tabindex="-1" class="assistance-card textedit-card" aria-label={t("TextEdit settings")} data-enabled={p.textedit_enabled.to_string()}>
                    <div class="assistance-card-heading"><span class="feature-mark">{icon("file")}</span><label class="setting-switch"><span>{if p.textedit_enabled{t("On")} else{t("Off")} }</span><span class="switch-control"><input type="checkbox" role="switch" aria-label={t("Enable TextEdit assistance")} checked={p.textedit_enabled} disabled={self.busy} onchange={check("textedit_enabled")}/><span class="switch-track" aria-hidden="true"/></span></label></div>
                    <h2>{t("Help where you write")} </h2><p class="card-description">{t("Write a thought in TextEdit. Get a little clarity, right underneath.")} </p>
                    <div class="assistance-fields">
                        <label class="setting-field pause-setting"><span>{t("Typing pause")} <small>{t("A moment to finish your thought.")} </small></span><span class="duration-input"><input aria-label={t("Typing pause (seconds)")} aria-describedby="pause-help" type="number" min="0.5" max="10" step="0.1" value={self.pause.clone()} disabled={self.busy} oninput={ctx.link().callback(|e:InputEvent|Msg::Pause(e.target_unchecked_into::<HtmlInputElement>().value()))}/><span aria-hidden="true">{t("sec")} </span></span></label>
                        <p class="field-help" id="pause-help">{t("Choose between 0.5 and 10 seconds.")} </p>
                        <label class="setting-field"><span>{t("Answer length")} </span><select value={p.textedit_style.clone()} disabled={self.busy} onchange={select("textedit_style")}><option value="concise" selected={p.textedit_style=="concise"}>{t("Short and practical")} </option><option value="detailed" selected={p.textedit_style=="detailed"}>{t("Detailed with examples")} </option></select></label>
                    </div>
                    <div class="card-reassurance">{icon("shield")}<span>{t("Auto starts only when you turn it on.")} </span></div>
                    <p class="setting-note">{t("Changing these settings pauses Auto. Turn it on again when you’re ready.")} </p>
                    <details class="feature-details"><summary>{t("Set up your Mac")} {icon("down")}</summary><p>{t("Use a plain text document in TextEdit. On your capture Mac, open System Settings → Privacy & Security → Automation and allow the CLI’s host app to control TextEdit. Your admin sets the capture interval in Backend settings.")} </p></details>
                    <a class="feature-link" href="/">{t("Back to your workspace")} {icon("arrow")}</a>
                </section>
            </div>
            <div class={classes!("assistance-save",dirty.then_some("has-changes"),self.saved.then_some("is-saved"))}><div class="save-state" role="status">{icon(if dirty{"clock"}else{"check"})}<span>{if self.busy{t("Saving your preferences…")} else if dirty{t("Unsaved changes")} else if self.saved{t("Saved to your account")} else{t("All changes saved")} }<small>{t("Only you decide how Invisible AI helps.")} </small></span></div><div class="save-actions">if dirty{<button class="discard-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Discard)}>{t("Discard")} </button>}<button class="primary-button" aria-label={t("Save assistance settings")} disabled={self.busy||!dirty} onclick={ctx.link().callback(|_|Msg::Save)}>{if self.busy{t("Saving…")} else{t("Save changes")} }{icon("check")}</button></div>
            if !self.error.is_empty(){<p class="notes-error" role="alert">{t(&self.error)}</p>}
            </div>
        </section>}
    }
}

fn seconds(ms: u64) -> String {
    (ms as f64 / 1000.0).to_string()
}
fn milliseconds(value: &str) -> Option<u64> {
    let seconds = value.parse::<f64>().ok()?;
    (seconds.is_finite() && (0.5..=10.0).contains(&seconds))
        .then(|| (seconds * 1000.0).round() as u64)
}
impl Assistance {
    fn set_patch(&mut self, key: &str, value: Value) {
        if self
            .original
            .as_ref()
            .is_some_and(|p| json!(p)[key] == value)
        {
            self.patch.as_object_mut().unwrap().remove(key);
        } else {
            self.patch[key] = value;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn friendly_seconds_preserve_backend_milliseconds() {
        for ms in [500, 1200, 2500, 9999, 10000] {
            assert_eq!(milliseconds(&seconds(ms)), Some(ms));
        }
        for invalid in ["", "NaN", "inf", "1e999", "-1", "0.49", "10.1"] {
            assert_eq!(milliseconds(invalid), None);
        }
    }
}
