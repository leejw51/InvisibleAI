//! Videos: every uploaded video as a card with a frame thumbnail, played in
//! place, searchable by name, with upload by button or drop, download and
//! delete. Playback streams from the backend with Range support.
use crate::i18n::{t, tf};
use crate::{api::api, icons::icon};
use serde_json::Value;
use web_sys::{DragEvent, HtmlInputElement};
use yew::prelude::*;

pub struct Videos {
    items: Vec<Value>,
    error: String,
    confirm: String,
    playing: String,
    query: String,
    uploading: usize,
    dragging: bool,
}
pub enum Msg {
    Load,
    Loaded(Result<Vec<Value>, String>),
    Query(String),
    Play(String),
    Delete(String),
    Deleted(Result<Value, String>),
    Upload(Vec<web_sys::File>),
    Uploaded(Result<Value, String>),
    Drag(bool),
    /// Tags saved for a clip's message, shown at once on every card of it.
    Tagged(String, Vec<String>),
}
/// Bytes as a short human size.
fn size_label(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{} KB", bytes >> 10)
    } else {
        format!("{bytes} B")
    }
}
fn files_of(list: Option<web_sys::FileList>) -> Vec<web_sys::File> {
    let Some(list) = list else {
        return vec![];
    };
    (0..list.length()).filter_map(|i| list.get(i)).collect()
}
impl Component for Videos {
    type Message = Msg;
    type Properties = crate::i18n::Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        Self {
            items: vec![],
            error: String::new(),
            confirm: String::new(),
            playing: String::new(),
            query: String::new(),
            uploading: 0,
            dragging: false,
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Load => {
                ctx.link()
                    .send_future(async { Msg::Loaded(api("GET", "/api/videos", None).await) });
            }
            Msg::Loaded(r) => match r {
                Ok(v) => self.items = v,
                Err(e) => self.error = e,
            },
            Msg::Query(q) => self.query = q,
            Msg::Play(id) => {
                self.playing = if self.playing == id {
                    String::new()
                } else {
                    id
                };
            }
            Msg::Delete(id) => {
                if self.confirm != id {
                    self.confirm = id;
                } else {
                    self.confirm.clear();
                    if self.playing == id {
                        self.playing.clear();
                    }
                    ctx.link().send_future(async move {
                        Msg::Deleted(api("DELETE", &format!("/api/files/{id}"), None).await)
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
                for item in self.items.iter_mut().filter(|v| v["memo_id"] == memo) {
                    item["tags"] = serde_json::json!(tags);
                }
            }
            Msg::Upload(files) => {
                self.dragging = false;
                let videos: Vec<web_sys::File> = files
                    .into_iter()
                    .filter(|f| {
                        f.type_().starts_with("video/")
                            || f.name().to_lowercase().ends_with(".mp4")
                            || f.name().to_lowercase().ends_with(".mov")
                            || f.name().to_lowercase().ends_with(".webm")
                            || f.name().to_lowercase().ends_with(".m4v")
                    })
                    .collect();
                if videos.is_empty() {
                    self.error = "Choose a video file".into();
                    return true;
                }
                self.error.clear();
                for file in videos {
                    self.uploading += 1;
                    ctx.link().send_future(async move {
                        let name = file.name();
                        // A clip added here is kept in the Wiki as a video. It
                        // goes up in parallel raw slices, up to 1 GB.
                        let result = crate::uploads::upload_video(
                            file.into(),
                            name,
                            crate::uploads::VideoTarget {
                                wiki: true,
                                ..Default::default()
                            },
                            None,
                        )
                        .await
                        .and_then(|f| serde_json::to_value(f).map_err(|e| e.to_string()));
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
        let query = self.query.trim().to_lowercase();
        let shown: Vec<&Value> = self
            .items
            .iter()
            .filter(|v| {
                query.is_empty()
                    || v["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
            })
            .collect();
        let total: u64 = self
            .items
            .iter()
            .map(|v| v["size"].as_u64().unwrap_or(0))
            .sum();
        html! {<section class={classes!("videos-page",self.dragging.then_some("is-dragging"))} data-testid="videos-page"
            ondragover={link.callback(|e:DragEvent|{e.prevent_default();Msg::Drag(true)})}
            ondragleave={link.callback(|_:DragEvent|Msg::Drag(false))}
            ondrop={link.callback(|e:DragEvent|{e.prevent_default();Msg::Upload(files_of(e.data_transfer().and_then(|d|d.files())))})}>
            <div class="page-intro"><div><div class="eyebrow">{icon("film")}{t("YOUR VIDEO LIBRARY")}</div><h1>{t("Every clip, ready to play.")}</h1><p>{t("Upload videos, watch them here, and keep them on your backend. Drop files anywhere on this page.")}</p></div>
                <div class="videos-actions">
                    <label class="primary-button upload-label">{icon("upload")}{if self.uploading>0{t("Uploading…")}else{t("Upload video")}}<input type="file" accept="video/*" multiple=true aria-label={t("Upload video")} onchange={link.callback(|e:Event|{let input=e.target_unchecked_into::<HtmlInputElement>();let files=files_of(input.files());input.set_value("");Msg::Upload(files)})}/></label>
                    <button class="secondary-button" onclick={link.callback(|_|Msg::Load)}>{icon("refresh")}{t("Refresh videos")}</button>
                </div>
            </div>
            <div class="videos-bar">
                <div class="wiki-search videos-search">{icon("search")}<input type="search" aria-label={t("Search videos")} placeholder={t("Search by name")} value={self.query.clone()} oninput={link.callback(|e:InputEvent|Msg::Query(e.target_unchecked_into::<HtmlInputElement>().value()))}/></div>
                <span class="small-muted">{tf("{count} videos · {size}", &[("count",self.items.len().to_string()),("size",size_label(total))])}</span>
            </div>
            if !self.error.is_empty(){<p class="error-banner" role="alert">{t(&self.error)}</p>}
            if self.items.is_empty(){<div class="empty-state">{icon("film")}<h3>{t("No videos yet.")}</h3><p>{t("Upload a video or drop one here to begin.")}</p></div>}
            else if shown.is_empty(){<div class="empty-state">{icon("search")}<h3>{t("No video matches.")}</h3></div>}
            <div class="videos-grid">{for shown.iter().map(|v|{
                let id=v["id"].as_str().unwrap_or("").to_owned();
                let name=v["name"].as_str().unwrap_or("video").to_owned();
                let url=v["url"].as_str().unwrap_or("").to_owned();
                let download=v["download"].as_str().unwrap_or("").to_owned();
                let thumb=v["thumbnail"].as_str().unwrap_or("").to_owned();
                let playing=self.playing==id;
                let confirming=self.confirm==id;
                let (play_id,del_id)=(id.clone(),id.clone());
                let memo=v["memo_id"].as_str().unwrap_or("").to_owned();
                let tags:Vec<String>=v["tags"].as_array().map(|a|a.iter().filter_map(|t|t.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                html!{<article class={classes!("video-card",playing.then_some("is-playing"))} key={id.clone()} data-testid="video-card">
                    if playing {<video controls=true autoplay=true playsinline=true preload="metadata" src={url.clone()} poster={thumb.clone()} aria-label={name.clone()}/>}
                    else {<button type="button" class="video-poster" aria-label={tf("Play {name}", &[("name",name.clone())])} onclick={link.callback(move |_|Msg::Play(play_id.clone()))}><img decoding="async" loading="lazy" src={thumb.clone()} alt=""/><span class="video-play">{icon("play")}</span></button>}
                    <div class="video-meta"><h3 title={name.clone()}>{&name}</h3><small>{size_label(v["size"].as_u64().unwrap_or(0))}{" · "}{crate::short_date(v["created_at"].as_str().unwrap_or(""))}</small>
                        if !memo.is_empty(){<crate::tags::TagEditor language={ctx.props().language} memo_id={memo.clone()} tags={tags} pending=true saved={link.callback({let id=memo.clone();move |tags|Msg::Tagged(id.clone(),tags)})}/>}
                        <div class="file-actions">
                            <a class="text-button" href={download} download={name.clone()}>{icon("download")}{t("Download")}</a>
                            <button class="text-button delete-action" aria-label={if confirming{t("Confirm delete video")}else{t("Delete video")}} onclick={link.callback(move |_|Msg::Delete(del_id.clone()))}>{icon("trash")}{if confirming{t("Confirm delete video")}else{t("Delete video")}}</button>
                        </div>
                    </div>
                </article>}})}</div>
        </section>}
    }
}
