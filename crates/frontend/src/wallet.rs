//! Wallet: USDC on Cronos, mainnet or testnet, from the key saved at sign-in.
//! The browser talks to the public Cronos JSON-RPC directly; the backend never
//! sees the key, and only a signed transaction ever leaves this page.
use crate::{account, i18n::t, icons::icon};
use causewaybay_shared::evm::{self, LegacyTransaction, Network, U256};
use causewaybay_shared::identity::username;
use gloo_timers::callback::Timeout;
use serde_json::{Value, json};
use std::collections::HashMap;
use web_sys::HtmlInputElement;
use yew::prelude::*;

const NETWORK_KEY: &str = "invisible-ai.wallet.network";
/// How long a panel takes to fade out before it leaves the DOM (wallet-card.js).
const FADE_MS: u32 = 420;

async fn rpc(network: &Network, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
    let response = gloo_net::http::Request::post(network.rpc)
        .header("content-type", "application/json")
        .body(body.to_string())
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|_| format!("Could not reach {}. Check your connection.", network.name))?;
    let value: Value = response
        .json()
        .await
        .map_err(|_| "The network answered with something unexpected.".to_string())?;
    if let Some(error) = value.get("error") {
        return Err(error["message"]
            .as_str()
            .unwrap_or("The network refused the request.")
            .to_string());
    }
    Ok(value["result"].clone())
}
fn quantity(value: &Value) -> Result<U256, String> {
    evm::parse_quantity(value.as_str().unwrap_or_default())
}
fn hex_data(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[derive(Clone, PartialEq)]
pub struct Balances {
    native: U256,
    usdc: U256,
}
async fn balances(network: Network, address: String) -> Result<Balances, String> {
    let owner = evm::parse_address(&address)?;
    let chain = quantity(&rpc(&network, "eth_chainId", json!([])).await?)?;
    if chain != U256::from(network.chain_id) {
        return Err(format!(
            "{} answered as a different chain. Refusing to continue.",
            network.rpc
        ));
    }
    let native = quantity(&rpc(&network, "eth_getBalance", json!([address, "latest"])).await?)?;
    let call = json!([{"to":network.usdc.address,"data":hex_data(&evm::erc20_balance_of(&owner))},"latest"]);
    let usdc = quantity(&rpc(&network, "eth_call", call).await?)?;
    Ok(Balances { native, usdc })
}

#[derive(Clone, PartialEq)]
pub struct Prepared {
    tx: LegacyTransaction,
    to: String,
    amount: String,
    fee: String,
}
async fn prepare(
    network: Network,
    from: String,
    to: String,
    amount: String,
    have: Balances,
) -> Result<Prepared, String> {
    // Everything that can be refused without the network is refused first.
    let request = evm::check_transfer(&network, &from, &to, &amount, have.usdc)?;
    let nonce = quantity(
        &rpc(
            &network,
            "eth_getTransactionCount",
            json!([from, "pending"]),
        )
        .await?,
    )?;
    let gas_price = quantity(&rpc(&network, "eth_gasPrice", json!([])).await?)?;
    let estimate = rpc(
        &network,
        "eth_estimateGas",
        json!([{"from":from,"to":network.usdc.address,"data":hex_data(&request.data)}]),
    )
    .await
    .and_then(|v| quantity(&v))
    .ok();
    let tx = evm::build_transfer(
        &network,
        &request,
        evm::Quote {
            nonce,
            gas_price,
            estimate,
        },
        have.native,
    )?;
    Ok(Prepared {
        to: evm::checksum(&request.recipient),
        amount: evm::format_units(request.value, network.usdc.decimals),
        fee: evm::format_units(tx.fee(), 18),
        tx,
    })
}

#[derive(Clone, PartialEq)]
struct Activity {
    network: Network,
    hash: String,
    amount: String,
    to: String,
    status: &'static str,
}
#[derive(Properties, PartialEq)]
pub struct Props {
    pub session: Value,
    pub copy: Callback<String>,
}
pub struct Wallet {
    network: Network,
    balances: Option<Balances>,
    loading: bool,
    error: String,
    to: String,
    amount: String,
    review: Option<Prepared>,
    review_leaving: bool,
    preparing: bool,
    sending: bool,
    activity: Vec<Activity>,
    generation: u32,
    _fade: Option<Timeout>,
    /// One receipt poll per pending transfer, keyed by hash.
    polls: HashMap<String, Timeout>,
}
pub enum Msg {
    Network(&'static str),
    Refresh,
    Loaded(u32, Result<Balances, String>),
    To(String),
    Amount(String),
    Max,
    Review,
    Prepared(u32, Result<Prepared, String>),
    Cancel,
    Removed,
    Send,
    Sent(Result<(String, Prepared), String>),
    Poll(String, u32),
    Receipt(String, u32, Result<Value, String>),
}
fn stored_network() -> Network {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(NETWORK_KEY).ok().flatten())
        .and_then(|key| evm::network(&key))
        .unwrap_or(evm::CRONOS_TESTNET)
}
impl Wallet {
    fn address(ctx: &Context<Self>) -> String {
        ctx.props().session["address"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
    /// The signing key, only if it belongs to the signed-in wallet.
    fn unlocked(ctx: &Context<Self>) -> bool {
        account::saved_address().is_some_and(|a| a == Self::address(ctx))
    }
    fn close_review(&mut self, ctx: &Context<Self>) {
        if self.review.is_none() || self.review_leaving {
            return;
        }
        self.review_leaving = true;
        let link = ctx.link().clone();
        self._fade = Some(Timeout::new(FADE_MS, move || {
            link.send_message(Msg::Removed)
        }));
    }
}
impl Component for Wallet {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Refresh);
        Self {
            network: stored_network(),
            balances: None,
            loading: false,
            error: String::new(),
            to: String::new(),
            amount: String::new(),
            review: None,
            review_leaving: false,
            preparing: false,
            sending: false,
            activity: Vec::new(),
            generation: 0,
            _fade: None,
            polls: HashMap::new(),
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Network(key) => {
                let Some(network) = evm::network(key) else {
                    return false;
                };
                if network == self.network || self.sending {
                    return false;
                }
                if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
                    let _ = s.set_item(NETWORK_KEY, key);
                }
                self.network = network;
                self.balances = None;
                self.review = None;
                self.review_leaving = false;
                self.error.clear();
                ctx.link().send_message(Msg::Refresh);
            }
            Msg::Refresh => {
                if !Self::unlocked(ctx) {
                    return true;
                }
                self.generation += 1;
                self.loading = true;
                let (generation, network, address) =
                    (self.generation, self.network, Self::address(ctx));
                ctx.link().send_future(async move {
                    Msg::Loaded(generation, balances(network, address).await)
                });
            }
            Msg::Loaded(generation, result) => {
                if generation != self.generation {
                    return false;
                }
                self.loading = false;
                match result {
                    Ok(b) => self.balances = Some(b),
                    Err(e) => self.error = e,
                }
            }
            Msg::To(v) => {
                self.to = v;
                self.error.clear();
            }
            Msg::Amount(v) => {
                self.amount = v;
                self.error.clear();
            }
            Msg::Max => {
                if let Some(b) = &self.balances {
                    self.amount = evm::format_units(b.usdc, self.network.usdc.decimals);
                }
            }
            Msg::Review => {
                let Some(have) = self.balances.clone() else {
                    return false;
                };
                if self.preparing || self.sending {
                    return false;
                }
                self.preparing = true;
                self.error.clear();
                let (generation, network, from) =
                    (self.generation, self.network, Self::address(ctx));
                let (to, amount) = (self.to.clone(), self.amount.clone());
                ctx.link().send_future(async move {
                    Msg::Prepared(generation, prepare(network, from, to, amount, have).await)
                });
            }
            Msg::Prepared(generation, result) => {
                self.preparing = false;
                if generation != self.generation {
                    return true;
                }
                match result {
                    Ok(p) => {
                        self.review = Some(p);
                        self.review_leaving = false;
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Cancel => self.close_review(ctx),
            Msg::Removed => {
                self.review = None;
                self.review_leaving = false;
            }
            Msg::Send => {
                let Some(prepared) = self.review.clone() else {
                    return false;
                };
                if self.sending || self.review_leaving {
                    return false;
                }
                // The key is read from local storage for this one signature
                // and dropped (zeroized) when the identity goes out of scope.
                let address = Self::address(ctx);
                let Some(identity) = account::identity().filter(|id| id.address() == address)
                else {
                    self.error = "This browser has no saved key for this wallet. Sign out and sign in again.".into();
                    return true;
                };
                let signed = match prepared.tx.sign(identity.signing_key()) {
                    Ok(signed) => signed,
                    Err(e) => {
                        self.error = e;
                        return true;
                    }
                };
                drop(identity);
                self.sending = true;
                let network = self.network;
                ctx.link().send_future(async move {
                    Msg::Sent(async {
                        let hash = rpc(&network, "eth_sendRawTransaction", json!([signed.raw_hex()])).await?;
                        let hash = hash.as_str().unwrap_or_default().to_string();
                        if !hash.eq_ignore_ascii_case(&signed.hash_hex()) {
                            return Err("The network reported a different transaction hash. Check the explorer before retrying.".into());
                        }
                        Ok((hash, prepared))
                    }.await)
                });
            }
            Msg::Sent(result) => {
                self.sending = false;
                match result {
                    Ok((hash, prepared)) => {
                        self.activity.insert(
                            0,
                            Activity {
                                network: self.network,
                                hash: hash.clone(),
                                amount: prepared.amount,
                                to: prepared.to,
                                status: "Pending",
                            },
                        );
                        self.to.clear();
                        self.amount.clear();
                        self.close_review(ctx);
                        ctx.link().send_message(Msg::Poll(hash, 0));
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Poll(hash, attempt) => {
                let Some(network) = self
                    .activity
                    .iter()
                    .find(|a| a.hash == hash)
                    .map(|a| a.network)
                else {
                    return false;
                };
                ctx.link().send_future(async move {
                    let receipt = rpc(&network, "eth_getTransactionReceipt", json!([hash])).await;
                    Msg::Receipt(hash, attempt, receipt)
                });
                return false;
            }
            Msg::Receipt(hash, attempt, result) => {
                let status = match &result {
                    Ok(r) if r.is_object() => Some(if r["status"] == "0x1" {
                        "Confirmed"
                    } else {
                        "Failed"
                    }),
                    _ if attempt >= 45 => Some("Unconfirmed"),
                    _ => None,
                };
                match status {
                    Some(status) => {
                        self.polls.remove(&hash);
                        if let Some(row) = self.activity.iter_mut().find(|a| a.hash == hash) {
                            row.status = status;
                        }
                        ctx.link().send_message(Msg::Refresh);
                    }
                    None => {
                        let link = ctx.link().clone();
                        let next = hash.clone();
                        self.polls.insert(
                            hash,
                            Timeout::new(2_000, move || {
                                link.send_message(Msg::Poll(next, attempt + 1))
                            }),
                        );
                        return false;
                    }
                }
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let address = Self::address(ctx);
        let network = self.network;
        let unlocked = Self::unlocked(ctx);
        let usdc = self
            .balances
            .as_ref()
            .map(|b| evm::format_units(b.usdc, network.usdc.decimals));
        let native = self
            .balances
            .as_ref()
            .map(|b| evm::format_units(b.native, 18));
        let state = if self.sending {
            "sending"
        } else if self.loading {
            "loading"
        } else {
            "ready"
        };
        let summary = match (&usdc, &native) {
            (Some(u), Some(n)) => format!(
                "{} {}: {u} {}, {n} {}",
                t("Wallet card"),
                network.name,
                network.usdc.symbol,
                network.native
            ),
            _ => format!("{} {}", t("Wallet card"), network.name),
        };
        let link = ctx.link();
        let tab = |key: &'static str, label: &'static str| {
            let on = network.key == key;
            html! {<button role="tab" aria-selected={on.to_string()} class={classes!(on.then_some("selected"))} disabled={self.sending} onclick={link.callback(move |_|Msg::Network(key))}>{t(label)}</button>}
        };
        html! {<section class="wallet-page" data-testid="wallet-page">
            <div class="page-intro"><div><div class="eyebrow">{icon("shield")}{t("YOUR STABLECOIN WALLET")}</div><h1>{t("Digital dollars, beautifully simple.")}</h1><p>{t("Hold and send USDC on Cronos. Your key stays in this browser and signs every transfer here.")}</p></div></div>
            <div class="wallet-network filter-tabs" role="tablist" aria-label={t("Network")}>{tab("cronos-testnet","Testnet")}{tab("cronos-mainnet","Mainnet")}</div>
            if network.chain_id == 25 {<p class="wallet-mainnet-note" data-motion="" role="note">{icon("info")}{t("Mainnet moves real USDC. Double-check every address.")}</p>}
            <div class="wallet-layout">
                <div class="wallet-card-column" data-motion="">
                    <cwb-wallet-card role="img" aria-label={summary} data-testid="wallet-card" data-network={network.key} data-state={state} data-address={address.clone()} data-holder={ctx.props().session["name"].as_str().map(str::to_owned).or_else(||username(&address).ok()).unwrap_or_default()} data-symbol={network.usdc.symbol} data-native-symbol={network.native} data-usdc={usdc.clone().unwrap_or_else(||"0".into())} data-native={native.clone().unwrap_or_else(||"0".into())}></cwb-wallet-card>
                    <div class="wallet-balances" aria-live="polite">
                        <div><span class="small-muted">{network.usdc.symbol}</span><strong data-testid="usdc-balance">{usdc.clone().unwrap_or_else(||"—".into())}</strong></div>
                        <div><span class="small-muted">{network.native}{" · "}{t("for fees")}</span><strong data-testid="native-balance">{native.unwrap_or_else(||"—".into())}</strong></div>
                        <button class="icon-button" aria-label={t("Refresh balances")} disabled={self.loading||!unlocked} onclick={link.callback(|_|Msg::Refresh)}>{icon("refresh")}</button>
                    </div>
                    <div class="wallet-receive">
                        <span class="small-muted">{t("Receive on")}{" "}{network.name}</span>
                        <code class="wallet-address" aria-label={t("Your wallet address")}>{address.clone()}</code>
                        <button class="secondary-button" onclick={ctx.props().copy.reform({let a=address.clone(); move |_|a.clone()})}>{icon("copy")}{t("Copy address")}</button>
                    </div>
                </div>
                <div class="wallet-actions" data-motion="" data-motion-delay="0.12">
                    if !unlocked {
                        <div class="wallet-locked" role="status"><h2>{t("Wallet locked on this browser")}</h2><p>{t("Sign out and sign in again with your recovery phrase or private key to save this wallet's key in this browser.")}</p></div>
                    } else if let Some(p) = &self.review {
                        <div class={classes!("wallet-review", self.review_leaving.then_some("is-leaving"))} data-motion="" data-testid="wallet-review" role="dialog" aria-label={t("Review transfer")}>
                            <div class="eyebrow">{t("REVIEW")}</div>
                            <p class="wallet-review-amount"><strong>{&p.amount}</strong>{" "}{network.usdc.symbol}</p>
                            <dl>
                                <dt>{t("To")}</dt><dd><code>{&p.to}</code></dd>
                                <dt>{t("Network")}</dt><dd>{network.name}</dd>
                                <dt>{t("Network fee, at most")}</dt><dd>{&p.fee}{" "}{network.native}</dd>
                            </dl>
                            <div class="modal-footer">
                                <button class="text-button" disabled={self.sending} onclick={link.callback(|_|Msg::Cancel)}>{t("Cancel")}</button>
                                <button class="primary-button" data-testid="wallet-confirm" disabled={self.sending||self.review_leaving} onclick={link.callback(|_|Msg::Send)}>{if self.sending {t("Sending…")} else {t("Confirm and send")}}{icon("arrow")}</button>
                            </div>
                        </div>
                    } else {
                        <form class="wallet-send" onsubmit={link.callback(|e:SubmitEvent|{e.prevent_default();Msg::Review})}>
                            <h2>{t("Send")}{" "}{network.usdc.symbol}</h2>
                            <label class="form-label">{t("Recipient address")}<input type="text" aria-label={t("Recipient address")} placeholder="0x…" autocomplete="off" spellcheck="false" value={self.to.clone()} oninput={link.callback(|e:InputEvent|Msg::To(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                            <label class="form-label">{t("Amount")}<span class="wallet-amount-field"><input type="text" inputmode="decimal" aria-label={t("Amount")} placeholder="0.00" autocomplete="off" value={self.amount.clone()} oninput={link.callback(|e:InputEvent|Msg::Amount(e.target_unchecked_into::<HtmlInputElement>().value()))}/><button type="button" class="chip-button" onclick={link.callback(|_|Msg::Max)}>{t("Max")}</button></span></label>
                            <button class="primary-button" type="submit" disabled={self.preparing||self.balances.is_none()||self.to.trim().is_empty()||self.amount.trim().is_empty()}>{if self.preparing {t("Checking…")} else {t("Review transfer")}}{icon("arrow")}</button>
                        </form>
                    }
                    if !self.error.is_empty() {<p class="login-error" role="alert" data-motion="">{t(&self.error)}</p>}
                    if !self.activity.is_empty() {
                        <div class="wallet-activity" data-motion=""><h3>{t("This session")}</h3><ul>{for self.activity.iter().map(|a|html!{
                            <li data-motion="" data-testid="wallet-activity"><span><strong>{&a.amount}{" "}{a.network.usdc.symbol}</strong><small>{t("to")}{" "}{format!("{}…{}",&a.to[..6],&a.to[a.to.len()-4..])}</small></span><span class={classes!("wallet-status", a.status.to_lowercase())}>{t(a.status)}</span><a href={format!("{}/tx/{}",a.network.explorer,a.hash)} target="_blank" rel="noopener noreferrer">{t("Explorer")}{icon("arrow")}</a></li>
                        })}</ul></div>
                    }
                </div>
            </div>
        </section>}
    }
}
