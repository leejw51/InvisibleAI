//! Recovery material lives only in this WASM instance, never browser storage or APIs.
use crate::{api::api, i18n::t, icons::icon};
use causewaybay_shared::identity::Identity;
use gloo_events::EventListener;
use gloo_timers::callback::Timeout;
use serde_json::{Value, json};
use std::cell::RefCell;
use web_sys::HtmlInputElement;
use yew::prelude::*;
use zeroize::{Zeroize, Zeroizing};

struct Secret {
    address: String,
    index: u32,
    phrase: Zeroizing<String>,
}
thread_local! { static SECRET: RefCell<Option<Secret>> = const { RefCell::new(None) }; }
pub fn remember(address: String, index: u32, phrase: Zeroizing<String>) {
    SECRET.with(|value| {
        *value.borrow_mut() = Some(Secret {
            address,
            index,
            phrase,
        })
    });
}
pub fn forget() {
    SECRET.with(|value| *value.borrow_mut() = None);
}
pub fn retain_for(address: Option<&str>) {
    SECRET.with(|value| {
        if value
            .borrow()
            .as_ref()
            .is_some_and(|s| Some(s.address.as_str()) != address)
        {
            *value.borrow_mut() = None;
        }
    });
}
pub fn confirm(message: &str) -> bool {
    web_sys::window().is_some_and(|w| w.confirm_with_message(&t(message)).unwrap_or(false))
}
pub fn words(phrase: &str) -> Html {
    html! {<ol class="recovery-words" aria-label={t("Recovery phrase words")}>{for phrase.split_whitespace().map(|word|html!{<li>{word}</li>})}</ol>}
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
    phrase: String,
    revealed: bool,
    busy: bool,
    error: String,
    saved: bool,
    hide: Option<Timeout>,
    _visibility: EventListener,
    _keyboard: EventListener,
    close_button: NodeRef,
}
pub enum Msg {
    Name(String),
    Phrase(String),
    Save,
    Saved(Result<Value, String>),
    Reveal,
    Hide,
    Unlock,
}
impl Account {
    fn reveal(&mut self, ctx: &Context<Self>) {
        self.revealed = true;
        let link = ctx.link().clone();
        self.hide = Some(Timeout::new(60_000, move || link.send_message(Msg::Hide)));
    }
}
impl Component for Account {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        let link = ctx.link().clone();
        let document = web_sys::window().unwrap().document().unwrap();
        let visibility = EventListener::new(&document, "visibilitychange", move |_| {
            link.send_message(Msg::Hide)
        });
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
            phrase: String::new(),
            revealed: false,
            busy: false,
            error: String::new(),
            saved: false,
            hide: None,
            _visibility: visibility,
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
            Msg::Phrase(v) => {
                self.phrase.zeroize();
                self.phrase = v;
                self.error.clear();
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
            Msg::Reveal => {
                if !confirm(
                    "Display your recovery phrase and wallet index? Anyone who can see the phrase can access your wallet. Make sure your screen is private.",
                ) {
                    return false;
                }
                self.reveal(ctx);
            }
            Msg::Hide => {
                self.revealed = false;
                self.phrase.zeroize();
                self.hide = None;
                self.error.clear();
            }
            Msg::Unlock => {
                let index = ctx.props().session["wallet_index"].as_u64().unwrap_or(0) as u32;
                let address = ctx.props().session["address"].as_str().unwrap_or_default();
                match Identity::from_mnemonic_at(&self.phrase, index) {
                    Ok(identity) if identity.address() == address => {
                        if confirm(
                            "Display your recovery phrase and wallet index? Anyone who can see the phrase can access your wallet. Make sure your screen is private.",
                        ) {
                            remember(
                                address.into(),
                                index,
                                Zeroizing::new(
                                    self.phrase.split_whitespace().collect::<Vec<_>>().join(" "),
                                ),
                            );
                            self.reveal(ctx);
                        }
                    }
                    Ok(_) => {
                        self.error =
                            "This recovery phrase does not match your signed-in wallet and index."
                                .into()
                    }
                    Err(e) => self.error = e,
                }
                self.phrase.zeroize();
            }
        }
        true
    }
    fn destroy(&mut self, _: &Context<Self>) {
        self.phrase.zeroize();
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let address = ctx.props().session["address"].as_str().unwrap_or_default();
        let index = ctx.props().session["wallet_index"].as_u64().unwrap_or(0) as u32;
        let recovery = if self.revealed {
            SECRET.with(|value| {
                value
                    .borrow()
                    .as_ref()
                    .filter(|s| s.address == address && s.index == index)
                    .map(|s| words(&s.phrase))
            })
        } else {
            None
        };
        html! {<div class="modal-backdrop" onclick={ctx.props().close.reform(|_|())}><section class="modal account-modal" role="dialog" aria-modal="true" aria-label={t("Account details")} onclick={Callback::from(|e:MouseEvent|e.stop_propagation())}>
            <div class="modal-heading"><h2>{t("Account details")}</h2><button ref={self.close_button.clone()} class="icon-button" aria-label={t("Close account details")} onclick={ctx.props().close.reform(|_|())}>{icon("close")}</button></div>
            <div class="settings-body">
                <code class="wallet-address" aria-label={t("Your wallet address")}>{address}</code>
                <label class="form-label">{t("User name")}<input aria-label={t("User name")} maxlength="80" value={self.name.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Name(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                <button class="secondary-button" disabled={self.busy||self.name.trim().is_empty()} onclick={ctx.link().callback(|_|Msg::Save)}>{t("Save name")}</button>
                if self.saved {<p role="status">{t("Name saved")}</p>}
                <div class="wallet-recovery">
                    if self.revealed {
                        <p>{t("Wallet index")}{": "}<strong aria-label={t("Wallet index")}>{index}</strong></p>
                        if let Some(recovery) = recovery { {recovery} } else {
                            <p class="small-muted">{t("The phrase is no longer in this tab’s memory. Re-enter it to view it again; it cannot be recovered from the server.")}</p>
                            <label class="form-label">{t("Re-enter recovery phrase")}<input type="password" aria-label={t("Re-enter recovery phrase")} autocomplete="off" spellcheck="false" value={self.phrase.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Phrase(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                            <button class="secondary-button" disabled={self.phrase.trim().is_empty()} onclick={ctx.link().callback(|_|Msg::Unlock)}>{t("Verify and show phrase")}</button>
                        }
                        <p class="small-muted">{t("Details hide after one minute or when you leave this tab.")}</p>
                        <button class="text-button" onclick={ctx.link().callback(|_|Msg::Hide)}>{t("Hide recovery details")}</button>
                    } else {
                        <p class="small-muted">{t("Recovery phrase and wallet index are hidden. Confirm before displaying them.")}</p>
                        <button class="secondary-button" onclick={ctx.link().callback(|_|Msg::Reveal)}>{t("Show recovery details")}</button>
                    }
                </div>
                if !self.error.is_empty(){<p class="login-error" role="alert">{t(&self.error)}</p>}
            </div>
        </section></div>}
    }
}
