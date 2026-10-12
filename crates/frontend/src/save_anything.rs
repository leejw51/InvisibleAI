//! One place to keep text and files. Uses the existing durable memo/tidy pipeline.
use crate::{
    api::api,
    i18n::{Language, t},
    icons::icon,
};
use base64::Engine;
use causewaybay_shared::{FileInfo, Memo};
use serde_json::json;
use yew::prelude::*;
#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub data_epoch: i64,
}
pub enum Msg {
    Text(String),
    Upload(Vec<web_sys::File>),
    Uploaded(Vec<Result<FileInfo, String>>),
    Remove(String),
    Save,
    Saved(Box<Result<Memo, String>>),
}
#[derive(Default)]
pub struct SaveAnything {
    text: String,
    files: Vec<FileInfo>,
    busy: bool,
    error: String,
    saved: Option<Memo>,
}
impl Component for SaveAnything {
    type Message = Msg;
    type Properties = Props;
    fn create(_: &Context<Self>) -> Self {
        Self::default()
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Text(text) => self.text = text,
            Msg::Remove(id) => self.files.retain(|f| f.id != id),
            Msg::Upload(files) => {
                if self.busy {
                    return false;
                }
                if files.len() + self.files.len() > 8 {
                    self.error = "Attach up to 8 photos or files per message".into();
                    return true;
                }
                self.busy = true;
                self.error.clear();
                self.saved = None;
                let epoch = ctx.props().data_epoch;
                ctx.link().send_future(async move {
     let mut results=Vec::new();
     for file in files {
      results.push(async {
       if file.size()>25.0*1024.0*1024.0 {return Err("Uploads must be below 25 MB".into());}
       let buffer=wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(|_|"Could not read file")?;
       let bytes=js_sys::Uint8Array::new(&buffer).to_vec();
       api("POST","/api/files",Some(json!({"name":file.name(),"data_base64":base64::engine::general_purpose::STANDARD.encode(bytes),"data_epoch":epoch}))).await
      }.await);
     }
     Msg::Uploaded(results)
    });
            }
            Msg::Uploaded(results) => {
                self.busy = false;
                for r in results {
                    match r {
                        Ok(file) => self.files.push(file),
                        Err(e) => self.error = e,
                    }
                }
            }
            Msg::Save => {
                if self.busy || (self.text.trim().is_empty() && self.files.is_empty()) {
                    return false;
                }
                self.busy = true;
                self.error.clear();
                self.saved = None;
                let body = json!({"content":self.text,"file_ids":self.files.iter().map(|f|&f.id).collect::<Vec<_>>(),"data_epoch":ctx.props().data_epoch});
                ctx.link().send_future(async move {
                    Msg::Saved(Box::new(api("POST", "/api/memos", Some(body)).await))
                });
            }
            Msg::Saved(result) => {
                self.busy = false;
                match *result {
                    Ok(m) => {
                        self.saved = Some(m);
                        self.text.clear();
                        self.files.clear();
                    }
                    Err(e) => self.error = e,
                }
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        html! {<section class="save-anything">
         <h1>{t("Save anything")}</h1><p>{t("A thought, a link, a photo, a file. Drop it here — I’ll keep it organized.")}</p>
         <label class="sr-only" for="anything-input">{t("What would you like to keep?")}</label>
         <textarea id="anything-input" placeholder={t("What would you like to keep?")} value={self.text.clone()} disabled={self.busy} oninput={ctx.link().callback(|e:InputEvent|Msg::Text(e.target_unchecked_into::<web_sys::HtmlTextAreaElement>().value()))}/>
         if !self.files.is_empty(){<ul class="anything-files">{for self.files.iter().map(|f|{let id=f.id.clone();html!{<li>{icon("file")}<span>{&f.name}</span><button class="text-button" disabled={self.busy} aria-label={format!("{} {}",t("Remove"),f.name)} onclick={ctx.link().callback(move |_|Msg::Remove(id.clone()))}>{icon("close")}</button></li>}})}</ul>}
         <div class="anything-actions"><label class="secondary-button upload-label">{icon("plus")}{t("Add any file")}<input type="file" multiple=true aria-label={t("Add any file")} disabled={self.busy} onchange={ctx.link().callback(|e:Event|{let i=e.target_unchecked_into::<web_sys::HtmlInputElement>();let files=i.files().map(|f|(0..f.length()).filter_map(|n|f.get(n)).collect()).unwrap_or_default();i.set_value("");Msg::Upload(files)})}/></label>
         <button class="primary-button" disabled={self.busy||(self.text.trim().is_empty()&&self.files.is_empty())} onclick={ctx.link().callback(|_|Msg::Save)}>{if self.busy{html!{<span class="spinner"/>}}else{icon("check")}}{t(if self.busy{"Saving…"}else{"Save it"})}</button></div>
         <p class="small-muted">{t("The original is saved first. AI organizes it when available. Find it anytime in Wiki.")}</p>
         if !self.error.is_empty(){<p class="field-error" role="alert">{t(&self.error)}</p>}
         if let Some(m)=&self.saved{<div class="anything-saved" role="status">{icon("check")}<span>{t("Saved! You can leave the rest to me.")}</span><a href={format!("/memos/{}",m.id)}>{t("View saved item")}{icon("arrow")}</a>if !m.input.trim().is_empty(){<a href="/notes">{t("Open Notes")}{icon("arrow")}</a>}</div>}
        </section>}
    }
}
