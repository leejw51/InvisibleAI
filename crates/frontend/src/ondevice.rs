//! On-device model picker for Backend settings: Apple's LLM and the Qwen
//! models the backend runs through its Rust MLX worker, with install state,
//! GPU-memory fit, and an eased download progress bar.
use crate::api::api;
use crate::i18n::{Language, t};
use crate::icons::icon;
use crate::tween::{ProgressBar, eta_text, format_bytes};
use causewaybay_shared::{LocalModel, LocalModels};
use gloo_events::EventListener;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub admin: bool,
    /// The provider and model currently chosen in the settings draft.
    pub provider: String,
    pub model: String,
    pub engine: String,
    pub onselect: Callback<(String, String)>,
}
pub enum Msg {
    Loaded(Result<LocalModels, String>),
    Download(String, bool),
    Cancel,
    Done(Result<(), String>),
}
/// The backend pushes catalog snapshots over SSE whenever a download moves or
/// the worker loads, so nothing here polls.
pub struct Picker {
    models: Option<LocalModels>,
    error: String,
    busy: bool,
    stream: Option<(web_sys::EventSource, EventListener)>,
}

fn gb(bytes: u64) -> String {
    format!("{:.0} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}
/// The one-line memory verdict shown under a model.
pub fn fit_text(m: &LocalModel) -> String {
    match m.fit.as_str() {
        "fits" if m.memory_gb > 0.0 => format!(
            "{} {:.1} GB · {}",
            t("uses"),
            m.memory_gb,
            t("fits GPU memory")
        ),
        "tight" => format!(
            "{} {:.1} GB · {}",
            t("uses"),
            m.memory_gb,
            t("tight on GPU memory")
        ),
        "too_large" => format!(
            "{} {:.1} GB · {}",
            t("uses"),
            m.memory_gb,
            t("too large for this Mac")
        ),
        "unknown" if m.memory_gb > 0.0 => format!("{} {:.1} GB", t("uses"), m.memory_gb),
        _ => String::new(),
    }
}

impl Picker {
    fn load(&mut self, ctx: &Context<Self>) {
        let link = ctx.link().clone();
        spawn_local(async move {
            link.send_message(Msg::Loaded(api("GET", "/api/local-ai/models", None).await));
        });
    }
    fn listen(&mut self, ctx: &Context<Self>) {
        let Ok(source) = web_sys::EventSource::new("/api/local-ai/models/events") else {
            return;
        };
        let link = ctx.link().clone();
        let listener = EventListener::new(&source, "message", move |event| {
            if let Some(message) = event.dyn_ref::<web_sys::MessageEvent>()
                && let Some(data) = message.data().as_string()
                && let Ok(models) = serde_json::from_str::<LocalModels>(&data)
            {
                link.send_message(Msg::Loaded(Ok(models)));
            }
        });
        self.stream = Some((source, listener));
    }
}
impl Component for Picker {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        let mut p = Self {
            models: None,
            error: String::new(),
            busy: false,
            stream: None,
        };
        p.load(ctx);
        p.listen(ctx);
        p
    }
    fn destroy(&mut self, _ctx: &Context<Self>) {
        if let Some((source, _)) = self.stream.take() {
            source.close();
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Msg::Loaded(Ok(models)) => self.models = Some(models),
            Msg::Loaded(Err(e)) => self.error = e,
            Msg::Download(id, force) => {
                self.busy = true;
                self.error.clear();
                let link = ctx.link().clone();
                spawn_local(async move {
                    let r: Result<serde_json::Value, String> = api(
                        "POST",
                        "/api/admin/local-ai/models/fetch",
                        Some(json!({"id": id, "force": force})),
                    )
                    .await;
                    link.send_message(Msg::Done(r.map(|_| ())));
                });
            }
            Msg::Cancel => {
                self.busy = true;
                let link = ctx.link().clone();
                spawn_local(async move {
                    let r: Result<serde_json::Value, String> =
                        api("DELETE", "/api/admin/local-ai/models/fetch", None).await;
                    link.send_message(Msg::Done(r.map(|_| ())));
                });
            }
            Msg::Done(r) => {
                self.busy = false;
                if let Err(e) = r {
                    self.error = e;
                }
                self.load(ctx);
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let Some(models) = &self.models else {
            return html! {<section class="ondevice-models" aria-label={t("On-device model")}><p class="small-muted">{if self.error.is_empty(){t("Checking installed models…")}else{self.error.clone()}}</p></section>};
        };
        let memory = match (models.unified_memory_bytes, models.gpu_budget_bytes) {
            (Some(total), Some(budget)) => format!(
                "{} {} · {} {}",
                t("Unified memory"),
                gb(total),
                t("GPU budget"),
                gb(budget)
            ),
            _ => t("GPU memory unknown"),
        };
        html! {<section class="ondevice-models" aria-label={t("On-device model")} data-testid="ondevice-models">
            <h4>{t("On-device model")}</h4>
            if ctx.props().engine != "service" {<p class="ondevice-reassurance">{icon("shield")}{t("AI answers stay on your Mac. Choose 9B for a 16 GB Mac.")}</p>}
            <p class="small-muted ondevice-memory" data-testid="ondevice-memory">{icon("shield")}<span>{memory}{" · "}{if models.worker_available{t("MLX worker ready")}else{t("MLX worker not built: run make mlx-ai on the backend")}}if models.busy||models.waiting>0{<span>{" · "}{if models.busy{t("answering")}else{String::new()}}{if models.waiting>0{format!(" · {} {}",models.waiting,t("waiting"))}else{String::new()}}</span>}</span></p>
            if !self.error.is_empty(){<p class="field-error" role="alert">{t(&self.error)}</p>}
            <div class="model-cards" role="radiogroup" aria-label={t("On-device model")}>
            {for models.models.iter().filter(|m| matches!(m.id.as_str(), "qwen3.5-9b" | "qwen3.8-27b") || (m.provider == ctx.props().provider && (m.provider == "apple" || m.model == ctx.props().model))).map(|m|{
                let selected = m.provider==ctx.props().provider && (m.provider=="apple" || m.model==ctx.props().model);
                let usable = m.provider=="apple" || ctx.props().engine=="service" || m.installed;
                let (provider,model)=(m.provider.clone(),m.model.clone());
                let download = m.download.clone();
                let id = m.id.clone();
                let downloading = download.as_ref().is_some_and(|d|d.active());
                html!{<div class={classes!("model-card",selected.then_some("selected"),(!usable).then_some("not-installed"))} data-testid={format!("model-card-{}",m.id)}>
                    <button type="button" class="model-choice" role="radio" aria-checked={selected.to_string()} onclick={ctx.props().onselect.reform(move |_|(provider.clone(),model.clone()))}>
                        <span class="model-check" aria-hidden="true">{if selected{icon("check")}else{Html::default()}}</span>
                        <strong>{&m.name}</strong>
                        <small>{t(if m.id == "qwen3.5-9b" {"Everyday · Recommended for 16 GB"} else if m.id == "qwen3.8-27b" {"More capable · Needs more memory"} else {&m.note})}</small>
                        <small>{if m.size_gb>0.0{format!("{:.1} GB · {}",m.size_gb,t(&m.note))}else{t(&m.note)}}</small>
                        if !fit_text(m).is_empty(){<small class={classes!("model-fit",m.fit.clone())}>{fit_text(m)}</small>}
                    </button>
                    <div class="model-state">
                        if m.provider=="apple"{<span class="model-badge">{if models.apple_available{t("Ready")}else{t("Needs the Apple worker")}}</span>}
                        else if m.installed{<span class="model-badge installed">{if models.loaded_model.as_deref()==Some(m.name.as_str()){t("Loaded in memory")}else{t("Installed")}}</span>}
                        else if let Some(d)=download.as_ref().filter(|d|d.active()){
                            <ProgressBar fraction={d.fraction()} label={format!("{} {}",t("Downloading"),m.name)} detail={format!("{} / {}{}{}",format_bytes(d.done_bytes),format_bytes(d.total_bytes),if d.bytes_per_second>0{format!(" · {}/s",format_bytes(d.bytes_per_second))}else{String::new()},{let eta=eta_text(d.total_bytes.saturating_sub(d.done_bytes),d.bytes_per_second);if eta.is_empty(){String::new()}else{format!(" · {}",eta)}})}/>
                            <small class="model-file">{if d.file.is_empty(){t("Finishing…")}else{format!("{} {}/{}: {}",t("file"),d.done_files+1,d.total_files,d.file)}}</small>
                            if ctx.props().admin{<button type="button" class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Cancel)}>{t("Cancel download")}</button>}
                        }
                        else{
                            if let Some(d)=download.as_ref(){
                                if let Some(e)=&d.error{<small class="field-error" role="alert">{t(e)}</small>}
                                else if d.cancelled{<small class="small-muted">{t("Download cancelled. Download again to resume.")}</small>}
                            }
                            if ctx.props().admin{
                                if m.fit=="too_large"{<button type="button" class="secondary-button" disabled={self.busy||downloading} onclick={ctx.link().callback({let id=id.clone();move |_|Msg::Download(id.clone(),true)})}>{icon("download")}{t("Download anyway")}</button>}
                                else{<button type="button" class="secondary-button" disabled={self.busy||downloading} onclick={ctx.link().callback(move |_|Msg::Download(id.clone(),false))}>{icon("download")}{t("Download")}</button>}
                            } else {<span class="model-badge">{t("Not installed")}</span>}
                        }
                    </div>
                </div>}
            })}
            </div>
            <p class="small-muted">{t("Models download once, then run on your Mac. 27B needs more memory than a 16 GB Mac can comfortably provide.")}</p>
        </section>}
    }
}
