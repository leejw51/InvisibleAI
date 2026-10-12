//! One message's Wiki tags, shown and edited in place: in the Images and
//! Videos libraries and in the Wiki's reader, the same edit as in the chat
//! (`PATCH /api/memos/{id}` with the list). Types and categories are fixed;
//! the rest is the user's to add, change or remove. Saving curates the
//! message, and the Wiki follows on its own (open pages refresh on the
//! backend's `wiki.updated` event).
use crate::i18n::{t, tf};
use crate::{api::api, icons::icon};
use causewaybay_shared::Memo;
use serde_json::json;
use web_sys::HtmlInputElement;
use yew::prelude::*;

/// Tags a user may put on one message by hand. Mirrors the server's limit.
pub const MAX_TAGS: usize = 12;
/// One spelling for a tag, as the server normalizes it: lowercase, no `#`,
/// words joined by `-`, letters, digits, `_` and `/` only, at most 40 characters.
pub fn normalize_tag(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut gap = false;
    for c in raw.trim().trim_start_matches('#').to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' || c == '/' {
            if gap && !out.is_empty() {
                out.push('-');
            }
            out.push(c);
            gap = false;
        } else if c.is_whitespace() || c == '-' || c == '.' || c == ',' {
            gap = true;
        }
    }
    let out: String = out.chars().take(40).collect();
    let out = out.trim_matches(|c| c == '-' || c == '/' || c == '_');
    (!out.is_empty()).then(|| out.to_string())
}

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: crate::i18n::Language,
    /// The message whose tags these are.
    pub memo_id: String,
    pub tags: Vec<String>,
    /// The list as saved, for the owner of the message to show at once.
    pub saved: Callback<Vec<String>>,
    /// What a tap on a tag does: open it where the editor sits (the Wiki),
    /// or, without one, follow a link to the tag's Wiki page.
    #[prop_or_default]
    pub pick: Option<Callback<String>>,
    /// Not yet analyzed: say so next to the offer to add tags by hand.
    #[prop_or_default]
    pub pending: bool,
}
pub enum Msg {
    Edit,
    Input(String),
    Add,
    Remove(String),
    Save,
    Saved(Result<Vec<String>, String>),
    Cancel,
}
pub struct TagEditor {
    editing: bool,
    draft: Vec<String>,
    input: String,
    saving: bool,
    error: String,
}
impl Component for TagEditor {
    type Message = Msg;
    type Properties = Props;
    fn create(_ctx: &Context<Self>) -> Self {
        Self {
            editing: false,
            draft: vec![],
            input: String::new(),
            saving: false,
            error: String::new(),
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Edit => {
                // The type and the category are not edited; they follow the message.
                self.draft = ctx
                    .props()
                    .tags
                    .iter()
                    .filter(|t| !causewaybay_shared::is_fixed_tag(t))
                    .cloned()
                    .collect();
                self.input.clear();
                self.error.clear();
                self.saving = false;
                self.editing = true;
            }
            Msg::Input(v) => {
                // A comma (or Enter) finishes the tag being typed; spaces stay, so
                // "machine learning" becomes one tag, machine-learning.
                if v.ends_with(',') {
                    self.input = v.trim_end_matches(',').to_string();
                    ctx.link().send_message(Msg::Add);
                } else {
                    self.input = v;
                }
            }
            Msg::Add => {
                if let Some(tag) = normalize_tag(&self.input)
                    && !self.draft.contains(&tag)
                    && self.draft.len() < MAX_TAGS
                {
                    self.draft.push(tag);
                }
                self.input.clear();
            }
            Msg::Remove(tag) => self.draft.retain(|t| *t != tag),
            Msg::Save => {
                if self.saving {
                    return false;
                }
                if let Some(tag) = normalize_tag(&self.input)
                    && !self.draft.contains(&tag)
                    && self.draft.len() < MAX_TAGS
                {
                    self.draft.push(tag);
                }
                self.input.clear();
                self.saving = true;
                let url = format!("/api/memos/{}", crate::api::encode(&ctx.props().memo_id));
                let body = json!({"tags":self.draft});
                ctx.link().send_future(async move {
                    Msg::Saved(api::<Memo>("PATCH", &url, Some(body)).await.map(|m| m.tags))
                });
            }
            Msg::Saved(r) => {
                self.saving = false;
                match r {
                    Ok(saved) => {
                        // The memo answers with the user's list; the type and the
                        // category stay as they were, so the chips read as the
                        // Wiki will show them without waiting for a sync.
                        let mut tags: Vec<String> = ctx
                            .props()
                            .tags
                            .iter()
                            .filter(|t| causewaybay_shared::is_fixed_tag(t))
                            .cloned()
                            .collect();
                        for tag in saved {
                            if !tags.contains(&tag) {
                                tags.push(tag);
                            }
                        }
                        self.editing = false;
                        self.draft.clear();
                        ctx.props().saved.emit(tags);
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Cancel => {
                self.editing = false;
                self.draft.clear();
                self.input.clear();
                self.saving = false;
                self.error.clear();
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let p = ctx.props();
        if self.editing {
            let full = self.draft.len() >= MAX_TAGS;
            return html! {<div class="chat-tags is-editing tag-editor" data-testid="tag-editor-form" role="group" aria-label={t("Edit tags")}>
                {for p.tags.iter().filter(|t|causewaybay_shared::is_fixed_tag(t)).map(|tag|html!{<span class={classes!("chat-tag",if causewaybay_shared::is_type(tag){"is-type"}else{"is-category"})} key={tag.clone()} title={if causewaybay_shared::is_type(tag){t("Type")}else{t("Category")}}>{"#"}{tag}</span>})}
                {for self.draft.iter().map(|tag|{let remove=tag.clone();html!{<span class="chat-tag" key={tag.clone()}>{"#"}{tag}<button type="button" class="chat-tag-remove" aria-label={tf("Remove tag {tag}", &[("tag",tag.clone())])} onclick={link.callback(move |_|Msg::Remove(remove.clone()))}>{icon("close")}</button></span>}})}
                <input class="chat-tag-input" type="text" aria-label={t("Add tag")} placeholder={if full{t("Up to 12 tags")}else{t("Add tag, Enter")}} disabled={full} value={self.input.clone()} maxlength="40" autocapitalize="none" autocomplete="off" enterkeyhint="done"
                    oninput={link.callback(|e:InputEvent|Msg::Input(e.target_unchecked_into::<HtmlInputElement>().value()))}
                    onkeydown={link.batch_callback(|e:KeyboardEvent|{if e.key()=="Enter"{e.prevent_default();Some(Msg::Add)}else if e.key()=="Escape"{Some(Msg::Cancel)}else{None}})}/>
                <button type="button" class="primary-button chat-tags-save" data-testid="tag-editor-save" disabled={self.saving} onclick={link.callback(|_|Msg::Save)}>{if self.saving{t("Saving…")}else{t("Save tags")}}</button>
                <button type="button" class="text-button" onclick={link.callback(|_|Msg::Cancel)}>{t("Cancel")}</button>
                if !self.error.is_empty(){<span class="login-error" role="alert">{t(&self.error)}</span>}
            </div>};
        }
        // In the Wiki the chips are the Wiki's own (a tap filters by the tag);
        // elsewhere they are the chat's, and lead to the tag's Wiki page.
        html! {<div class={classes!(if p.pick.is_some(){"wiki-card-tags"}else{"chat-tags"},"tag-editor")} data-testid="tag-editor">
            {for p.tags.iter().map(|tag|{
                let name=tag.clone();
                let chip=if p.pick.is_some(){"wiki-tag"}else{"chat-tag"};
                let class=classes!(chip,causewaybay_shared::is_type(tag).then_some("is-type"),causewaybay_shared::is_category(tag).then_some("is-category"));
                match &p.pick {
                    Some(pick)=>{let pick=pick.clone();html!{<button type="button" key={tag.clone()} class={class} onclick={Callback::from(move |_|pick.emit(name.clone()))}>{"#"}{tag}</button>}}
                    None=>html!{<a key={tag.clone()} class={class} href={format!("/wiki/tag/{}",crate::api::encode(tag))} title={t("Open in Wiki")}>{"#"}{tag}</a>},
                }
            })}
            if p.pending && p.tags.is_empty(){<span class="chat-tags-pending" data-testid="chat-tags-pending">{icon("hash")}{t("Tagging…")}</span>}
            <button type="button" class="text-button chat-tags-edit" data-testid="tag-editor-edit" aria-label={t("Edit tags")} onclick={link.callback(|_|Msg::Edit)}>{icon("hash")}{if p.tags.iter().all(|t|causewaybay_shared::is_fixed_tag(t)){t("Add tags")}else{t("Edit tags")}}</button>
        </div>}
    }
}
