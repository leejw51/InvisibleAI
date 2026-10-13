//! Wallet: CRO, USDC or any ERC-20 on Cronos, mainnet or testnet, from the key
//! saved at sign-in. The browser talks to the public Cronos JSON-RPC directly;
//! the backend keeps only the choices (network, asset, saved tokens) so a new
//! sign-in opens the wallet as it was left. It never sees the key, and only a
//! signed transaction ever leaves this page.
use crate::{account, api::api, i18n::t, icons::icon};
use causewaybay_shared::evm::{
    self, Asset, CustomToken, LegacyTransaction, Network, U256, WalletSettings,
};
use causewaybay_shared::identity::username;
use gloo_timers::callback::Timeout;
use serde_json::{Value, json};
use std::collections::HashMap;
use web_sys::HtmlInputElement;
use yew::prelude::*;

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
/// `eth_call` on a contract, returning the raw bytes it answered.
async fn call(network: &Network, contract: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let answer = rpc(
        network,
        "eth_call",
        json!([{"to":contract,"data":hex_data(data)},"latest"]),
    )
    .await?;
    hex::decode(answer.as_str().unwrap_or_default().trim_start_matches("0x"))
        .map_err(|_| "The network answered with something unexpected.".to_string())
}
async fn check_chain(network: &Network) -> Result<(), String> {
    let chain = quantity(&rpc(network, "eth_chainId", json!([])).await?)?;
    if chain != U256::from(network.chain_id) {
        return Err(format!(
            "{} answered as a different chain. Refusing to continue.",
            network.rpc
        ));
    }
    Ok(())
}

#[derive(Clone, PartialEq)]
pub struct Balances {
    /// CRO: it pays every fee, whatever is being sent.
    native: U256,
    /// The chosen asset; the same as `native` when that is CRO.
    asset: U256,
    gas_price: U256,
}
async fn balances(network: Network, asset: Asset, address: String) -> Result<Balances, String> {
    let owner = evm::parse_address(&address)?;
    check_chain(&network).await?;
    let native = quantity(&rpc(&network, "eth_getBalance", json!([address, "latest"])).await?)?;
    let gas_price = quantity(&rpc(&network, "eth_gasPrice", json!([])).await?)?;
    let held = match asset.contract(&network) {
        None => native,
        Some(contract) => {
            if let Asset::Erc20(token) = &asset {
                // The saved decimals decide how much is sent: a contract that
                // now answers differently is refused, not trusted.
                let decimals =
                    evm::abi_decimals(&call(&network, &contract, &evm::ERC20_DECIMALS).await?)?;
                if decimals != token.decimals {
                    return Err(format!(
                        "{} now reports {decimals} decimals, not {} as saved. Remove it and add it again.",
                        token.symbol, token.decimals
                    ));
                }
            }
            let word = call(&network, &contract, &evm::erc20_balance_of(&owner)).await?;
            if word.len() != 32 {
                return Err("The contract did not answer like an ERC-20 token.".into());
            }
            U256::from_be_slice(&word)
        }
    };
    Ok(Balances {
        native,
        asset: held,
        gas_price,
    })
}
/// Read an ERC-20's own description from its contract on `network`.
async fn look_up(network: Network, address: String) -> Result<CustomToken, String> {
    let address = evm::checksum(&evm::parse_address(&address)?);
    check_chain(&network).await?;
    let code = rpc(&network, "eth_getCode", json!([address, "latest"])).await?;
    if matches!(code.as_str(), None | Some("0x") | Some("0x0") | Some("")) {
        return Err(format!(
            "There is no contract at this address on {}.",
            network.name
        ));
    }
    let symbol = evm::abi_string(&call(&network, &address, &evm::ERC20_SYMBOL).await?)?;
    let decimals = evm::abi_decimals(&call(&network, &address, &evm::ERC20_DECIMALS).await?)?;
    // Some tokens have no name(); the symbol and address still identify them.
    let name = match call(&network, &address, &evm::ERC20_NAME).await {
        Ok(bytes) => evm::abi_string(&bytes).unwrap_or_default(),
        Err(_) => String::new(),
    };
    evm::validate_token(
        &network,
        &CustomToken {
            address,
            symbol,
            name,
            decimals,
        },
    )
}

#[derive(Clone, PartialEq)]
pub struct Prepared {
    tx: LegacyTransaction,
    to: String,
    amount: String,
    symbol: String,
    fee: String,
    /// Amount plus fee, when the amount is CRO too.
    total: Option<String>,
}
async fn prepare(
    network: Network,
    asset: Asset,
    from: String,
    to: String,
    amount: String,
    have: Balances,
) -> Result<Prepared, String> {
    // Everything that can be refused without the network is refused first.
    let request = evm::check_transfer(&network, &asset, &from, &to, &amount, have.asset)?;
    let destination = evm::checksum(&request.to);
    let nonce = quantity(
        &rpc(
            &network,
            "eth_getTransactionCount",
            json!([from, "pending"]),
        )
        .await?,
    )?;
    let gas_price = quantity(&rpc(&network, "eth_gasPrice", json!([])).await?)?;
    let mut probe = json!({"from":from,"to":destination,"data":hex_data(&request.data)});
    if !request.value.is_zero() {
        probe["value"] = json!(format!(
            "0x{}",
            hex::encode(request.value.to_be_bytes()).trim_start_matches('0')
        ));
    }
    let estimate = rpc(&network, "eth_estimateGas", json!([probe]))
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
    let native = matches!(asset, Asset::Native);
    Ok(Prepared {
        to: evm::checksum(&request.recipient),
        amount: evm::format_units(request.amount, asset.decimals(&network)),
        symbol: asset.symbol(&network),
        fee: evm::format_units(tx.fee(), 18),
        total: native.then(|| {
            evm::format_units(tx.fee().checked_add(request.value).unwrap_or(U256::MAX), 18)
        }),
        tx,
    })
}

#[derive(Clone, PartialEq)]
struct Activity {
    network: Network,
    hash: String,
    amount: String,
    symbol: String,
    to: String,
    status: &'static str,
}
#[derive(Properties, PartialEq)]
pub struct Props {
    pub session: Value,
    pub copy: Callback<String>,
}
pub struct Wallet {
    /// The backend's copy of this account's choices; `None` until it answers.
    settings: Option<WalletSettings>,
    network: Network,
    asset: Asset,
    balances: Option<Balances>,
    loading: bool,
    error: String,
    to: String,
    amount: String,
    review: Option<Prepared>,
    review_leaving: bool,
    preparing: bool,
    sending: bool,
    /// The "Other ERC-20" panel.
    others: bool,
    token_address: String,
    token_preview: Option<CustomToken>,
    token_checking: bool,
    token_error: String,
    activity: Vec<Activity>,
    generation: u32,
    _fade: Option<Timeout>,
    /// One receipt poll per pending transfer, keyed by hash.
    polls: HashMap<String, Timeout>,
}
pub enum Msg {
    Settings(Result<WalletSettings, String>),
    Saved(Result<WalletSettings, String>),
    Network(&'static str),
    Asset(String),
    Others(bool),
    TokenAddress(String),
    LookUp,
    LookedUp(String, Result<CustomToken, String>),
    AddToken,
    RemoveToken(String),
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
fn short(address: &str) -> String {
    if address.len() < 12 {
        return address.into();
    }
    format!("{}…{}", &address[..6], &address[address.len() - 4..])
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
    /// Show what the settings say for the current network, refreshing the
    /// balances only if the asset actually changed.
    fn apply(&mut self, ctx: &Context<Self>, settings: WalletSettings) {
        let asset = settings.asset_on(&self.network);
        self.settings = Some(settings);
        if asset != self.asset {
            self.asset = asset;
            self.reset_view();
            ctx.link().send_message(Msg::Refresh);
        }
    }
    fn reset_view(&mut self) {
        self.balances = None;
        self.review = None;
        self.review_leaving = false;
        self.amount.clear();
        self.error.clear();
    }
    fn save(ctx: &Context<Self>, method: &'static str, url: String, body: Option<Value>) {
        ctx.link()
            .send_future(async move { Msg::Saved(api(method, &url, body).await) });
    }
}
impl Component for Wallet {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link()
            .send_future(async { Msg::Settings(api("GET", "/api/wallet", None).await) });
        Self {
            settings: None,
            network: evm::CRONOS_TESTNET,
            asset: Asset::Usdc,
            balances: None,
            loading: false,
            error: String::new(),
            to: String::new(),
            amount: String::new(),
            review: None,
            review_leaving: false,
            preparing: false,
            sending: false,
            others: false,
            token_address: String::new(),
            token_preview: None,
            token_checking: false,
            token_error: String::new(),
            activity: Vec::new(),
            generation: 0,
            _fade: None,
            polls: HashMap::new(),
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Settings(result) => {
                let settings = result.unwrap_or_else(|e| {
                    self.error = e;
                    WalletSettings::default()
                });
                self.network = evm::network(&settings.network).unwrap_or(evm::CRONOS_TESTNET);
                self.asset = settings.asset_on(&self.network);
                self.others = matches!(self.asset, Asset::Erc20(_));
                self.settings = Some(settings);
                ctx.link().send_message(Msg::Refresh);
            }
            Msg::Saved(result) => match result {
                Ok(settings) => self.apply(ctx, settings),
                Err(e) => self.error = e,
            },
            Msg::Network(key) => {
                let Some(network) = evm::network(key) else {
                    return false;
                };
                if network == self.network || self.sending {
                    return false;
                }
                self.network = network;
                self.asset = self
                    .settings
                    .as_ref()
                    .map(|s| s.asset_on(&network))
                    .unwrap_or(Asset::Usdc);
                self.others = matches!(self.asset, Asset::Erc20(_));
                self.token_preview = None;
                self.token_error.clear();
                self.reset_view();
                if let Some(s) = &mut self.settings {
                    s.network = key.into();
                }
                Self::save(
                    ctx,
                    "PATCH",
                    "/api/wallet".into(),
                    Some(json!({"network":key})),
                );
                ctx.link().send_message(Msg::Refresh);
            }
            Msg::Asset(key) => {
                if self.sending || self.asset.key() == key {
                    return false;
                }
                let Some(settings) = &mut self.settings else {
                    return false;
                };
                settings
                    .selected
                    .insert(self.network.key.into(), key.clone());
                self.asset = settings.asset_on(&self.network);
                if !matches!(self.asset, Asset::Erc20(_)) {
                    self.others = false;
                }
                self.reset_view();
                Self::save(
                    ctx,
                    "PATCH",
                    "/api/wallet".into(),
                    Some(json!({"network":self.network.key,"asset":key})),
                );
                ctx.link().send_message(Msg::Refresh);
            }
            Msg::Others(open) => {
                self.others = open;
                self.token_error.clear();
            }
            Msg::TokenAddress(v) => {
                self.token_address = v;
                self.token_preview = None;
                self.token_error.clear();
            }
            Msg::LookUp => {
                if self.token_checking {
                    return false;
                }
                self.token_checking = true;
                self.token_preview = None;
                self.token_error.clear();
                let (network, address) = (self.network, self.token_address.trim().to_string());
                ctx.link().send_future(async move {
                    Msg::LookedUp(network.key.into(), look_up(network, address).await)
                });
            }
            Msg::LookedUp(network, result) => {
                self.token_checking = false;
                if network != self.network.key {
                    return true;
                }
                match result {
                    Ok(token) => self.token_preview = Some(token),
                    Err(e) => self.token_error = e,
                }
            }
            Msg::AddToken => {
                let Some(token) = self.token_preview.take() else {
                    return false;
                };
                self.token_address.clear();
                Self::save(
                    ctx,
                    "POST",
                    "/api/wallet/tokens".into(),
                    Some(json!({"network":self.network.key,"token":token})),
                );
            }
            Msg::RemoveToken(address) => {
                if self.sending
                    || !account::confirm(
                        "Remove this token from your list? Its balance stays on the chain, and you can add it again any time.",
                    )
                {
                    return false;
                }
                Self::save(
                    ctx,
                    "DELETE",
                    format!("/api/wallet/tokens/{}/{}", self.network.key, address),
                    None,
                );
            }
            Msg::Refresh => {
                if !Self::unlocked(ctx) || self.settings.is_none() {
                    return true;
                }
                self.generation += 1;
                self.loading = true;
                let (generation, network, asset, address) = (
                    self.generation,
                    self.network,
                    self.asset.clone(),
                    Self::address(ctx),
                );
                ctx.link().send_future(async move {
                    Msg::Loaded(generation, balances(network, asset, address).await)
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
                    // CRO keeps back enough for the fee of the send itself.
                    let max = match self.asset {
                        Asset::Native => evm::max_native(b.native, b.gas_price),
                        _ => b.asset,
                    };
                    self.amount = evm::format_units(max, self.asset.decimals(&self.network));
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
                let (generation, network, asset, from) = (
                    self.generation,
                    self.network,
                    self.asset.clone(),
                    Self::address(ctx),
                );
                let (to, amount) = (self.to.clone(), self.amount.clone());
                ctx.link().send_future(async move {
                    Msg::Prepared(
                        generation,
                        prepare(network, asset, from, to, amount, have).await,
                    )
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
                                symbol: prepared.symbol,
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
        let asset = &self.asset;
        let unlocked = Self::unlocked(ctx);
        let link = ctx.link();
        let intro = html! {<div class="page-intro"><div><div class="eyebrow">{icon("shield")}{t("YOUR CRONOS WALLET")}</div><h1>{t("Your money on Cronos, simply.")}</h1><p>{t("Hold and send CRO, USDC or any ERC-20 token on Cronos. Your key stays in this browser and signs every transfer here.")}</p></div></div>};
        let Some(settings) = &self.settings else {
            return html! {<section class="wallet-page" data-testid="wallet-page">{intro}<p class="small-muted" role="status">{t("Loading your wallet…")}</p></section>};
        };
        let symbol = asset.symbol(&network);
        let decimals = asset.decimals(&network);
        let native_asset = matches!(asset, Asset::Native);
        let held = self
            .balances
            .as_ref()
            .map(|b| evm::format_units(b.asset, decimals));
        let native = self
            .balances
            .as_ref()
            .map(|b| evm::format_units(b.native, 18));
        let no_fee = self.balances.as_ref().is_some_and(|b| b.native.is_zero());
        let state = if self.sending {
            "sending"
        } else if self.loading {
            "loading"
        } else {
            "ready"
        };
        let kind = match asset {
            Asset::Native => "native",
            Asset::Usdc => "usdc",
            Asset::Erc20(_) => "erc20",
        };
        let summary = match (&held, &native) {
            (Some(h), Some(n)) if !native_asset => format!(
                "{} {}: {h} {symbol}, {n} {}",
                t("Wallet card"),
                network.name,
                network.native
            ),
            (Some(h), _) => format!("{} {}: {h} {symbol}", t("Wallet card"), network.name),
            _ => format!("{} {}", t("Wallet card"), network.name),
        };
        let tab = |key: &'static str, label: &'static str| {
            let on = network.key == key;
            html! {<button role="tab" aria-selected={on.to_string()} class={classes!(on.then_some("selected"))} disabled={self.sending} onclick={link.callback(move |_|Msg::Network(key))}>{t(label)}</button>}
        };
        // The two main choices: CRONOS and USDC.
        let main = |key: &'static str, title: &'static str, sub: String, detail: &'static str| {
            let on = asset.key() == key;
            html! {<button role="radio" aria-checked={on.to_string()} data-testid={format!("asset-{key}")} class={classes!("wallet-asset-option", on.then_some("selected"))} disabled={self.sending} onclick={link.callback(move |_|Msg::Asset(key.into()))}>
                <span class="wallet-asset-title">{title}</span><span class="wallet-asset-symbol">{sub}</span><span class="wallet-asset-detail">{t(detail)}</span>
            </button>}
        };
        let custom = match asset {
            Asset::Erc20(token) => Some(token.clone()),
            _ => None,
        };
        let tokens = settings.tokens_on(&network);
        let explorer_address = |a: &str| format!("{}/address/{}", network.explorer, a);
        // Exactly which asset this page sends: large, with its contract.
        let identity = html! {<div class={classes!("wallet-asset-identity", kind)} data-testid="asset-identity" data-motion="" aria-live="polite">
            <div class="wallet-asset-identity-main">
                <span class="wallet-asset-big" data-testid="asset-symbol">{&symbol}</span>
                <span class="wallet-asset-name">{asset.name(&network)}</span>
            </div>
            <p class="wallet-asset-kind">{match asset {
                Asset::Native => format!("{} · {}", t("Native coin"), network.name),
                Asset::Usdc => format!("{} · {}", t("Stablecoin (ERC-20)"), network.name),
                Asset::Erc20(_) => format!("{} · {}", t("Custom ERC-20 token"), network.name),
            }}</p>
            if let Some(contract) = asset.contract(&network) {
                <div class="wallet-asset-contract"><span class="small-muted">{t("Contract")}</span><code data-testid="asset-contract">{contract.clone()}</code>
                    <button class="icon-button" aria-label={t("Copy contract address")} onclick={ctx.props().copy.reform({let c=contract.clone(); move |_|c.clone()})}>{icon("copy")}</button>
                    <a href={explorer_address(&contract)} target="_blank" rel="noopener noreferrer">{t("Explorer")}{icon("arrow")}</a></div>
            } else {
                <p class="wallet-asset-contract small-muted">{t("No contract: CRO is the network's own coin, and it pays every network fee.")}</p>
            }
            if let Some(token) = &custom {
                if evm::imitates_builtin(&network, token) {
                    <p class="wallet-warning" role="alert" data-testid="lookalike-warning">{icon("info")}{t("This token's name looks like CRO or USDC, but it is a different contract. Check the address before sending.")}</p>
                }
            }
        </div>};
        // CRO pays every fee, so its balance is always in view.
        let fees = html! {<div class={classes!("wallet-fee", no_fee.then_some("is-empty"), native_asset.then_some("is-native"))} data-testid="fee-balance" role="status">
            {icon("info")}
            <span>{if native_asset {
                format!("{} {}", t("Network fees are paid from this balance, in"), network.native)
            } else {
                format!("{} {}", t("Network fees are paid in"), network.native)
            }}</span>
            <strong>{native.clone().map(|n| format!("{n} {}", network.native)).unwrap_or_else(||"—".into())}</strong>
            if no_fee {<small>{t("Add some CRO to this wallet to pay network fees before sending.")}</small>}
        </div>};
        html! {<section class="wallet-page" data-testid="wallet-page">
            {intro}
            <div class="wallet-network filter-tabs" role="tablist" aria-label={t("Network")}>{tab("cronos-testnet","Testnet")}{tab("cronos-mainnet","Mainnet")}</div>
            if network.chain_id == 25 {<p class="wallet-mainnet-note" data-motion="" role="note">{icon("info")}{t("Mainnet moves real funds. Double-check every address.")}</p>}
            <div class="wallet-assets" role="radiogroup" aria-label={t("Asset")}>
                {main(evm::NATIVE_KEY, "CRONOS", network.native.to_string(), "Network coin · pays fees")}
                {main(evm::USDC_KEY, "USDC", network.usdc.symbol.to_string(), "Stablecoin · 1 USDC ≈ 1 US dollar")}
            </div>
            <button class={classes!("text-button","wallet-other-toggle",(self.others||custom.is_some()).then_some("selected"))} aria-expanded={self.others.to_string()} data-testid="asset-other" disabled={self.sending} onclick={link.callback({let open=self.others; move |_|Msg::Others(!open)})}>
                {match &custom {Some(c) => format!("{}: {}", t("Other ERC-20"), c.symbol), None => t("Other ERC-20 token")}}{icon(if self.others {"up"} else {"down"})}
            </button>
            if self.others {
                <div class="wallet-others" data-motion="" data-testid="wallet-others">
                    if !tokens.is_empty() {
                        <ul class="wallet-token-list" aria-label={t("Saved tokens")}>{for tokens.iter().map(|token| {
                            let key = token.address.to_ascii_lowercase();
                            let on = asset.key() == key;
                            html!{<li class={classes!(on.then_some("selected"))}>
                                <button class="wallet-token-pick" role="radio" aria-checked={on.to_string()} disabled={self.sending} onclick={link.callback({let k=key.clone(); move |_|Msg::Asset(k.clone())})}><strong>{&token.symbol}</strong><span>{&token.name}</span><code>{short(&token.address)}</code></button>
                                <button class="icon-button" aria-label={format!("{} {}", t("Remove"), token.symbol)} disabled={self.sending} onclick={link.callback({let a=token.address.clone(); move |_|Msg::RemoveToken(a.clone())})}>{icon("close")}</button>
                            </li>}
                        })}</ul>
                    }
                    <form class="wallet-token-add" onsubmit={link.callback(|e:SubmitEvent|{e.prevent_default();Msg::LookUp})}>
                        <label class="form-label">{t("Token contract address")}<input type="text" aria-label={t("Token contract address")} placeholder="0x…" autocomplete="off" spellcheck="false" value={self.token_address.clone()} oninput={link.callback(|e:InputEvent|Msg::TokenAddress(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                        <button class="secondary-button" type="submit" disabled={self.token_checking||self.token_address.trim().is_empty()}>{if self.token_checking {t("Checking…")} else {t("Look up token")}}</button>
                    </form>
                    if let Some(token) = &self.token_preview {
                        <div class="wallet-token-preview" data-motion="" data-testid="token-preview">
                            <span class="wallet-asset-big">{&token.symbol}</span>
                            <span class="wallet-asset-name">{&token.name}</span>
                            <dl>
                                <dt>{t("Network")}</dt><dd>{network.name}</dd>
                                <dt>{t("Contract")}</dt><dd><code>{&token.address}</code></dd>
                                <dt>{t("Decimals")}</dt><dd>{token.decimals}</dd>
                            </dl>
                            if evm::imitates_builtin(&network, token) {
                                <p class="wallet-warning" role="alert">{icon("info")}{t("This token's name looks like CRO or USDC, but it is a different contract. Check the address before sending.")}</p>
                            }
                            <button class="primary-button" data-testid="add-token" onclick={link.callback(|_|Msg::AddToken)}>{t("Add and use this token")}{icon("arrow")}</button>
                        </div>
                    }
                    if !self.token_error.is_empty() {<p class="login-error" role="alert">{t(&self.token_error)}</p>}
                </div>
            }
            {identity}
            <div class="wallet-layout">
                <div class="wallet-card-column" data-motion="">
                    <cwb-wallet-card role="img" aria-label={summary} data-testid="wallet-card" data-network={network.key} data-state={state} data-kind={kind} data-address={address.clone()} data-holder={ctx.props().session["name"].as_str().map(str::to_owned).or_else(||username(&address).ok()).unwrap_or_default()} data-symbol={symbol.clone()} data-native-symbol={network.native} data-balance={held.clone().unwrap_or_else(||"0".into())} data-native={native.clone().unwrap_or_else(||"0".into())}></cwb-wallet-card>
                    <div class="wallet-balances" aria-live="polite">
                        <div><span class="small-muted">{&symbol}</span><strong data-testid="asset-balance">{held.clone().unwrap_or_else(||"—".into())}</strong></div>
                        if !native_asset {<div><span class="small-muted">{network.native}{" · "}{t("for fees")}</span><strong data-testid="native-balance">{native.clone().unwrap_or_else(||"—".into())}</strong></div>}
                        <button class="icon-button" aria-label={t("Refresh balances")} disabled={self.loading||!unlocked} onclick={link.callback(|_|Msg::Refresh)}>{icon("refresh")}</button>
                    </div>
                    {fees}
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
                            <p class="wallet-review-amount"><strong>{&p.amount}</strong>{" "}<span data-testid="review-symbol">{&p.symbol}</span></p>
                            <dl>
                                <dt>{t("To")}</dt><dd><code>{&p.to}</code></dd>
                                <dt>{t("Network")}</dt><dd>{network.name}</dd>
                                if let Some(contract) = asset.contract(&network) {<dt>{t("Token contract")}</dt><dd><code>{contract}</code></dd>}
                                <dt>{t("Network fee, at most")}</dt><dd>{&p.fee}{" "}{network.native}</dd>
                                if let Some(total) = &p.total {<dt>{t("Total, at most")}</dt><dd><strong>{total}{" "}{network.native}</strong></dd>}
                            </dl>
                            <div class="modal-footer">
                                <button class="text-button" disabled={self.sending} onclick={link.callback(|_|Msg::Cancel)}>{t("Cancel")}</button>
                                <button class="primary-button" data-testid="wallet-confirm" disabled={self.sending||self.review_leaving} onclick={link.callback(|_|Msg::Send)}>{if self.sending {t("Sending…")} else {t("Confirm and send")}}{icon("arrow")}</button>
                            </div>
                        </div>
                    } else {
                        <form class="wallet-send" onsubmit={link.callback(|e:SubmitEvent|{e.prevent_default();Msg::Review})}>
                            <h2>{t("Send")}{" "}<span data-testid="send-symbol">{&symbol}</span></h2>
                            <label class="form-label">{t("Recipient address")}<input type="text" aria-label={t("Recipient address")} placeholder="0x…" autocomplete="off" spellcheck="false" value={self.to.clone()} oninput={link.callback(|e:InputEvent|Msg::To(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                            <label class="form-label">{t("Amount")}<span class="wallet-amount-field"><input type="text" inputmode="decimal" aria-label={t("Amount")} placeholder="0.00" autocomplete="off" value={self.amount.clone()} oninput={link.callback(|e:InputEvent|Msg::Amount(e.target_unchecked_into::<HtmlInputElement>().value()))}/><span class="wallet-amount-symbol">{&symbol}</span><button type="button" class="chip-button" onclick={link.callback(|_|Msg::Max)}>{t("Max")}</button></span></label>
                            <button class="primary-button" type="submit" disabled={self.preparing||self.balances.is_none()||self.to.trim().is_empty()||self.amount.trim().is_empty()}>{if self.preparing {t("Checking…")} else {t("Review transfer")}}{icon("arrow")}</button>
                        </form>
                    }
                    if !self.error.is_empty() {<p class="login-error" role="alert" data-motion="">{t(&self.error)}</p>}
                    if !self.activity.is_empty() {
                        <div class="wallet-activity" data-motion=""><h3>{t("This session")}</h3><ul>{for self.activity.iter().map(|a|html!{
                            <li data-motion="" data-testid="wallet-activity"><span><strong>{&a.amount}{" "}{&a.symbol}</strong><small>{t("to")}{" "}{short(&a.to)}</small></span><span class={classes!("wallet-status", a.status.to_lowercase())}>{t(a.status)}</span><a href={format!("{}/tx/{}",a.network.explorer,a.hash)} target="_blank" rel="noopener noreferrer">{t("Explorer")}{icon("arrow")}</a></li>
                        })}</ul></div>
                    }
                </div>
            </div>
        </section>}
    }
}
