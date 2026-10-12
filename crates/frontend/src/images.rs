use crate::i18n::t;
use crate::{api::api, icons::icon};
use base64::Engine;
use causewaybay_shared::ImageInfo;
use serde_json::{Value, json};
use web_sys::{DragEvent, HtmlInputElement};
use yew::prelude::*;
pub struct Images {
    items: Vec<ImageInfo>,
    error: String,
    confirm: String,
    uploading: usize,
    dragging: bool,
}
pub enum Msg {
    Load,
    Loaded(Result<Vec<ImageInfo>, String>),
    Delete(ImageInfo),
    Deleted(Result<Value, String>),
    /// Pictures added here directly, by the button or by dropping them on the page.
    Upload(Vec<web_sys::File>),
    Uploaded(Result<Value, String>),
    Drag(bool),
    /// Tags saved for a picture's message, shown at once on every card of it.
    Tagged(String, Vec<String>),
}
fn files_of(list: Option<web_sys::FileList>) -> Vec<web_sys::File> {
    let Some(list) = list else {
        return vec![];
    };
    (0..list.length()).filter_map(|i| list.get(i)).collect()
}
impl Component for Images {
    type Message = Msg;
    type Properties = crate::i18n::Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        Self {
            items: vec![],
            error: String::new(),
            confirm: String::new(),
            uploading: 0,
            dragging: false,
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Load => {
                ctx.link()
                    .send_future(async { Msg::Loaded(api("GET", "/api/images", None).await) });
            }
            Msg::Loaded(r) => match r {
                Ok(v) => self.items = v,
                Err(e) => self.error = e,
            },
            Msg::Delete(i) => {
                if self.confirm != i.id {
                    self.confirm = i.id;
                } else {
                    self.confirm.clear();
                    ctx.link().send_future(async move {
                        Msg::Deleted(
                            api(
                                "DELETE",
                                &if i.source == "file" {
                                    format!("/api/files/{}", i.id)
                                } else {
                                    format!("/api/memos/{}/image", i.id)
                                },
                                None,
                            )
                            .await,
                        )
                    });
                }
            }
            Msg::Deleted(r) => match r {
                Ok(_) => {
                    self.error.clear();
                    ctx.link().send_message(Msg::Load);
                }
                Err(e) => self.error = e,
            },
            Msg::Drag(on) => self.dragging = on,
            Msg::Tagged(memo, tags) => {
                for item in self.items.iter_mut().filter(|i| i.memo_id == memo) {
                    item.tags = tags.clone();
                }
            }
            Msg::Upload(files) => {
                self.dragging = false;
                let pictures: Vec<web_sys::File> = files
                    .into_iter()
                    .filter(|f| f.type_().starts_with("image/"))
                    .collect();
                if pictures.is_empty() {
                    self.error = "Choose an image file".into();
                    return true;
                }
                self.error.clear();
                for file in pictures {
                    self.uploading += 1;
                    ctx.link().send_future(async move {
                        let result = async {
                            if file.size() > 25.0 * 1024.0 * 1024.0 {
                                return Err("Uploads must be below 25 MB".to_string());
                            }
                            let bytes = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
                                .await
                                .map_err(|_| "Could not read file".to_string())?;
                            let data = js_sys::Uint8Array::new(&bytes).to_vec();
                            api::<Value>(
                                "POST",
                                "/api/files",
                                // A picture added here is kept in the Wiki as a photo.
                                Some(json!({"name":file.name(),"data_base64":base64::engine::general_purpose::STANDARD.encode(data),"wiki":true})),
                            )
                            .await
                        }
                        .await;
                        Msg::Uploaded(result)
                    });
                }
            }
            Msg::Uploaded(r) => {
                self.uploading = self.uploading.saturating_sub(1);
                match r {
                    Ok(_) => ctx.link().send_message(Msg::Load),
                    Err(e) => self.error = e,
                }
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        html! {<section class={classes!("images-page",self.dragging.then_some("is-dragging"))} data-testid="images-page"
            ondragover={link.callback(|e:DragEvent|{e.prevent_default();Msg::Drag(true)})}
            ondragleave={link.callback(|_:DragEvent|Msg::Drag(false))}
            ondrop={link.callback(|e:DragEvent|{e.prevent_default();Msg::Upload(files_of(e.data_transfer().and_then(|d|d.files())))})}>
            <div class="page-intro"><div><div class="eyebrow">{icon("screen")}{t("YOUR VISUAL LIBRARY")}</div><h1>{t("Every captured moment.")}</h1><p>{t("Screenshots and uploaded images, stored on your backend. Drop pictures anywhere on this page.")}</p></div>
                <div class="videos-actions">
                    <label class="primary-button upload-label">{icon("upload")}{if self.uploading>0{t("Uploading…")}else{t("Upload image")}}<input type="file" accept="image/*" multiple=true aria-label={t("Upload image")} onchange={link.callback(|e:Event|{let input=e.target_unchecked_into::<HtmlInputElement>();let files=files_of(input.files());input.set_value("");Msg::Upload(files)})}/></label>
                    <button class="secondary-button" onclick={link.callback(|_|Msg::Load)}>{icon("refresh")}{t("Refresh images")}</button>
                </div>
            </div>
            if !self.error.is_empty(){<p class="error-banner" role="alert">{t(&self.error)}</p>}
            if self.items.is_empty(){<div class="empty-state">{icon("screen")}<h3>{t("No images yet.")}</h3><p>{t("Capture a screen, upload an image, or drop one here to begin.")}</p></div>}
            <div class="images-grid">{for self.items.iter().map(|i|{let item=i.clone();html!{<article class="image-card"><a href={if i.source=="file"{format!("/api/files/{}/image",i.id)}else{i.url.clone()}} target="_blank" rel="noreferrer"><img decoding="async" src={format!("/api/{}/{}/thumbnail",if i.source=="file"{"files"}else{"memos"},i.id)} loading="lazy" alt={i.name.clone()}/></a><div><h3>{&i.name}</h3><small>{if i.source=="file"{t("Uploaded image")}else{t("Screenshot")}}</small>if !i.memo_id.is_empty(){<crate::tags::TagEditor language={ctx.props().language} memo_id={i.memo_id.clone()} tags={i.tags.clone()} pending=true saved={link.callback({let id=i.memo_id.clone();move |tags|Msg::Tagged(id.clone(),tags)})}/>}<div class="file-actions"><a class="text-button" href={i.url.clone()} download={i.name.clone()}>{icon("download")}{t("Download")}</a><button class="text-button delete-action" onclick={ctx.link().callback(move |_|Msg::Delete(item.clone()))}>{icon("trash")}{if self.confirm==i.id{t("Confirm delete image")}else{t("Delete image")}}</button></div></div></article>}})}</div>
        </section>}
    }
}
