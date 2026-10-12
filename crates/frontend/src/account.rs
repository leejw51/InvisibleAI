//! The wallet's private key is the one secret the browser keeps, and it is kept
//! only in local storage: never rendered, never sent to the backend or an AI
//! prompt. The recovery phrase is never stored at all; a signed-in phrase is
//! reduced to its derived key. Secrets leave only through an explicit Copy.
use crate::{api::api, clipboard, i18n::t, icons::icon};
use causewaybay_shared::identity::Identity;
use gloo_events::EventListener;
use serde_json::{Value, json};
use web_sys::HtmlInputElement;
use yew::prelude::*;
use zeroize::Zeroizing;

pub const KEY: &str = "invisible-ai.wallet.key";
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}
/// Save the signed-in wallet's private key in this browser.
pub fn remember(identity: &Identity) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(KEY, identity.private_key_hex().as_str());
    }
}
pub fn forget() {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(KEY);
    }
}
/// The saved wallet, or nothing if storage is empty, blocked or holds junk.
pub fn identity() -> Option<Identity> {
    let key = Zeroizing::new(storage()?.get_item(KEY).ok()??);
    Identity::from_private_key(&key).ok()
}
/// The saved wallet's address; derived from the key, so it cannot disagree.
pub fn saved_address() -> Option<String> {
    identity().map(|id| id.address())
}
/// A session for a different wallet makes the saved key stale.
pub fn retain_for(address: Option<&str>) {
    if let Some(address) = address
        && saved_address().is_some_and(|saved| saved != address)
    {
        forget();
    }
}
/// Put a secret on the clipboard without ever rendering it.
pub async fn copy_secret(text: Zeroizing<String>) -> bool {
    let navigator = web_sys::window().unwrap().navigator();
    let available = js_sys::Reflect::get(&navigator, &"clipboard".into())
        .is_ok_and(|v| !v.is_undefined() && !v.is_null());
    if available
        && wasm_bindgen_futures::JsFuture::from(navigator.clipboard().write_text(&text))
            .await
            .is_ok()
    {
        return true;
    }
    clipboard::copy_on_http(&text)
}
pub fn confirm(message: &str) -> bool {
    web_sys::window().is_some_and(|w| w.confirm_with_message(&t(message)).unwrap_or(false))
}
#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: crate::i18n::Language,
    pub session: Value,
    pub changed: Callback<()>,
    pub close: Callback<()>,
}
pub struct Account {
    name: String,
    busy: bool,
    error: String,
    saved: bool,
    copied: Option<bool>,
    _keyboard: EventListener,
    close_button: NodeRef,
}
pub enum Msg {
    Name(String),
    Save,
    Saved(Result<Value, String>),
    CopyKey,
    Copied(bool),
}
impl Component for Account {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        let document = web_sys::window().unwrap().document().unwrap();
        let close = ctx.props().close.clone();
        let keyboard = EventListener::new(&document, "keydown", move |e| {
            use wasm_bindgen::JsCast;
            if e.dyn_ref::<KeyboardEvent>()
                .is_some_and(|e| e.key() == "Escape")
            {
                close.emit(());
            }
        });
        Self {
            name: ctx.props().session["name"]
                .as_str()
                .unwrap_or_default()
                .into(),
            busy: false,
            error: String::new(),
            saved: false,
            copied: None,
            _keyboard: keyboard,
            close_button: NodeRef::default(),
        }
    }
    fn rendered(&mut self, _: &Context<Self>, first: bool) {
        if first && let Some(button) = self.close_button.cast::<web_sys::HtmlElement>() {
            let _ = button.focus();
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Name(v) => {
                self.name = v;
                self.saved = false;
            }
            Msg::Save => {
                if self.busy {
                    return false;
                }
                self.busy = true;
                self.error.clear();
                let body = json!({"name":self.name});
                ctx.link().send_future(async move {
                    Msg::Saved(api("PATCH", "/api/auth/profile", Some(body)).await)
                });
            }
            Msg::Saved(result) => {
                self.busy = false;
                match result {
                    Ok(v) => {
                        self.name = v["name"].as_str().unwrap_or_default().into();
                        self.saved = true;
                        ctx.props().changed.emit(());
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::CopyKey => {
                let address = ctx.props().session["address"].as_str().unwrap_or_default();
                let Some(identity) = identity().filter(|id| id.address() == address) else {
                    self.error = "This browser has no saved key for this wallet. Sign in again with your recovery phrase or private key.".into();
                    return true;
                };
                if !confirm(
                    "Copy your private key to the clipboard? Anyone who can paste it can spend from your wallet. Clear your clipboard after saving it somewhere safe.",
                ) {
                    return false;
                }
                let key = identity.private_key_hex();
                ctx.link()
                    .send_future(async move { Msg::Copied(copy_secret(key).await) });
            }
            Msg::Copied(ok) => self.copied = Some(ok),
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let address = ctx.props().session["address"].as_str().unwrap_or_default();
        let index = ctx.props().session["wallet_index"].as_u64().unwrap_or(0) as u32;
        let saved = saved_address().is_some_and(|saved| saved == address);
        html! {<div class="modal-backdrop" onclick={ctx.props().close.reform(|_|())}><section class="modal account-modal" role="dialog" aria-modal="true" aria-label={t("Account details")} onclick={Callback::from(|e:MouseEvent|e.stop_propagation())}>
            <div class="modal-heading"><h2>{t("Account details")}</h2><button ref={self.close_button.clone()} class="icon-button" aria-label={t("Close account details")} onclick={ctx.props().close.reform(|_|())}>{icon("close")}</button></div>
            <div class="settings-body">
                <code class="wallet-address" aria-label={t("Your wallet address")}>{address}</code>
                <label class="form-label">{t("User name")}<input aria-label={t("User name")} maxlength="80" value={self.name.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Name(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                <button class="secondary-button" disabled={self.busy||self.name.trim().is_empty()} onclick={ctx.link().callback(|_|Msg::Save)}>{t("Save name")}</button>
                if self.saved {<p role="status">{t("Name saved")}</p>}
                <div class="wallet-recovery">
                    <p>{t("Wallet index")}{": "}<strong aria-label={t("Wallet index")}>{index}</strong></p>
                    <p class="small-muted">{if saved {t("Your private key is saved in this browser only. It is never shown on screen or sent to the backend.")} else {t("This browser has no saved key for this wallet.")}}</p>
                    if saved {<button class="secondary-button" data-testid="copy-private-key" onclick={ctx.link().callback(|_|Msg::CopyKey)}>{icon("copy")}{t("Copy private key")}</button>}
                    if let Some(ok) = self.copied {<p role="status">{if ok {t("Private key copied")} else {t("Could not copy. Your browser blocked clipboard access.")}}</p>}
                </div>
                if !self.error.is_empty(){<p class="login-error" role="alert">{t(&self.error)}</p>}
            </div>
        </section></div>}
    }
}
