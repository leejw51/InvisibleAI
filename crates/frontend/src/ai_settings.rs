//! One AI choice for the whole browser: which AI answers (a local model, a
//! cloud provider, or the Codex / Claude Code tools) and which model. The user
//! decides; the pick is kept in this browser's local storage, mirrored to the
//! account so backend sessions and the policy checks follow it, and the same
//! picker appears wherever AI is used (the AI settings page, Chat, Coding).
use crate::{
    api::api,
    i18n::{Language, t},
    icons::icon,
};
use causewaybay_shared::{Settings, UserAiPreferences};
use serde_json::{Value, json};
use web_sys::{HtmlInputElement, HtmlSelectElement};
use yew::prelude::*;

const HYBRID_HELP: &str = "Hybrid keeps Chat and remote requests on-device. Auto can request cloud help. Coding can fall back to cloud only when cloud confirmation is off.";

fn key(owner: &str) -> String {
    format!("causewaybay.chat.{owner}")
}
/// This browser's remembered pick: `{provider, models:{provider:model}, input_mode}`.
pub fn preferences(owner: &str) -> Value {
    let mut v = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(&key(owner)).ok().flatten())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}));
    if !v["models"].is_object() {
        v["models"] = json!({});
    }
    v
}
/// Remember a provider and its model in this browser, keeping the other
/// fields (the chat's reading mode) as they are. False when storage is off.
pub fn remember(owner: &str, provider: &str, model: Option<&str>) -> bool {
    let mut v = preferences(owner);
    v["provider"] = json!(provider);
    if let Some(model) = model
        && !matches!(provider, "apple" | "mlx")
    {
        v["models"][provider] = json!(model);
    }
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .is_some_and(|s| s.set_item(&key(owner), &v.to_string()).is_ok())
}
/// The provider in use: the account's saved choice when there is one (every
/// picker writes it), otherwise this browser's remembered pick, otherwise the
/// backend default. Model names sync through the account.
pub fn ai_choice(settings: &Settings, preferences: &Value) -> (String, String) {
    if let Some(p) = &settings.user_ai {
        let id = p.provider().to_owned();
        let model = provider_model(settings, &preferences["models"], &id);
        return (id, model);
    }
    let provider = preferences["provider"]
        .as_str()
        .filter(|id| {
            settings.providers.iter().any(|p| {
                p.id == *id
                    && (!settings.local_ai.local_only || matches!(*id, "apple" | "mlx"))
                    && (p.configured || settings.demo || matches!(*id, "apple" | "mlx"))
            })
        })
        .unwrap_or_else(|| {
            settings
                .user_ai
                .as_ref()
                .map(|p| p.provider())
                .unwrap_or(&settings.provider)
        })
        .to_string();
    let model = provider_model(settings, &preferences["models"], &provider);
    (provider, model)
}
pub fn provider_model(settings: &Settings, models: &Value, provider: &str) -> String {
    // Apple/MLX models are installed and selected on the backend. Legacy browser
    // preferences must never pin a model that the admin has replaced.
    settings
        .user_models
        .get(provider)
        .map(String::as_str)
        .or_else(|| {
            (!matches!(provider, "apple" | "mlx"))
                .then(|| models[provider].as_str())
                .flatten()
        })
        .map(str::to_string)
        .or_else(|| {
            settings
                .providers
                .iter()
                .find(|p| p.id == provider)
                .map(|p| p.model.clone())
        })
        .unwrap_or_default()
}
/// The account policy a provider pick means: a local model keeps On-device or
/// Hybrid, a tool becomes its own mode, a cloud provider means Cloud AI.
pub fn policy_for(settings: &Settings, provider: &str) -> UserAiPreferences {
    let mut p = settings
        .user_ai
        .clone()
        .unwrap_or_else(|| crate::backend_default(settings));
    if matches!(provider, "apple" | "mlx") {
        if !matches!(p.mode.as_str(), "ondevice" | "hybrid") {
            p.mode = "ondevice".into();
        }
        p.local_provider = provider.into();
    } else if causewaybay_shared::cli_provider(provider) {
        p.mode = provider.into();
    } else {
        p.mode = "cloudai".into();
        p.cloud_provider = provider.into();
    }
    p
}
pub fn mode_label(mode: &str) -> &'static str {
    match mode {
        "ondevice" => "On-device",
        "hybrid" => "Hybrid",
        "cloudai" => "Cloud AI",
        "codex" => "Codex",
        "claude_code" => "Claude Code",
        _ => "AI",
    }
}

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub owner: String,
    pub settings: Settings,
    /// Names the controls ("Chat", "Coding", "Settings") so each place keeps
    /// distinct labels: `{prefix} AI provider`, `{prefix} AI model`.
    pub prefix: &'static str,
    #[prop_or_default]
    pub disabled: bool,
    /// The full page shows the modes too; a panel keeps to provider and model.
    #[prop_or_default]
    pub full: bool,
    /// A one-line shortcut: the provider list only, for quick switching.
    #[prop_or_default]
    pub compact: bool,
    /// The pick in use, emitted on every change: `(provider, model)`.
    pub changed: Callback<(String, String)>,
    /// The account policy changed on the backend; the shell reloads settings.
    pub ai_saved: Callback<()>,
}
pub struct AiPicker {
    provider: String,
    model: String,
    models: Value,
    busy: bool,
    error: String,
}
pub enum Msg {
    Choose(String),
    Mode(String),
    Saved(String, Result<UserAiPreferences, String>),
    Model(String),
    SaveModel,
    ModelSaved(Result<Value, String>),
}
impl Component for AiPicker {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        let preferences = preferences(&ctx.props().owner);
        let (provider, model) = ai_choice(&ctx.props().settings, &preferences);
        Self {
            provider,
            model,
            models: preferences["models"].clone(),
            busy: false,
            error: String::new(),
        }
    }
    fn changed(&mut self, ctx: &Context<Self>, old: &Self::Properties) -> bool {
        // The account policy arrived or changed elsewhere (another page, another
        // device), the admin replaced the local model, or Local-only came on:
        // follow the settings, keeping this browser's cloud models.
        let settings = &ctx.props().settings;
        let local = matches!(self.provider.as_str(), "apple" | "mlx");
        let stale = old.settings.user_ai != settings.user_ai
            || old.settings.user_models != settings.user_models
            || self.provider.is_empty()
            || (local && self.model != provider_model(settings, &json!({}), &self.provider))
            || (settings.local_ai.local_only && !local);
        if stale && !self.busy && !settings.provider.is_empty() {
            let (provider, model) = ai_choice(settings, &preferences(&ctx.props().owner));
            // A model being typed for the same cloud provider survives the
            // account round trip; only a provider change or a local model replaces it.
            if provider != self.provider || model != self.model {
                self.provider = provider;
                self.model = model;
                ctx.props()
                    .changed
                    .emit((self.provider.clone(), self.model.clone()));
            }
        }
        true
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Choose(id) => {
                if self.busy || id.is_empty() || id == self.provider {
                    return false;
                }
                let policy = policy_for(&ctx.props().settings, &id);
                self.save(ctx, id, policy)
            }
            Msg::Mode(mode) => {
                let settings = &ctx.props().settings;
                let mut policy = settings
                    .user_ai
                    .clone()
                    .unwrap_or_else(|| crate::backend_default(settings));
                if self.busy || policy.mode == mode {
                    return false;
                }
                policy.mode = mode;
                let provider = policy.provider().to_owned();
                self.save(ctx, provider, policy)
            }
            Msg::Saved(previous, r) => {
                self.busy = false;
                match r {
                    Ok(_) => {
                        remember(&ctx.props().owner, &self.provider, Some(&self.model));
                        ctx.props()
                            .changed
                            .emit((self.provider.clone(), self.model.clone()));
                        ctx.props().ai_saved.emit(());
                    }
                    Err(e) => {
                        self.provider = previous;
                        self.model =
                            provider_model(&ctx.props().settings, &self.models, &self.provider);
                        self.error = e;
                    }
                }
                true
            }
            Msg::SaveModel => {
                if self.provider == "apple" {
                    return false;
                }
                let body = json!({"provider":self.provider,"model":self.model});
                ctx.link().send_future(async move {
                    Msg::ModelSaved(api("PATCH", "/api/ai/models", Some(body)).await)
                });
                false
            }
            Msg::ModelSaved(result) => {
                match result {
                    Ok(_) => {
                        self.error.clear();
                        ctx.props().ai_saved.emit(());
                    }
                    Err(e) => self.error = e,
                }
                true
            }
            Msg::Model(v) => {
                if self.provider == "apple" {
                    return false;
                }
                self.model = v;
                self.models[&self.provider] = json!(self.model);
                if !remember(&ctx.props().owner, &self.provider, Some(&self.model)) {
                    self.error =
                        "Browser storage is unavailable; this selection lasts for this session."
                            .into();
                }
                ctx.props()
                    .changed
                    .emit((self.provider.clone(), self.model.clone()));
                true
            }
        }
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let link = ctx.link();
        let settings = &ctx.props().settings;
        let prefix = ctx.props().prefix;
        let local = matches!(self.provider.as_str(), "apple" | "mlx");
        let select_id = format!("{}-ai-select", prefix.to_lowercase());
        let mode = settings
            .user_ai
            .as_ref()
            .map(|p| p.mode.clone())
            .unwrap_or_default();
        let ready = |id: &str| {
            settings
                .providers
                .iter()
                .any(|p| p.id == id && (p.configured || settings.demo))
        };
        let can_cloud = !settings.local_ai.local_only
            && settings.providers.iter().any(|p| {
                !matches!(p.id.as_str(), "apple" | "mlx")
                    && !causewaybay_shared::cli_provider(&p.id)
                    && (p.configured || settings.demo)
            });
        let modes: [(&str, bool); 5] = [
            ("ondevice", ready("apple") || ready("mlx")),
            ("hybrid", (ready("apple") || ready("mlx")) && can_cloud),
            ("cloudai", can_cloud),
            ("codex", !settings.local_ai.local_only && ready("codex")),
            (
                "claude_code",
                !settings.local_ai.local_only && ready("claude_code"),
            ),
        ];
        if ctx.props().compact {
            return html! {<label class="quick-pick" for={select_id.clone()} data-testid={format!("{}-ai-picker",prefix.to_lowercase())}>{icon(if local {"shield"} else {"spark"})}<span>{t("AI")}</span>
                <select id={select_id.clone()} data-testid={select_id} aria-label={format!("{prefix} {}",t("AI provider"))} disabled={self.busy||ctx.props().disabled} value={self.provider.clone()} onchange={link.callback(|e:Event|Msg::Choose(e.target_unchecked_into::<HtmlSelectElement>().value()))}>
                    {for settings.providers.iter().filter(|p|p.configured||settings.demo||p.id==self.provider).map(|p|html!{<option value={p.id.clone()} selected={p.id==self.provider} disabled={settings.local_ai.local_only&&!matches!(p.id.as_str(),"apple"|"mlx")}>{p.name.clone()}</option>})}
                </select>
                if !self.error.is_empty(){<span class="error-text" role="alert">{t(&self.error)}</span>}
            </label>};
        }
        html! {<div class={classes!("ai-picker",ctx.props().full.then_some("is-full"))} data-testid={format!("{}-ai-picker",prefix.to_lowercase())}>
            if ctx.props().full {<div class="ai-picker-modes" role="group" aria-label={t("AI mode")}>
                {for modes.iter().map(|(id,available)|{let id=id.to_string();let active=mode==id;let pick=id.clone();html!{<button type="button" key={id.clone()} class={classes!("chip-button",active.then_some("is-active"))} aria-pressed={active.to_string()} disabled={self.busy||ctx.props().disabled||!*available} onclick={link.callback(move |_|Msg::Mode(pick.clone()))}>{icon(match id.as_str(){"ondevice"=>"shield","hybrid"=>"hybrid","codex"=>"codex","claude_code"=>"code",_=>"globe"})}{t(mode_label(&id))}</button>}})}
            </div>}
            if mode == "hybrid" {<p class="small-muted" data-testid="hybrid-scope">{t(HYBRID_HELP)}</p>}
            <div class="chat-model-choice">
                <label for={select_id.clone()}>{icon(if local {"shield"} else {"globe"})}{t("AI")}<select id={select_id.clone()} data-testid={select_id} aria-label={format!("{prefix} {}",t("AI provider"))} disabled={self.busy||ctx.props().disabled} value={self.provider.clone()} onchange={link.callback(|e:Event|Msg::Choose(e.target_unchecked_into::<HtmlSelectElement>().value()))}>
                    {for settings.providers.iter().filter(|p|p.configured||settings.demo||p.id==self.provider).map(|p|html!{<option value={p.id.clone()} selected={p.id==self.provider} disabled={settings.local_ai.local_only&&!matches!(p.id.as_str(),"apple"|"mlx")}>{format!("{} · {}",p.name,t(if matches!(p.id.as_str(),"apple"|"mlx"){"Local AI"}else if causewaybay_shared::cli_provider(&p.id){"Command-line tool"}else{"Cloud AI"}))}</option>})}
                </select></label>
                if self.provider == "mlx" {<label>{t("On-device model")}<select aria-label={format!("{prefix} {}",t("On-device model"))} disabled={self.busy||ctx.props().disabled} value={self.model.clone()} onchange={link.batch_callback(|e:Event|vec![Msg::Model(e.target_unchecked_into::<HtmlSelectElement>().value()),Msg::SaveModel])}>
                    <option value="mlx-community/Qwen3.5-9B-4bit" selected={self.model=="mlx-community/Qwen3.5-9B-4bit"}>{t("9B · Everyday · Recommended for 16 GB")}</option>
                    <option value="mlx-community/Qwen3.8-27B-4bit" selected={self.model=="mlx-community/Qwen3.8-27B-4bit"}>{t("27B · More capable · Needs more memory")}</option>
                    if !matches!(self.model.as_str(),"mlx-community/Qwen3.5-9B-4bit"|"mlx-community/Qwen3.8-27B-4bit") {<option value={self.model.clone()} selected=true>{&self.model}</option>}
                </select></label>} else {<label>{t("Model")}<input aria-label={format!("{prefix} {}",t("AI model"))} readonly={local} value={self.model.clone()} onchange={link.callback(|_|Msg::SaveModel)} oninput={link.callback(|e:InputEvent|Msg::Model(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>}
                <span class="small-muted">{if self.provider == "apple"{t("Saved on this browser and your account · model managed by admin")}else{t("Saved on this browser and your account")}}</span>
                if !self.error.is_empty(){<span class="error-text" role="alert">{t(&self.error)}</span>}
            </div>
        </div>}
    }
}
impl AiPicker {
    /// Switch at once so a model typed right after the choice lands under the
    /// new provider; the previous pick is restored if the account update fails.
    fn save(&mut self, ctx: &Context<Self>, provider: String, policy: UserAiPreferences) -> bool {
        self.busy = true;
        self.error.clear();
        let previous = std::mem::replace(&mut self.provider, provider);
        self.model = provider_model(&ctx.props().settings, &self.models, &self.provider);
        ctx.link().send_future(async move {
            Msg::Saved(
                previous,
                api("PATCH", "/api/ai/preferences", Some(json!(policy))).await,
            )
        });
        true
    }
}

/// The AI settings page: the one place that explains the modes, with the
/// same picker every other page embeds.
pub fn page(
    language: Language,
    owner: String,
    settings: Settings,
    ai_saved: Callback<()>,
    navigate: Callback<String>,
) -> Html {
    let mode = settings
        .user_ai
        .as_ref()
        .map(|p| p.mode.clone())
        .unwrap_or_default();
    html! {<section class="ai-page" data-testid="ai-page">
        <div class="page-intro"><div><div class="eyebrow">{icon("spark")}{t("WHICH AI ANSWERS")}</div><h1>{t("Your AI, your choice.")}</h1><p>{t("Pick the AI once here. Chat, Coding and your Autonomous AI sessions all use it. The pick is saved on this browser and mirrored to your account, so every page and device agrees.")}</p></div></div>
        <div class="ai-page-card">
            <AiPicker language={language} owner={owner} settings={settings.clone()} prefix="Settings" full=true changed={Callback::from(|_|())} ai_saved={ai_saved}/>
        </div>
        <dl class="ai-page-modes">
            <div class={classes!((mode=="ondevice").then_some("is-active"))}><dt>{icon("shield")}{t("On-device")}</dt><dd>{t("Apple's model or a Qwen model on the backend Mac. Nothing leaves your network.")}</dd></div>
            <div class={classes!((mode=="hybrid").then_some("is-active"))}><dt>{icon("hybrid")}{t("Hybrid")}</dt><dd>{t(HYBRID_HELP)}</dd></div>
            <div class={classes!((mode=="cloudai").then_some("is-active"))}><dt>{icon("globe")}{t("Cloud AI")}</dt><dd>{t("Anthropic, OpenAI, Grok or Gemini through the backend's keys.")}</dd></div>
            <div class={classes!((mode=="codex"||mode=="claude_code").then_some("is-active"))}><dt>{icon("codex")}{t("Codex / Claude Code")}</dt><dd>{t("The command-line tools signed in on the backend Mac, billed to those subscriptions. In Coding they work directly in the project folder.")}</dd></div>
        </dl>
        <p class="small-muted"><button class="text-button" onclick={navigate.reform(|_|"autonomous".to_string())}>{icon("bolt")}{t("Autonomous AI sessions and cloud approvals")}{icon("arrow")}</button></p>
    </section>}
}

#[cfg(test)]
mod tests {
    use super::*;
    use causewaybay_shared::Provider;

    fn settings(mode: &str) -> Settings {
        Settings {
            provider: "grok".into(),
            providers: [
                ("grok", "grok-4"),
                ("openai", "gpt"),
                ("mlx", "installed-model"),
            ]
            .into_iter()
            .map(|(id, model)| Provider {
                id: id.into(),
                model: model.into(),
                configured: true,
                ..Default::default()
            })
            .collect(),
            user_ai: Some(UserAiPreferences {
                mode: mode.into(),
                local_provider: "mlx".into(),
                cloud_provider: "grok".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn account_choice_wins_and_browser_choice_only_fills_the_gap() {
        let s = settings("cloudai");
        let prefs =
            json!({"provider":"openai","models":{"openai":"personal-model","grok":"my-grok"}});
        // The account's saved cloud provider beats a stale browser pick; the
        // model for that provider still comes from this browser.
        assert_eq!(ai_choice(&s, &prefs), ("grok".into(), "my-grok".into()));
        assert_eq!(ai_choice(&s, &json!({})), ("grok".into(), "grok-4".into()));
        let mut none = settings("cloudai");
        none.user_ai = None;
        assert_eq!(
            ai_choice(&none, &prefs),
            ("openai".into(), "personal-model".into())
        );
    }

    #[test]
    fn local_policies_ignore_stale_models_and_cloud_browser_choices() {
        for mode in ["ondevice", "hybrid"] {
            let mut s = settings(mode);
            let prefs = json!({"provider":"openai","models":{"mlx":"obsolete-model"}});
            assert_eq!(
                ai_choice(&s, &prefs),
                ("mlx".into(), "installed-model".into())
            );
            s.providers[2].model = "replacement-model".into();
            assert_eq!(
                ai_choice(&s, &prefs),
                ("mlx".into(), "replacement-model".into())
            );
        }
    }

    #[test]
    fn a_pick_maps_to_the_account_policy() {
        let s = settings("hybrid");
        assert_eq!(policy_for(&s, "mlx").mode, "hybrid");
        assert_eq!(policy_for(&s, "openai").mode, "cloudai");
        assert_eq!(policy_for(&s, "openai").cloud_provider, "openai");
        assert_eq!(policy_for(&s, "claude_code").mode, "claude_code");
        let cloud = settings("cloudai");
        assert_eq!(policy_for(&cloud, "mlx").mode, "ondevice");
    }
}
