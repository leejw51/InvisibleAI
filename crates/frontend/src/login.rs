use crate::{
    account,
    api::api,
    i18n::t,
    icons::{brand, icon},
};
use causewaybay_shared::identity::{Identity, mnemonic_from_entropy, username};
use gloo_timers::callback::Timeout;
use serde_json::{Value, json};
use web_sys::HtmlInputElement;
use yew::prelude::*;
use zeroize::{Zeroize, Zeroizing};

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: crate::i18n::Language,
    pub session: Value,
    pub changed: Callback<()>,
}
#[derive(Clone, Copy, PartialEq)]
enum Method {
    Phrase,
    PrivateKey,
}
pub struct Login {
    method: Method,
    saved: Option<String>,
    copied: Option<bool>,
    phrase: String,
    index: String,
    name: String,
    custom_name: bool,
    generated: bool,
    recorded: bool,
    account: bool,
    busy: bool,
    error: String,
    preview: Option<Timeout>,
}
pub enum Msg {
    Method(Method),
    Copy,
    Copied(bool),
    Saved,
    Phrase(String),
    Index(String),
    Name(String),
    Preview,
    Generate,
    Existing,
    Recorded(bool),
    Account(bool),
    Login,
    Apply,
    Finished(Result<Value, String>),
    Logout,
}
fn wallet_index(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|v| *v < (1 << 31))
        .ok_or_else(|| "Wallet index must be a whole number from 0 to 2147483647.".into())
}
impl Login {
    fn identity(&self) -> Result<(u32, Identity), String> {
        match self.method {
            Method::PrivateKey => Identity::from_private_key(&self.phrase).map(|id| (0, id)),
            Method::Phrase => wallet_index(&self.index).and_then(|index| {
                Identity::from_mnemonic_at(&self.phrase, index).map(|id| (index, id))
            }),
        }
    }
    /// Sign the backend's challenge; only the signature leaves the browser, and
    /// the key is saved to local storage once the backend accepts it.
    fn sign_in(&mut self, ctx: &Context<Self>, index: u32, identity: Identity, name: Option<String>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error.clear();
        ctx.link().send_future(async move { Msg::Finished(async {
            let address = identity.address();
            let c: Value = api("POST", "/api/auth/challenge", Some(json!({"address":address}))).await?;
            let message = c["message"].as_str().ok_or("Missing login challenge")?;
            let host = web_sys::window().unwrap().location().host().unwrap_or_default();
            if !message.starts_with(&format!("Causewaybay Bot login\nHost: {host}\nAddress: {address}\nNonce: ")) { return Err("Invalid login challenge".into()); }
            let signature = identity.sign(message)?;
            let result = api("POST", "/api/auth/verify", Some(json!({"nonce":c["nonce"],"signature":signature,"wallet_index":index,"name":name}))).await?;
            account::remember(&identity, index);
            Ok(result)
        }.await) });
    }
    fn preview(&mut self, ctx: &Context<Self>) {
        let link = ctx.link().clone();
        self.preview = Some(Timeout::new(300, move || link.send_message(Msg::Preview)));
    }
}
impl Component for Login {
    type Message = Msg;
    type Properties = Props;
    fn create(_: &Context<Self>) -> Self {
        Self {
            method: Method::Phrase,
            saved: account::saved_address(),
            copied: None,
            phrase: String::new(),
            index: "0".into(),
            name: String::new(),
            custom_name: false,
            generated: false,
            recorded: false,
            account: false,
            busy: false,
            error: String::new(),
            preview: None,
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Method(method) => {
                self.phrase.zeroize();
                self.method = method;
                self.generated = false;
                self.recorded = false;
                self.copied = None;
                self.error.clear();
                if !self.custom_name {
                    self.name.clear();
                }
            }
            Msg::Copy => {
                let phrase = Zeroizing::new(self.phrase.clone());
                ctx.link()
                    .send_future(async move { Msg::Copied(account::copy_secret(phrase).await) });
            }
            Msg::Copied(ok) => self.copied = Some(ok),
            Msg::Saved => {
                let Some((identity, index)) = account::saved() else {
                    self.saved = None;
                    self.error = "The saved wallet is no longer in this browser. Sign in again.".into();
                    return true;
                };
                self.sign_in(ctx, index, identity, None);
            }
            Msg::Phrase(v) => {
                self.phrase.zeroize();
                self.phrase = v;
                self.error.clear();
                self.preview(ctx);
            }
            Msg::Index(v) => {
                self.index = v;
                self.recorded = false;
                self.error.clear();
                self.preview(ctx);
            }
            Msg::Name(v) => {
                self.custom_name = !v.is_empty();
                self.name = v;
            }
            Msg::Preview => {
                if !self.custom_name {
                    self.name = self
                        .identity()
                        .and_then(|(_, id)| username(&id.address()))
                        .unwrap_or_default();
                }
            }
            Msg::Generate => {
                if self.busy
                    || !account::confirm(
                        "Generate a new recovery phrase? It is never shown on screen: use Copy to save it somewhere safe, with your wallet index, before continuing.",
                    )
                {
                    return false;
                }
                let result = (|| {
                    let mut entropy = Zeroizing::new([0u8; 16]);
                    web_sys::window()
                        .ok_or("Browser unavailable")?
                        .crypto()
                        .map_err(|_| "Secure random generation is unavailable")?
                        .get_random_values_with_u8_array(entropy.as_mut())
                        .map_err(|_| "Secure random generation is unavailable")?;
                    mnemonic_from_entropy(&entropy)
                })();
                match result {
                    Ok(phrase) => {
                        self.phrase.zeroize();
                        self.phrase = phrase.to_string();
                        self.method = Method::Phrase;
                        self.generated = true;
                        self.recorded = false;
                        self.copied = None;
                        self.error.clear();
                        ctx.link().send_message(Msg::Preview);
                    }
                    Err(error) => self.error = error,
                }
            }
            Msg::Existing => {
                self.phrase.zeroize();
                self.generated = false;
                self.recorded = false;
                self.copied = None;
                if !self.custom_name {
                    self.name.clear();
                }
            }
            Msg::Recorded(v) => self.recorded = v,
            Msg::Account(v) => self.account = v,
            Msg::Login => {
                if self.busy || (self.generated && !self.recorded) {
                    return false;
                }
                match self.identity() {
                    Err(e) => self.error = e,
                    Ok((index, identity)) => {
                        let name = self.custom_name.then(|| self.name.trim().to_string());
                        if name.as_ref().is_some_and(|v| {
                            v.is_empty()
                                || v.chars().count() > 80
                                || v.chars().any(char::is_control)
                        }) {
                            self.error = "User name must contain 1 to 80 characters without control characters.".into();
                            return true;
                        }
                        self.sign_in(ctx, index, identity, name);
                    }
                }
            }
            Msg::Finished(r) => {
                self.busy = false;
                match r {
                    Ok(_) => {
                        self.phrase.zeroize();
                        self.generated = false;
                        self.recorded = false;
                        self.error.clear();
                        self.saved = account::saved_address();
                        ctx.props().changed.emit(());
                    }
                    Err(e) => {
                        self.error = if e == "AUTH_REQUIRED" {
                            "Login expired or signature rejected. Try again.".into()
                        } else {
                            e
                        }
                    }
                }
            }
            Msg::Apply => {
                if self.busy {
                    return false;
                }
                self.busy = true;
                ctx.link().send_future(async {
                    Msg::Finished(api("POST", "/api/auth/apply", Some(json!({}))).await)
                });
            }
            Msg::Logout => {
                if self.busy {
                    return false;
                }
                self.busy = true;
                account::forget();
                ctx.link().send_future(async {
                    Msg::Finished(api("POST", "/api/auth/logout", None).await)
                });
            }
        }
        true
    }
    fn destroy(&mut self, _: &Context<Self>) {
        self.phrase.zeroize();
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let session = &ctx.props().session;
        let signed = session["authenticated"] == true;
        let role = session["role"].as_str().unwrap_or("");
        html! {<main class="login-page"><section class="login-card"><span class="brand-mark">{brand()}</span><div class="eyebrow">{"INVISIBLE AI"}</div>
            if signed {
                <h1>{if role=="blocked" {t("Access is unavailable.")}else if role=="unregistered" {t("Your wallet, your workspace.")}else{t("You’re on the waiting list.")}}</h1>
                <p>{if role=="blocked" {t("Contact a workspace admin about your registration.")}else if role=="unregistered" {t("You’re signed in. Apply to use your private AI workspace.")}else{t("An admin will review your registration. This page updates when you’re approved.")}}</p>
                <code class="wallet-address" aria-label={t("Your wallet address")}>{session["address"].as_str().unwrap_or_default()}</code>
                <p class="wallet-name" aria-label={t("Your name")}>{session["name"].as_str().unwrap_or_default()}</p>
                if role=="unregistered" {<button class="primary-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Apply)}>{t("Apply for registration")}{icon("arrow")}</button>}
                <button class="secondary-button" onclick={ctx.link().callback(|_|Msg::Account(true))}>{t("Account details")}</button>
                <button class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Logout)}>{t("Sign out")}</button>
            } else {
                <h1>{t("Welcome to your space.")}</h1><p>{t("Sign in with your wallet recovery phrase or private key.")}</p>
                if let Some(saved) = &self.saved {
                    <div class="saved-wallet" data-testid="saved-wallet">
                        <p class="small-muted">{t("Saved in this browser")}</p>
                        <p class="wallet-name">{username(saved).unwrap_or_default()}</p>
                        <code class="wallet-address" aria-label={t("Saved wallet address")}>{saved}</code>
                        <button class="primary-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Saved)}>{if self.busy{t("Signing in…")}else{t("Continue with saved wallet")}}{icon("arrow")}</button>
                    </div>
                    <div class="settings-divider"/>
                }
                <div class="filter-tabs login-method" role="group" aria-label={t("Sign-in method")}>
                    <button aria-pressed={(self.method==Method::Phrase).to_string()} class={classes!((self.method==Method::Phrase).then_some("selected"))} disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Method(Method::Phrase))}>{t("Recovery phrase")}</button>
                    <button aria-pressed={(self.method==Method::PrivateKey).to_string()} class={classes!((self.method==Method::PrivateKey).then_some("selected"))} disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Method(Method::PrivateKey))}>{t("Private key")}</button>
                </div>
                if self.generated {
                    <h2 class="wallet-backup-title">{t("Save your new recovery phrase")}</h2>
                    <p>{t("For your safety the words are never shown. Copy them to a password manager or other safe place, with your wallet index. You need both to sign in again.")}</p>
                    <button class="secondary-button" data-testid="copy-recovery-phrase" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Copy)}>{icon("copy")}{t("Copy recovery phrase")}</button>
                    if let Some(ok) = self.copied {<p role="status">{if ok {t("Recovery phrase copied")} else {t("Could not copy. Your browser blocked clipboard access.")}}</p>}
                } else if self.method == Method::PrivateKey {
                    <label class="form-label">{t("Private key")}<input type="password" aria-label={t("Private key")} autocomplete="off" spellcheck="false" disabled={self.busy} value={self.phrase.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Phrase(e.target_unchecked_into::<HtmlInputElement>().value()))} onkeydown={ctx.link().batch_callback(|e:KeyboardEvent|if e.key()=="Enter"{Some(Msg::Login)}else{None})}/></label>
                } else {
                    <label class="form-label">{t("Recovery phrase")}<input type="password" aria-label={t("Recovery phrase")} autocomplete="off" spellcheck="false" disabled={self.busy} value={self.phrase.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Phrase(e.target_unchecked_into::<HtmlInputElement>().value()))} onkeydown={ctx.link().batch_callback(|e:KeyboardEvent|if e.key()=="Enter"{Some(Msg::Login)}else{None})}/></label>
                }
                if self.method == Method::Phrase {
                    <label class="form-label">{t("Wallet index")}<input type="number" min="0" max="2147483647" step="1" aria-label={t("Wallet index")} disabled={self.busy} value={self.index.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Index(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                    <p class="small-muted">{t("Default: 0. A different index opens a different wallet.")}</p>
                }
                <label class="form-label">{t("User name")}<input type="text" maxlength="80" aria-label={t("User name")} autocomplete="off" disabled={self.busy} placeholder={t("Derived from your wallet")} value={self.name.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Name(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                <p class="small-muted">{t("A wallet-derived name is suggested. You can change it here or in Account details. Existing saved names are kept unless you edit this field.")}</p>
                if self.generated {
                    <label class="wallet-backup-check"><input type="checkbox" checked={self.recorded} disabled={self.busy} onchange={ctx.link().callback(|e:Event|Msg::Recorded(e.target_unchecked_into::<HtmlInputElement>().checked()))}/>{t("I saved my recovery phrase and wallet index.")}</label>
                }
                <p class="small-muted">{t("Your phrase or key is treated like a password: it is never shown, never sent to the backend or to AI, and only the derived private key is saved, in this browser’s local storage.")}</p>
                <button class="primary-button" disabled={self.busy||self.phrase.trim().is_empty()||(self.generated&&!self.recorded)} onclick={ctx.link().callback(|_|Msg::Login)}>{if self.busy{t("Signing in…")}else{t("Sign in")}}{icon("arrow")}</button>
                <button class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Generate)}>{t("Create new recovery phrase")}</button>
                if self.generated {<button class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::Existing)}>{t("Use existing recovery phrase")}</button>}
                <p class="small-muted">{t("New wallets require admin approval before they can use the workspace.")}</p>
            }
            if !self.error.is_empty(){<p class="login-error" role="alert">{t(&self.error)}</p>}
        </section>
        if self.account {<account::Account language={ctx.props().language} session={session.clone()} changed={ctx.props().changed.clone()} close={ctx.link().callback(|_|Msg::Account(false))}/>}
        </main>}
    }
}
