//! Wallet: every coin on every network the Causewaybay Wallet knows, plus
//! Arbitrum. Two main buttons (Cronos Mainnet's CRO and USDC on Cronos
//! Mainnet) and one search box reach all of them. Balances are read straight
//! from each chain's public endpoint; EVM coins can be sent, everything else is
//! view and receive. The backend keeps only public choices — the active coin,
//! recent picks, added ERC-20s and receive addresses — so a new sign-in opens
//! the wallet as it was left. Keys and phrases stay in this browser.
use crate::{account, api::api, i18n::t, icons::icon};
use causewaybay_shared::chains;
use causewaybay_shared::evm::{self, Asset, CustomToken, LegacyTransaction, Network, U256};
use causewaybay_shared::identity::username;
use causewaybay_shared::registry::{self, Chain, Coin, MAIN_COINS, Net, Standard, WalletSettings};
use gloo_timers::callback::Timeout;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;
use yew::prelude::*;

/// How long a panel takes to fade out before it leaves the DOM (wallet-card.js).
const FADE_MS: u32 = 420;
/// Search rows shown at once; a narrower query finds the rest.
const MAX_RESULTS: usize = 40;

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
    /// The network's own coin: it pays every fee, whatever is being sent.
    native: U256,
    /// The active coin; the same as `native` when that is what is shown.
    asset: U256,
    /// EVM only, for Max.
    gas_price: U256,
}
async fn evm_balances(network: Network, asset: Asset, address: String) -> Result<Balances, String> {
    let owner = evm::parse_address(&address)?;
    check_chain(&network).await?;
    let native = quantity(&rpc(&network, "eth_getBalance", json!([address, "latest"])).await?)?;
    let gas_price = quantity(&rpc(&network, "eth_gasPrice", json!([])).await?)?;
    let held = match asset.contract(&network) {
        None => native,
        Some(contract) => {
            if let Asset::Erc20(token) = &asset {
                // The listed decimals decide how much is sent: a contract that
                // answers differently is refused, not trusted.
                let decimals =
                    evm::abi_decimals(&call(&network, &contract, &evm::ERC20_DECIMALS).await?)?;
                if decimals != token.decimals {
                    return Err(format!(
                        "{} reports {decimals} decimals, not {} as listed. Not shown.",
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
/// A non-EVM balance through `window.cwbChains` (frontend/wallet-chains.js).
async fn js_balance(net: &Net, coin: &Coin, address: &str) -> Result<U256, String> {
    let unloaded = || "The wallet scripts did not load.".to_string();
    let window = web_sys::window().ok_or("Browser unavailable")?;
    let chains = js_sys::Reflect::get(&window, &"cwbChains".into()).map_err(|_| unloaded())?;
    let read: js_sys::Function = js_sys::Reflect::get(&chains, &"balance".into())
        .ok()
        .and_then(|f| f.dyn_into().ok())
        .ok_or_else(unloaded)?;
    let parse = |v: Value| js_sys::JSON::parse(&v.to_string()).map_err(|_| unloaded());
    let net_js = parse(
        json!({"key": net.key, "chain": net.chain.key(), "endpoint": net.endpoint, "name": net.name}),
    )?;
    let mut coin_value = serde_json::to_value(coin).map_err(|e| e.to_string())?;
    if coin.standard == Standard::SplToken {
        coin_value["account"] = json!(chains::solana::associated_token_address(address, &coin.id)?);
    }
    let coin_js = parse(coin_value)?;
    let promise: js_sys::Promise = read
        .call3(&chains, &net_js, &coin_js, &address.into())
        .map_err(|_| unloaded())?
        .dyn_into()
        .map_err(|_| unloaded())?;
    let value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| {
            js_sys::Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
                .unwrap_or_else(|| format!("Could not read {}.", net.name))
        })?;
    value
        .as_string()
        .and_then(|s| s.parse::<U256>().ok())
        .ok_or_else(|| "The network reported a balance that is not a number.".into())
}
async fn coin_balances(coin: Coin, address: String) -> Result<Balances, String> {
    if let Some((network, asset)) = coin.evm_asset() {
        return evm_balances(network, asset, address).await;
    }
    let net = coin.net();
    let asset = js_balance(net, &coin, &address).await?;
    let native = if coin.standard == Standard::Native {
        asset
    } else {
        let own = registry::coin(&registry::native_key(net)).ok_or("Unknown network coin")?;
        js_balance(net, &own, &address).await?
    };
    Ok(Balances {
        native,
        asset,
        gas_price: U256::ZERO,
    })
}
/// Read an ERC-20's own description from its contract.
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
    total: Option<String>,
}
async fn prepare(
    coin: Coin,
    from: String,
    to: String,
    amount: String,
    have: Balances,
) -> Result<Prepared, String> {
    let (network, asset) = coin
        .evm_asset()
        .ok_or("Sending is not available for this coin yet.")?;
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
        symbol: coin.symbol.clone(),
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
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Receive,
    Send,
}
#[derive(Properties, PartialEq)]
pub struct Props {
    pub session: Value,
    pub copy: Callback<String>,
}
pub struct Wallet {
    settings: Option<WalletSettings>,
    coin: Coin,
    /// Network key → this wallet's receive address there.
    addresses: BTreeMap<String, String>,
    /// Networks whose address this browser derived itself (or the signed-in
    /// EVM address); the rest were recorded by an earlier phrase sign-in.
    verified: BTreeSet<String>,
    /// Bumped per backend write; only the newest reply is applied, so a slow
    /// earlier one cannot undo a later choice.
    saves: u32,
    balances: Option<Balances>,
    loading: bool,
    error: String,
    query: String,
    searching: bool,
    highlight: usize,
    preview: Option<(String, CustomToken)>,
    preview_error: String,
    looking: bool,
    tab: Tab,
    to: String,
    amount: String,
    review: Option<Prepared>,
    review_leaving: bool,
    preparing: bool,
    sending: bool,
    /// Bumped on every confirmed send; the card plays its send animation.
    sent: u32,
    last_sent: String,
    activity: Vec<Activity>,
    generation: u32,
    _fade: Option<Timeout>,
    _blur: Option<Timeout>,
    polls: HashMap<String, Timeout>,
}
pub enum Msg {
    Settings(Result<WalletSettings, String>),
    Saved(u32, Result<WalletSettings, String>),
    Choose(String),
    Query(String),
    Searching(bool),
    Blurred,
    Key(KeyboardEvent),
    LookUp(&'static str),
    LookedUp(String, Result<CustomToken, String>),
    AddToken,
    RemoveToken,
    Tab(Tab),
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
    let chars: Vec<char> = address.chars().collect();
    if chars.len() < 14 {
        return address.into();
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 6..].iter().collect();
    format!("{head}…{tail}")
}
/// Receive addresses: the signed-in EVM address on every EVM network, and the
/// other chains derived from the saved phrase — or, without one in this
/// browser, the public addresses an earlier sign-in recorded.
fn addresses(
    session_address: &str,
    settings: &WalletSettings,
) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let evm_address = evm::parse_address(session_address)
        .map(|a| evm::checksum(&a))
        .ok();
    let derived = account::saved_mnemonic().zip(account::saved().map(|(_, index)| index));
    let (mut out, mut verified) = (BTreeMap::new(), BTreeSet::new());
    for net in registry::NETS {
        let own = match net.chain {
            Chain::Evm => evm_address.clone(),
            _ => derived
                .as_ref()
                .and_then(|(phrase, index)| chains::address(phrase, *index, net.key).ok()),
        };
        if own.is_some() {
            verified.insert(net.key.to_string());
        }
        if let Some(address) = own.or_else(|| settings.addresses.get(net.key).cloned()) {
            out.insert(net.key.to_string(), address);
        }
    }
    (out, verified)
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
    fn receive_address(&self) -> Option<String> {
        self.addresses.get(&self.coin.network).cloned()
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
    fn show(&mut self, ctx: &Context<Self>, coin: Coin) {
        if coin == self.coin {
            return;
        }
        // Receive was forced by a view-only coin, not chosen: give Send back.
        if !self.coin.can_send() && coin.can_send() {
            self.tab = Tab::Send;
        }
        self.coin = coin;
        self.balances = None;
        self.review = None;
        self.review_leaving = false;
        self.amount.clear();
        self.error.clear();
        if !self.coin.can_send() {
            self.tab = Tab::Receive;
        }
        ctx.link().send_message(Msg::Refresh);
    }
    fn save(
        &mut self,
        ctx: &Context<Self>,
        method: &'static str,
        url: String,
        body: Option<Value>,
    ) {
        self.saves += 1;
        let seq = self.saves;
        ctx.link()
            .send_future(async move { Msg::Saved(seq, api(method, &url, body).await) });
    }
    fn results(&self) -> Vec<Coin> {
        let Some(settings) = &self.settings else {
            return vec![];
        };
        let all = settings.all_coins();
        if self.query.trim().is_empty() {
            // Recent picks first, then everything in registry order.
            let mut rows: Vec<Coin> = settings
                .recent
                .iter()
                .filter_map(|k| all.iter().find(|c| &c.key == k).cloned())
                .collect();
            rows.extend(
                all.iter()
                    .filter(|c| !settings.recent.contains(&c.key))
                    .cloned(),
            );
            rows.truncate(MAX_RESULTS);
            return rows;
        }
        registry::search(&self.query, &all)
            .into_iter()
            .take(MAX_RESULTS)
            .cloned()
            .collect()
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
            coin: registry::coin(registry::DEFAULT_COIN).expect("default coin"),
            addresses: BTreeMap::new(),
            verified: BTreeSet::new(),
            saves: 0,
            balances: None,
            loading: false,
            error: String::new(),
            query: String::new(),
            searching: false,
            highlight: 0,
            preview: None,
            preview_error: String::new(),
            looking: false,
            tab: Tab::Send,
            to: String::new(),
            amount: String::new(),
            review: None,
            review_leaving: false,
            preparing: false,
            sending: false,
            sent: 0,
            last_sent: String::new(),
            activity: Vec::new(),
            generation: 0,
            _fade: None,
            _blur: None,
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
                (self.addresses, self.verified) = addresses(&Self::address(ctx), &settings);
                // Record the public addresses this browser could derive, once.
                let fresh: BTreeMap<_, _> = self
                    .addresses
                    .iter()
                    .filter(|(k, v)| settings.addresses.get(*k) != Some(*v))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if !fresh.is_empty() {
                    self.save(
                        ctx,
                        "PATCH",
                        "/api/wallet".into(),
                        Some(json!({"addresses": fresh})),
                    );
                }
                self.coin = settings.active_coin();
                if !self.coin.can_send() {
                    self.tab = Tab::Receive;
                }
                self.settings = Some(settings);
                ctx.link().send_message(Msg::Refresh);
            }
            Msg::Saved(seq, _) if seq != self.saves => return false,
            Msg::Saved(_, result) => match result {
                Ok(settings) => {
                    let coin = settings.active_coin();
                    self.settings = Some(settings);
                    self.show(ctx, coin);
                }
                Err(e) => self.error = e,
            },
            Msg::Choose(key) => {
                if self.sending {
                    return false;
                }
                let Some(coin) = self
                    .settings
                    .as_ref()
                    .and_then(|s| s.all_coins().into_iter().find(|c| c.key == key))
                else {
                    return false;
                };
                self.searching = false;
                self.query.clear();
                self.preview = None;
                if let Some(s) = &mut self.settings {
                    s.active = key.clone();
                    s.recent.retain(|k| *k != key);
                    s.recent.insert(0, key.clone());
                }
                self.show(ctx, coin);
                self.save(
                    ctx,
                    "PATCH",
                    "/api/wallet".into(),
                    Some(json!({"active": key})),
                );
            }
            Msg::Query(v) => {
                self.query = v;
                self.searching = true;
                self.highlight = 0;
                self.preview = None;
                self.preview_error.clear();
            }
            Msg::Searching(open) => {
                self._blur = None;
                self.searching = open;
            }
            Msg::Blurred => {
                // Late enough that a click on a result lands first.
                let link = ctx.link().clone();
                self._blur = Some(Timeout::new(160, move || {
                    link.send_message(Msg::Searching(false))
                }));
                return false;
            }
            Msg::Key(e) => {
                let rows = self.results();
                match e.key().as_str() {
                    "ArrowDown" => {
                        e.prevent_default();
                        self.searching = true;
                        self.highlight = (self.highlight + 1).min(rows.len().saturating_sub(1));
                    }
                    "ArrowUp" => {
                        e.prevent_default();
                        self.highlight = self.highlight.saturating_sub(1);
                    }
                    "Enter" => {
                        e.prevent_default();
                        if let Some(coin) = rows.get(self.highlight) {
                            ctx.link().send_message(Msg::Choose(coin.key.clone()));
                        }
                    }
                    "Escape" => self.searching = false,
                    _ => return false,
                }
            }
            Msg::LookUp(network) => {
                let Some(evm_network) = evm::network(network) else {
                    return false;
                };
                if self.looking {
                    return false;
                }
                self.looking = true;
                self.preview = None;
                self.preview_error.clear();
                let address = self.query.trim().to_string();
                ctx.link().send_future(async move {
                    Msg::LookedUp(network.into(), look_up(evm_network, address).await)
                });
            }
            Msg::LookedUp(network, result) => {
                self.looking = false;
                self.searching = false;
                match result {
                    Ok(token) => self.preview = Some((network, token)),
                    Err(e) => self.preview_error = e,
                }
            }
            Msg::AddToken => {
                let Some((network, token)) = self.preview.take() else {
                    return false;
                };
                self.query.clear();
                self.searching = false;
                self.save(
                    ctx,
                    "POST",
                    "/api/wallet/tokens".into(),
                    Some(json!({"network": network, "token": token})),
                );
            }
            Msg::RemoveToken => {
                if !self.coin.custom
                    || self.sending
                    || !account::confirm(
                        "Remove this token from your list? Its balance stays on the chain, and you can add it again any time.",
                    )
                {
                    return false;
                }
                self.save(
                    ctx,
                    "DELETE",
                    format!("/api/wallet/tokens/{}/{}", self.coin.network, self.coin.id),
                    None,
                );
            }
            Msg::Tab(tab) => {
                if tab == Tab::Send && !self.coin.can_send() {
                    return false;
                }
                self.tab = tab;
            }
            Msg::Refresh => {
                let Some(address) = self.receive_address() else {
                    return true;
                };
                if self.settings.is_none() {
                    return true;
                }
                self.generation += 1;
                self.loading = true;
                let (generation, coin) = (self.generation, self.coin.clone());
                ctx.link().send_future(async move {
                    Msg::Loaded(generation, coin_balances(coin, address).await)
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
                if let (Some(b), Some((network, asset))) = (&self.balances, self.coin.evm_asset()) {
                    // The network's coin keeps back the fee for the send itself.
                    let max = match asset {
                        Asset::Native => evm::max_native(
                            b.native,
                            b.gas_price,
                            if network.exact_native_gas {
                                evm::NATIVE_GAS
                            } else {
                                evm::FALLBACK_GAS
                            },
                        ),
                        _ => b.asset,
                    };
                    self.amount = evm::format_units(max, asset.decimals(&network));
                }
            }
            Msg::Review => {
                let Some(have) = self.balances.clone() else {
                    return false;
                };
                if self.preparing || self.sending || !self.coin.can_send() {
                    return false;
                }
                self.preparing = true;
                self.error.clear();
                let (generation, coin, from) =
                    (self.generation, self.coin.clone(), Self::address(ctx));
                let (to, amount) = (self.to.clone(), self.amount.clone());
                ctx.link().send_future(async move {
                    Msg::Prepared(generation, prepare(coin, from, to, amount, have).await)
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
                let Some((network, _)) = self.coin.evm_asset() else {
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
                        self.sent += 1;
                        self.last_sent =
                            format!("{}|{}|{}", self.sent, prepared.amount, prepared.symbol);
                        let network = self
                            .coin
                            .evm_asset()
                            .map(|(n, _)| n)
                            .unwrap_or(evm::CRONOS_MAINNET);
                        self.activity.insert(
                            0,
                            Activity {
                                network,
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
        let link = ctx.link();
        let intro = html! {<div class="wallet-intro"><div class="eyebrow">{icon("wallet")}{t("YOUR WALLET")}</div><h1>{t("Every coin, one calm place.")}</h1></div>};
        if self.settings.is_none() {
            return html! {<section class="wallet-page" data-testid="wallet-page">{intro}<div class="wallet-skeleton" role="status" aria-label={t("Loading your wallet…")}><span/><span/><span/></div></section>};
        };
        let coin = &self.coin;
        let net = coin.net();
        let unlocked = Self::unlocked(ctx);
        let receive = self.receive_address();
        let held = self
            .balances
            .as_ref()
            .map(|b| evm::format_units(b.asset, coin.decimals));
        let fee = self
            .balances
            .as_ref()
            .map(|b| evm::format_units(b.native, net.decimals));
        let native = coin.standard == Standard::Native;
        let no_fee = self.balances.as_ref().is_some_and(|b| b.native.is_zero());
        let state = if self.sending {
            "sending"
        } else if self.loading {
            "loading"
        } else {
            "ready"
        };
        let kind = if native {
            "native"
        } else if coin.tags.iter().any(|t| t == "usd") {
            "usd"
        } else {
            "token"
        };
        let summary = match &held {
            Some(h) => format!("{} {}: {h} {}", t("Wallet card"), net.name, coin.symbol),
            None => format!("{} {}", t("Wallet card"), net.name),
        };
        let holder = ctx.props().session["name"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| username(&Self::address(ctx)).ok())
            .unwrap_or_default();

        // The two main buttons.
        let main = MAIN_COINS.iter().map(|key| {
            let on = coin.key == *key;
            let (label, sub) = if *key == "cro-cronos-mainnet" {
                ("CRONOS MAINNET", t("CRO · pays fees"))
            } else {
                ("USDC", t("Stablecoin on Cronos"))
            };
            html! {<button aria-pressed={on.to_string()} data-testid={format!("main-{key}")} disabled={self.sending} onclick={link.callback(move |_| Msg::Choose((*key).into()))}><strong>{label}</strong><small>{sub}</small></button>}
        });

        // Search: every coin on every network, plus "add this contract" rows.
        let rows = self.results();
        let query_address = evm::parse_address(self.query.trim()).is_ok();
        let search = html! {<div class={classes!("wallet-search", self.searching.then_some("is-open"))}>
            <label class="wallet-search-box">{icon("search")}
                <input type="search" role="combobox" aria-expanded={self.searching.to_string()} aria-controls="wallet-search-results" aria-label={t("Search coins and networks")} placeholder={t("Search coins, networks or a contract address")} autocomplete="off" spellcheck="false" value={self.query.clone()}
                    oninput={link.callback(|e: InputEvent| Msg::Query(e.target_unchecked_into::<HtmlInputElement>().value()))}
                    onfocus={link.callback(|_| Msg::Searching(true))} onblur={link.callback(|_| Msg::Blurred)} onkeydown={link.callback(Msg::Key)}/>
                <kbd>{rows.len()}</kbd>
            </label>
            if self.searching {
                <div class="wallet-search-panel" data-motion="">
                    <ul id="wallet-search-results" role="listbox" aria-label={t("Coins")} data-stagger="" data-testid="search-results">
                        {for rows.iter().enumerate().map(|(i, c)| {
                            let n = c.net();
                            let key = c.key.clone();
                            let on = c.key == coin.key;
                            html! {<li role="option" aria-selected={on.to_string()} data-key={c.key.clone()} class={classes!((i == self.highlight).then_some("is-highlighted"))}
                                onmousedown={link.callback(move |e: MouseEvent| { e.prevent_default(); Msg::Choose(key.clone()) })}>
                                <span class={classes!("coin-badge", n.chain.key())}>{c.symbol.chars().take(4).collect::<String>()}</span>
                                <span class="coin-text"><strong>{&c.symbol}</strong><span>{&c.name}</span></span>
                                <span class="coin-net">{n.name}{if n.testnet { html!{<em>{t("test")}</em>} } else { html!{} }}</span>
                                <span class={classes!("coin-can", c.can_send().then_some("send"))}>{if c.can_send() { t("Send") } else { t("View") }}</span>
                            </li>}
                        })}
                    </ul>
                    if rows.is_empty() && !query_address {<p class="small-muted wallet-search-empty">{t("Nothing matches. Try a symbol, a network or a contract address.")}</p>}
                    if query_address {
                        <div class="wallet-search-add" data-testid="search-add">
                            <span class="small-muted">{t("Add this contract as a token on")}</span>
                            {for evm::NETWORKS.iter().map(|n| { let key = n.key; html! {<button class="chip-button" disabled={self.looking} onmousedown={link.callback(move |e: MouseEvent| { e.prevent_default(); Msg::LookUp(key) })}>{n.name}</button>} })}
                        </div>
                    }
                </div>
            }
        </div>};
        let preview = self.preview.as_ref().map(|(network, token)| {
            let n = registry::net_of(network).map(|n| n.name).unwrap_or_default();
            let lookalike = evm::network(network).is_some_and(|en| evm::imitates_builtin(&en, token));
            html! {<div class="wallet-token-preview" data-motion="" data-testid="token-preview">
                <span class="wallet-asset-big">{&token.symbol}</span><span class="wallet-asset-name">{&token.name}</span>
                <dl><dt>{t("Network")}</dt><dd>{n}</dd><dt>{t("Contract")}</dt><dd><code>{&token.address}</code></dd><dt>{t("Decimals")}</dt><dd>{token.decimals}</dd></dl>
                if lookalike {<p class="wallet-warning" role="alert">{icon("info")}{t("This token's name looks like CRO or USDC, but it is a different contract. Check the address before sending.")}</p>}
                <button class="primary-button" data-testid="add-token" onclick={link.callback(|_| Msg::AddToken)}>{t("Add and use this token")}{icon("arrow")}</button>
            </div>}
        });

        // Exactly which coin this is, large: symbol, name, network, contract.
        let id_link = match net.chain {
            Chain::Evm | Chain::Solana if !coin.id.is_empty() => Some(net.address_url(&coin.id)),
            _ => None,
        };
        let lookalike = coin.custom
            && evm::network(&coin.network).is_some_and(|en| {
                evm::imitates_builtin(
                    &en,
                    &CustomToken {
                        address: coin.id.clone(),
                        symbol: coin.symbol.clone(),
                        name: coin.name.clone(),
                        decimals: coin.decimals,
                    },
                )
            });
        let identity = html! {<div class={classes!("wallet-coin", net.chain.key())} data-testid="asset-identity" aria-live="polite">
            <div class="wallet-coin-head">
                <span class="wallet-asset-big" data-testid="asset-symbol">{&coin.symbol}</span>
                <span class="wallet-coin-tags">
                    <span class={classes!("chain-pill", net.chain.key())}>{net.chain.name()}</span>
                    <span class="net-pill" data-testid="asset-network">{net.name}</span>
                    if net.testnet {<span class="test-pill">{t("Test network")}</span>}
                    if !coin.can_send() {<span class="view-pill">{t("View and receive")}</span>}
                </span>
            </div>
            <p class="wallet-asset-name">{&coin.name}</p>
            if coin.id.is_empty() {
                <p class="wallet-coin-id small-muted">{format!("{} {}", t("The network's own coin. It pays every fee on"), net.name)}</p>
            } else {
                <div class="wallet-coin-id"><span class="small-muted">{match coin.standard { Standard::SplToken => t("Mint"), Standard::CardanoAsset => t("Asset"), _ => t("Contract") }}</span>
                    <code data-testid="asset-contract">{coin.id.clone()}</code>
                    <button class="icon-button" aria-label={t("Copy contract address")} onclick={ctx.props().copy.reform({ let c = coin.id.clone(); move |_| c.clone() })}>{icon("copy")}</button>
                    if let Some(href) = id_link {<a href={href} target="_blank" rel="noopener noreferrer">{t("Explorer")}{icon("arrow")}</a>}
                </div>
            }
            if lookalike {<p class="wallet-warning" role="alert" data-testid="lookalike-warning">{icon("info")}{t("This token's name looks like CRO or USDC, but it is a different contract. Check the address before sending.")}</p>}
            if coin.custom {<button class="text-button" onclick={link.callback(|_| Msg::RemoveToken)}>{t("Remove from my list")}</button>}
        </div>};

        let balances = html! {<div class="wallet-balance-strip" aria-live="polite">
            <div class="wallet-balance-main"><span class="small-muted">{t("Balance")}</span>
                <strong><span data-testid="asset-balance">{held.clone().unwrap_or_else(|| "—".into())}</span><em>{&coin.symbol}</em></strong></div>
            <div class={classes!("wallet-fee", no_fee.then_some("is-empty"), native.then_some("is-native"))} data-testid="fee-balance">
                {icon("info")}
                <span>{if native { format!("{} {}", t("Network fees are paid from this balance, in"), net.symbol) } else { format!("{} {}", t("Network fees are paid in"), net.symbol) }}</span>
                <strong data-testid="native-balance">{fee.clone().map(|f| format!("{f} {}", net.symbol)).unwrap_or_else(|| "—".into())}</strong>
                if no_fee && coin.can_send() {<small>{format!("{} {}", t("Add some of this network's coin to pay fees before sending:"), net.symbol)}</small>}
            </div>
            <button class="icon-button" aria-label={t("Refresh balances")} disabled={self.loading || receive.is_none()} onclick={link.callback(|_| Msg::Refresh)}>{icon("refresh")}</button>
        </div>};

        let receive_panel = match &receive {
            Some(address) => {
                html! {<div class="wallet-receive" data-motion="" data-testid="receive-panel">
                    <span class="small-muted">{t("Your address on")}{" "}{net.name}</span>
                    <code class="wallet-address" data-testid="receive-address" aria-label={t("Your wallet address")}>{address.clone()}</code>
                    <div class="wallet-receive-actions">
                        <button class="secondary-button" onclick={ctx.props().copy.reform({ let a = address.clone(); move |_| a.clone() })}>{icon("copy")}{t("Copy address")}</button>
                        <a class="text-button" href={net.address_url(address)} target="_blank" rel="noopener noreferrer">{t("Explorer")}{icon("arrow")}</a>
                        if let Some(faucet) = net.faucet {<a class="text-button" href={faucet} target="_blank" rel="noopener noreferrer">{t("Faucet")}{icon("arrow")}</a>}
                    </div>
                    if !self.verified.contains(net.key) {<p class="wallet-warning" role="note" data-testid="address-unverified">{icon("info")}{t("Recorded from an earlier sign-in with your recovery phrase; this browser cannot re-derive it. Sign in with the phrase here to confirm it before sharing it for large amounts.")}</p>}
                if !coin.can_send() {<p class="small-muted">{format!("{} {}", t("Sending is not available here yet. You can receive and view on"), net.chain.name())}</p>}
                </div>}
            }
            None => {
                html! {<div class="wallet-locked" data-motion="" role="status" data-testid="phrase-needed"><h2>{t("Recovery phrase needed")}</h2><p>{format!("{} {}", net.chain.name(), t("addresses come from your recovery phrase. Sign out and sign in with it once on this browser to see them; they are then remembered for your account."))}</p></div>}
            }
        };
        let send_panel = if !unlocked {
            html! {<div class="wallet-locked" role="status"><h2>{t("Wallet locked on this browser")}</h2><p>{t("Sign out and sign in again with your recovery phrase or private key to save this wallet's key in this browser.")}</p></div>}
        } else if let Some(p) = &self.review {
            html! {<div class={classes!("wallet-review", self.review_leaving.then_some("is-leaving"))} data-motion="" data-testid="wallet-review" role="dialog" aria-label={t("Review transfer")}>
                <div class="eyebrow">{t("REVIEW")}</div>
                <p class="wallet-review-amount"><strong>{&p.amount}</strong>{" "}<span data-testid="review-symbol">{&p.symbol}</span></p>
                <dl>
                    <dt>{t("To")}</dt><dd><code>{&p.to}</code></dd>
                    <dt>{t("Network")}</dt><dd>{net.name}</dd>
                    if !coin.id.is_empty() {<dt>{t("Token contract")}</dt><dd><code>{coin.id.clone()}</code></dd>}
                    <dt>{t("Network fee, at most")}</dt><dd>{&p.fee}{" "}{net.symbol}</dd>
                    if let Some(total) = &p.total {<dt>{t("Total, at most")}</dt><dd><strong>{total}{" "}{net.symbol}</strong></dd>}
                </dl>
                <div class="modal-footer">
                    <button class="text-button" disabled={self.sending} onclick={link.callback(|_| Msg::Cancel)}>{t("Cancel")}</button>
                    <button class="primary-button" data-testid="wallet-confirm" data-flight-origin="" disabled={self.sending || self.review_leaving} onclick={link.callback(|_| Msg::Send)}>{if self.sending { t("Sending…") } else { t("Confirm and send") }}{icon("send")}</button>
                </div>
            </div>}
        } else {
            html! {<form class="wallet-send" data-motion="" onsubmit={link.callback(|e: SubmitEvent| { e.prevent_default(); Msg::Review })}>
                <div class="wallet-amount-hero" aria-hidden="true"><span class="wallet-amount-roll" data-roll="" data-testid="amount-roll" data-value={self.amount.trim().to_string()}></span><em>{&coin.symbol}</em></div>
                <label class="form-label">{t("Amount")}<span class="wallet-amount-field"><input type="text" inputmode="decimal" aria-label={t("Amount")} placeholder="0.00" autocomplete="off" value={self.amount.clone()} oninput={link.callback(|e: InputEvent| Msg::Amount(e.target_unchecked_into::<HtmlInputElement>().value()))}/><span class="wallet-amount-symbol" data-testid="send-symbol">{&coin.symbol}</span><button type="button" class="chip-button" onclick={link.callback(|_| Msg::Max)}>{t("Max")}</button></span></label>
                <label class="form-label">{t("Recipient address")}<input type="text" aria-label={t("Recipient address")} placeholder="0x…" autocomplete="off" spellcheck="false" value={self.to.clone()} oninput={link.callback(|e: InputEvent| Msg::To(e.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                <button class="primary-button" type="submit" disabled={self.preparing || self.balances.is_none() || self.to.trim().is_empty() || self.amount.trim().is_empty()}>{if self.preparing { t("Checking…") } else { t("Review transfer") }}{icon("arrow")}</button>
            </form>}
        };
        let tab = |which: Tab, label: &'static str, enabled: bool| {
            let on = self.tab == which;
            html! {<button role="tab" aria-selected={on.to_string()} disabled={!enabled || self.sending} onclick={link.callback(move |_| Msg::Tab(which))}>{t(label)}</button>}
        };

        html! {<section class={classes!("wallet-page", format!("chain-{}", net.chain.key()))} data-testid="wallet-page">
            {intro}
            <div class="wallet-stage">
                <cwb-wallet-card role="img" aria-label={summary} data-testid="wallet-card" data-network={net.key} data-coin={coin.key.clone()} data-state={state} data-kind={kind}
                    data-address={receive.clone().unwrap_or_default()} data-holder={holder} data-symbol={coin.symbol.clone()} data-native-symbol={net.symbol}
                    data-balance={held.clone().unwrap_or_else(|| "0".into())} data-native={fee.clone().unwrap_or_else(|| "0".into())} data-send={self.last_sent.clone()}></cwb-wallet-card>
            </div>
            <div class="wallet-choose">
                <div class="wallet-main" role="group" aria-label={t("Main coins")} data-glider=""><span class="glider" aria-hidden="true"></span>{for main}</div>
                {search}
            </div>
            {for preview}
            if !self.preview_error.is_empty() {<p class="login-error" role="alert">{t(&self.preview_error)}</p>}
            if !net.testnet && coin.can_send() {<p class="wallet-mainnet-note" role="note">{icon("info")}{t("Mainnet moves real funds. Double-check every address.")}</p>}
            {identity}
            {balances}
            <div class="wallet-tabs filter-tabs" role="tablist" aria-label={t("Wallet actions")} data-glider=""><span class="glider" aria-hidden="true"></span>{tab(Tab::Send, "Send", coin.can_send())}{tab(Tab::Receive, "Receive", true)}</div>
            <div class="wallet-panel">{if self.tab == Tab::Send && coin.can_send() { send_panel } else { receive_panel }}</div>
            if !self.error.is_empty() {<p class="login-error" role="alert" data-motion="">{t(&self.error)}</p>}
            if !self.activity.is_empty() {
                <div class="wallet-activity" data-motion=""><h3>{t("This session")}</h3><ul data-stagger="">{for self.activity.iter().map(|a| html! {
                    <li data-testid="wallet-activity"><span><strong>{&a.amount}{" "}{&a.symbol}</strong><small>{t("to")}{" "}{short(&a.to)}</small></span><span class={classes!("wallet-status", a.status.to_lowercase())}>{t(a.status)}</span><a href={registry::net_of(a.network.key).map(|n| n.tx_url(&a.hash)).unwrap_or_default()} target="_blank" rel="noopener noreferrer">{t("Explorer")}{icon("arrow")}</a></li>
                })}</ul></div>
            }
        </section>}
    }
}
