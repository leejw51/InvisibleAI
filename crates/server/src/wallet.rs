//! Wallet choices per account: the active coin, recent picks, ERC-20s the user
//! added and the receive address on each network, so a later sign-in opens the
//! wallet as it was left. Only public information is kept: keys and phrases
//! stay in the browser and never reach the backend.
use crate::{Api, ApiError, PrivateState, db};
use anyhow::Result;
use axum::{Json, extract::Path, http::StatusCode};
use causewaybay_shared::evm::{self, CustomToken, MAX_TOKENS, U256};
use causewaybay_shared::registry::{
    self, Chain, Coin, LIMITED_COINS, MAX_RECENT, Net, PendingTransfer, WalletSettings,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use std::collections::BTreeMap;

/// A prepared transfer the user has not confirmed within this long is dropped
/// from the wallet's pending list: the chat that asked for it has moved on.
pub const PENDING_HOURS: i64 = 1;

pub fn init(conn: &Connection) -> Result<()> {
    crate::schema::ensure(
        conn,
        "CREATE TABLE IF NOT EXISTS wallet_state(owner TEXT PRIMARY KEY,active TEXT NOT NULL,recent TEXT NOT NULL DEFAULT '[]',updated_at TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS wallet_tokens(owner TEXT NOT NULL,network TEXT NOT NULL,address TEXT NOT NULL,symbol TEXT NOT NULL,name TEXT NOT NULL,decimals INTEGER NOT NULL,added_at TEXT NOT NULL,PRIMARY KEY(owner,network,address));
         CREATE TABLE IF NOT EXISTS wallet_addresses(owner TEXT NOT NULL,network TEXT NOT NULL,address TEXT NOT NULL,PRIMARY KEY(owner,network));
         CREATE TABLE IF NOT EXISTS wallet_usage(owner TEXT NOT NULL,coin TEXT NOT NULL,uses INTEGER NOT NULL DEFAULT 0,last_used TEXT NOT NULL,PRIMARY KEY(owner,coin));
         CREATE TABLE IF NOT EXISTS wallet_limits(owner TEXT NOT NULL,coin TEXT NOT NULL,max_amount TEXT NOT NULL,updated_at TEXT NOT NULL,PRIMARY KEY(owner,coin));
         CREATE TABLE IF NOT EXISTS wallet_requests(id TEXT PRIMARY KEY,owner TEXT NOT NULL,memo_id TEXT NOT NULL,coin TEXT NOT NULL,to_address TEXT NOT NULL,amount TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',hash TEXT NOT NULL DEFAULT '',from_before TEXT NOT NULL DEFAULT '',to_before TEXT NOT NULL DEFAULT '',from_after TEXT NOT NULL DEFAULT '',to_after TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS wallet_requests_owner ON wallet_requests(owner,status,created_at);",
    )
}

/// Erasing an account leaves no wallet choice, token or address behind,
/// including rows an earlier build of this page kept.
pub fn forget_owner(conn: &Connection, owner: &str) -> Result<()> {
    for table in [
        "wallet_state",
        "wallet_tokens",
        "wallet_addresses",
        "wallet_usage",
        "wallet_limits",
        "wallet_requests",
        "wallet_settings",
        "wallet_selections",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |r| r.get(0),
        )?;
        if exists {
            conn.execute(&format!("DELETE FROM {table} WHERE owner=?1"), [owner])?;
        }
    }
    Ok(())
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}
fn net(key: &str) -> Result<&'static Net, ApiError> {
    registry::net_of(key).ok_or_else(|| bad("Unknown network."))
}
fn evm_net(key: &str) -> Result<evm::Network, ApiError> {
    net(key)?
        .evm
        .ok_or_else(|| bad("Tokens can be added on Cronos and Arbitrum."))
}

pub fn read(conn: &Connection, owner: &str) -> Result<WalletSettings> {
    let mut settings = WalletSettings::default();
    if let Some((active, recent)) = conn
        .query_row(
            "SELECT active,recent FROM wallet_state WHERE owner=?1",
            [owner],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        settings.active = active;
        settings.recent = serde_json::from_str(&recent).unwrap_or_default();
    }
    let mut rows = conn.prepare(
        "SELECT network,address,symbol,name,decimals FROM wallet_tokens WHERE owner=?1 ORDER BY added_at,address",
    )?;
    for row in rows.query_map([owner], |r| {
        Ok((
            r.get::<_, String>(0)?,
            CustomToken {
                address: r.get(1)?,
                symbol: r.get(2)?,
                name: r.get(3)?,
                decimals: r.get(4)?,
            },
        ))
    })? {
        let (network, token) = row?;
        settings.tokens.entry(network).or_default().push(token);
    }
    for row in conn
        .prepare("SELECT coin,uses FROM wallet_usage WHERE owner=?1")?
        .query_map([owner], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
        })?
    {
        let (coin, uses) = row?;
        settings.uses.insert(coin, uses);
    }
    for row in conn
        .prepare("SELECT network,address FROM wallet_addresses WHERE owner=?1")?
        .query_map([owner], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
    {
        let (network, address) = row?;
        settings.addresses.insert(network, address);
    }
    for row in conn
        .prepare("SELECT coin,max_amount FROM wallet_limits WHERE owner=?1")?
        .query_map([owner], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
    {
        let (coin, amount) = row?;
        if LIMITED_COINS.contains(&coin.as_str()) {
            settings.limits.insert(coin, amount);
        }
    }
    settings.pending = requests(conn, owner, Some("pending"))?;
    // A coin that is no longer offered (a removed token) reads as the default.
    settings.active = settings.active_coin().key;
    Ok(settings)
}

/// This account's prepared transfers, newest first: those in `status`, or the
/// most recent of any status. Pending ones older than `PENDING_HOURS` are
/// left out: nobody is coming back to confirm them.
pub fn requests(
    conn: &Connection,
    owner: &str,
    status: Option<&str>,
) -> Result<Vec<PendingTransfer>> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(PENDING_HOURS)).to_rfc3339();
    let mut rows = conn.prepare(&format!(
        "SELECT {REQUEST_COLUMNS} FROM wallet_requests WHERE owner=?1 AND (?2 IS NULL OR status=?2) AND (status!='pending' OR created_at>=?3) ORDER BY created_at DESC LIMIT 20"
    ))?;
    let found = rows
        .query_map(params![owner, status, cutoff], request_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(found.into_iter().filter_map(with_coin).collect())
}
const REQUEST_COLUMNS: &str = "id,owner,coin,to_address,amount,status,hash,from_before,to_before,from_after,to_after,created_at";
/// A row of `wallet_requests` in `REQUEST_COLUMNS` order; the coin's symbol
/// and network are filled in by `with_coin`.
fn request_row(r: &rusqlite::Row) -> rusqlite::Result<PendingTransfer> {
    Ok(PendingTransfer {
        id: r.get(0)?,
        from: r.get(1)?,
        coin: r.get(2)?,
        to: r.get(3)?,
        amount: r.get(4)?,
        status: r.get(5)?,
        hash: r.get(6)?,
        from_before: r.get(7)?,
        to_before: r.get(8)?,
        from_after: r.get(9)?,
        to_after: r.get(10)?,
        created_at: r.get(11)?,
        symbol: String::new(),
        network: String::new(),
    })
}
/// A request of a coin the registry no longer lists is not shown.
fn with_coin(mut p: PendingTransfer) -> Option<PendingTransfer> {
    let coin = registry::coin(&p.coin)?;
    p.symbol = coin.symbol.clone();
    p.network = coin.network.clone();
    Some(p)
}

/// One request of this account by id, whatever its status or age.
pub fn request_by_id(conn: &Connection, owner: &str, id: &str) -> Result<Option<PendingTransfer>> {
    let found = conn
        .query_row(
            &format!("SELECT {REQUEST_COLUMNS} FROM wallet_requests WHERE id=?1 AND owner=?2"),
            params![id, owner],
            request_row,
        )
        .optional()?;
    Ok(found.and_then(with_coin))
}

/// Check a limit the user typed: a whole-coin decimal the coin can represent,
/// above zero and below `MAX_LIMIT`.
fn valid_limit(coin: &Coin, text: &str) -> Api<String> {
    let text = text.trim();
    let units = evm::parse_units(text, coin.decimals).map_err(bad)?;
    if units.is_zero() {
        return Err(bad("A limit must be above zero."));
    }
    let ceiling = evm::parse_units(registry::MAX_LIMIT, coin.decimals).map_err(bad)?;
    if units > ceiling {
        return Err(bad(format!(
            "A limit can be at most {} {}.",
            registry::MAX_LIMIT,
            coin.symbol
        )));
    }
    Ok(evm::format_units(units, coin.decimals))
}

/// Why a transfer of `amount` of `coin` may not be prepared by the AI, or the
/// amount in base units when it may. The recipient is checked for shape and
/// checksum, and must not be the wallet itself.
pub fn check_request(
    settings: &WalletSettings,
    owner: &str,
    coin_key: &str,
    to: &str,
    amount: &str,
) -> Result<(Coin, String, U256), String> {
    if !LIMITED_COINS.contains(&coin_key) {
        return Err(format!(
            "The AI can prepare transfers of {} only.",
            LIMITED_COINS
                .iter()
                .filter_map(|k| registry::coin(k))
                .map(|c| format!("{} on {}", c.symbol, c.net().name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let coin = registry::coin(coin_key).ok_or("Unknown coin.")?;
    let recipient = evm::checksum(&evm::parse_address(to)?);
    if recipient.eq_ignore_ascii_case(owner.trim()) {
        return Err(
            "That is the wallet's own address. Sending to yourself moves nothing and still costs a fee."
                .into(),
        );
    }
    let units = evm::parse_units(amount.trim(), coin.decimals)?;
    if units.is_zero() {
        return Err("Enter an amount greater than zero.".into());
    }
    let limit = settings
        .limit_for(&coin)
        .ok_or("This coin has no transfer limit configured.")?;
    if units > limit {
        return Err(format!(
            "{} {} is over this wallet's hard limit of {} {} per transfer. The user can change the limit under Wallet or AI settings.",
            evm::format_units(units, coin.decimals),
            coin.symbol,
            settings.limit_text(&coin.key).unwrap_or_default(),
            coin.symbol
        ));
    }
    Ok((coin, recipient, units))
}

/// Record a transfer the AI prepared for `memo`, for the wallet page to pick up.
pub fn create_request(
    conn: &Connection,
    owner: &str,
    memo: &str,
    coin: &Coin,
    to: &str,
    amount: &str,
) -> Result<PendingTransfer> {
    let request = PendingTransfer {
        id: db::id(),
        coin: coin.key.clone(),
        symbol: coin.symbol.clone(),
        network: coin.network.clone(),
        from: owner.into(),
        to: to.into(),
        amount: amount.into(),
        status: "pending".into(),
        created_at: db::now(),
        ..Default::default()
    };
    conn.execute(
        "INSERT INTO wallet_requests(id,owner,memo_id,coin,to_address,amount,status,hash,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'pending','',?7,?7)",
        params![request.id, owner, memo, request.coin, request.to, request.amount, request.created_at],
    )?;
    Ok(request)
}

/// Count one more use of `key`: choosing it, or sending it.
fn count_use(conn: &Connection, owner: &str, key: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO wallet_usage(owner,coin,uses,last_used) VALUES(?1,?2,1,?3) ON CONFLICT(owner,coin) DO UPDATE SET uses=min(uses+1,1000000),last_used=excluded.last_used",
        params![owner, key, db::now()],
    )?;
    Ok(())
}
/// Make `key` the active coin and the newest recent pick, and count the use.
fn activate(conn: &Connection, owner: &str, key: &str) -> Result<()> {
    count_use(conn, owner, key)?;
    let mut recent = read(conn, owner)?.recent;
    recent.retain(|k| k != key);
    recent.insert(0, key.to_string());
    recent.truncate(MAX_RECENT);
    conn.execute(
        "INSERT INTO wallet_state(owner,active,recent,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(owner) DO UPDATE SET active=excluded.active,recent=excluded.recent,updated_at=excluded.updated_at",
        params![owner, key, serde_json::to_string(&recent)?, db::now()],
    )?;
    Ok(())
}

/// A receive address as the browser derived it, in its chain's own form. On
/// EVM it must be the signed-in wallet itself; elsewhere it is checked for
/// shape only, since the backend never holds the key that would prove it.
fn valid_address(owner: &str, network: &Net, address: &str) -> Api<String> {
    let address = address.trim();
    match network.chain {
        Chain::Evm => {
            let bytes = evm::parse_address(address).map_err(bad)?;
            let checked = evm::checksum(&bytes);
            if owner.starts_with("0x") && !checked.eq_ignore_ascii_case(owner) {
                return Err(bad("The EVM address must be this wallet's own."));
            }
            Ok(checked)
        }
        _ => {
            if !(20..=128).contains(&address.len())
                || !address
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '_')
            {
                return Err(bad(format!("That is not a {} address.", network.name)));
            }
            Ok(address.to_string())
        }
    }
}

pub async fn get(PrivateState(s): PrivateState) -> Api<Json<WalletSettings>> {
    let conn = s.db.0.lock().unwrap();
    Ok(Json(read(&conn, s.db.owner())?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    addresses: Option<BTreeMap<String, String>>,
    /// A coin just sent: counted, without changing what is shown.
    #[serde(default)]
    used: Option<String>,
    /// Hard limits per transfer, coin key → whole coins; only `LIMITED_COINS`.
    #[serde(default)]
    limits: Option<BTreeMap<String, String>>,
}
/// Choose the active coin and/or record receive addresses derived in the browser.
pub async fn update(
    PrivateState(s): PrivateState,
    Json(v): Json<Update>,
) -> Api<Json<WalletSettings>> {
    let mut conn = s.db.0.lock().unwrap();
    s.db.writable(&conn)?;
    let owner = s.db.owner().to_string();
    let tx = conn.transaction()?;
    if let Some(addresses) = v.addresses {
        if addresses.len() > registry::NETS.len() {
            return Err(bad("Too many addresses."));
        }
        for (key, address) in addresses {
            let network = net(&key)?;
            let address = valid_address(&owner, network, &address)?;
            tx.execute(
                "INSERT INTO wallet_addresses(owner,network,address) VALUES(?1,?2,?3) ON CONFLICT(owner,network) DO UPDATE SET address=excluded.address",
                params![owner, network.key, address],
            )?;
        }
    }
    if let Some(key) = v.used {
        if !read(&tx, &owner)?.all_coins().iter().any(|c| c.key == key) {
            return Err(bad("Unknown coin."));
        }
        count_use(&tx, &owner, &key)?;
    }
    if let Some(key) = v.active {
        let offered = read(&tx, &owner)?.all_coins().iter().any(|c| c.key == key);
        if !offered {
            return Err(bad("Add this token before choosing it."));
        }
        activate(&tx, &owner, &key)?;
    }
    if let Some(limits) = v.limits {
        for (key, text) in limits {
            if !LIMITED_COINS.contains(&key.as_str()) {
                return Err(bad("Limits apply to CRO, TCRO and USDC on Cronos only."));
            }
            let coin = registry::coin(&key).ok_or_else(|| bad("Unknown coin."))?;
            let amount = valid_limit(&coin, &text)?;
            tx.execute(
                "INSERT INTO wallet_limits(owner,coin,max_amount,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(owner,coin) DO UPDATE SET max_amount=excluded.max_amount,updated_at=excluded.updated_at",
                params![owner, key, amount, db::now()],
            )?;
        }
    }
    tx.commit()?;
    Ok(Json(read(&conn, &owner)?))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestOutcome {
    /// `pending` (still, with the balances read before sending), `sent` (with
    /// the transaction hash) or `cancelled`.
    status: String,
    #[serde(default)]
    hash: Option<String>,
    /// Whole-coin balances the browser read: both before the transfer, and
    /// both once it arrived. Each is kept as sent and shown as is.
    #[serde(default)]
    from_before: Option<String>,
    #[serde(default)]
    to_before: Option<String>,
    #[serde(default)]
    from_after: Option<String>,
    #[serde(default)]
    to_after: Option<String>,
}
/// A balance the browser reports: a short decimal, or nothing.
fn valid_balance(value: Option<&str>) -> Api<Option<String>> {
    let Some(text) = value.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if text.len() > 48
        || !text.chars().all(|c| c.is_ascii_digit() || c == '.')
        || text.matches('.').count() > 1
    {
        return Err(bad("That is not a balance."));
    }
    Ok(Some(text.to_string()))
}
/// The browser reports on a transfer the AI prepared: the balances it read
/// before sending (the request stays `pending`), then `sent` with the hash,
/// then the balances after arrival; or `cancelled`. A request leaves
/// `pending` once only, and only its own sender's report can add the after
/// balances. The chat that asked is told through its memo's events.
pub async fn resolve_request(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Json(v): Json<RequestOutcome>,
) -> Api<Json<PendingTransfer>> {
    if !matches!(v.status.as_str(), "pending" | "sent" | "cancelled") {
        return Err(bad(
            "A request stays pending, or ends as sent or cancelled.",
        ));
    }
    let hash = match (v.status.as_str(), v.hash.as_deref()) {
        ("sent", Some(h)) => {
            let h = h.trim();
            if h.len() != 66
                || !h.starts_with("0x")
                || !h[2..].chars().all(|c| c.is_ascii_hexdigit())
            {
                return Err(bad("That is not a transaction hash."));
            }
            h.to_ascii_lowercase()
        }
        ("sent", None) => return Err(bad("A sent transfer names its transaction hash.")),
        _ => String::new(),
    };
    let from_before = valid_balance(v.from_before.as_deref())?;
    let to_before = valid_balance(v.to_before.as_deref())?;
    let from_after = valid_balance(v.from_after.as_deref())?;
    let to_after = valid_balance(v.to_after.as_deref())?;
    let owner = s.db.owner().to_string();
    let memo = {
        let conn = s.db.0.lock().unwrap();
        s.db.writable(&conn)?;
        let now = db::now();
        let changed = match v.status.as_str() {
            // Still pending: only the balances read before sending are recorded.
            "pending" => conn.execute(
                "UPDATE wallet_requests SET from_before=coalesce(?3,from_before),to_before=coalesce(?4,to_before),updated_at=?5 WHERE id=?1 AND owner=?2 AND status='pending'",
                params![id, owner, from_before, to_before, now],
            )?,
            status => {
                let left = conn.execute(
                    "UPDATE wallet_requests SET status=?3,hash=?4,from_before=coalesce(?5,from_before),to_before=coalesce(?6,to_before),from_after=coalesce(?7,from_after),to_after=coalesce(?8,to_after),updated_at=?9 WHERE id=?1 AND owner=?2 AND status='pending'",
                    params![id, owner, status, hash, from_before, to_before, from_after, to_after, now],
                )?;
                if left == 0 && status == "sent" {
                    // Already sent: the same transaction's arrival adds the balances after.
                    conn.execute(
                        "UPDATE wallet_requests SET from_after=coalesce(?4,from_after),to_after=coalesce(?5,to_after),updated_at=?6 WHERE id=?1 AND owner=?2 AND status='sent' AND hash=?3",
                        params![id, owner, hash, from_after, to_after, now],
                    )?
                } else {
                    left
                }
            }
        };
        if changed == 0 {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                "No pending transfer with that id".into(),
            ));
        }
        conn.query_row(
            "SELECT memo_id FROM wallet_requests WHERE id=?1",
            [&id],
            |r| r.get::<_, String>(0),
        )?
    };
    let request = {
        let conn = s.db.0.lock().unwrap();
        request_by_id(&conn, &owner, &id)?
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Transfer not found".into()))?
    };
    // The chat card follows the outcome; a memo that is gone has no card.
    if s.db.require("memos", &memo).is_ok() {
        s.db.event(
            &memo,
            "wallet_request",
            &serde_json::to_string(&request).map_err(anyhow::Error::from)?,
        )?;
    }
    Ok(Json(request))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewToken {
    network: String,
    token: CustomToken,
}
/// Save an ERC-20 the browser read from its contract, and make it active.
pub async fn add_token(
    PrivateState(s): PrivateState,
    Json(v): Json<NewToken>,
) -> Api<Json<WalletSettings>> {
    let network = evm_net(&v.network)?;
    let token = evm::validate_token(&network, &v.token).map_err(bad)?;
    if registry::coins()
        .iter()
        .any(|c| c.network == network.key && c.id.eq_ignore_ascii_case(&token.address))
    {
        return Err(bad(
            "That token is already in the list. Search for it instead.",
        ));
    }
    let mut conn = s.db.0.lock().unwrap();
    s.db.writable(&conn)?;
    let owner = s.db.owner().to_string();
    let tx = conn.transaction()?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM wallet_tokens WHERE owner=?1 AND network=?2 AND lower(address)!=?3",
        params![owner, network.key, token.address.to_ascii_lowercase()],
        |r| r.get(0),
    )?;
    if count as usize >= MAX_TOKENS {
        return Err(bad(format!(
            "You can add up to {MAX_TOKENS} tokens per network. Remove one first."
        )));
    }
    tx.execute(
        "INSERT INTO wallet_tokens(owner,network,address,symbol,name,decimals,added_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(owner,network,address) DO UPDATE SET symbol=excluded.symbol,name=excluded.name,decimals=excluded.decimals",
        params![owner, network.key, token.address, token.symbol, token.name, token.decimals, db::now()],
    )?;
    activate(
        &tx,
        &owner,
        &registry::custom_key(network.key, &token.address),
    )?;
    tx.commit()?;
    Ok(Json(read(&conn, &owner)?))
}

/// Forget an added token; if it was active, the default coin is shown instead.
pub async fn remove_token(
    PrivateState(s): PrivateState,
    Path((network_key, address)): Path<(String, String)>,
) -> Api<Json<WalletSettings>> {
    let network = evm_net(&network_key)?;
    let address = evm::parse_address(&address)
        .map(|a| evm::checksum(&a))
        .map_err(bad)?;
    let mut conn = s.db.0.lock().unwrap();
    s.db.writable(&conn)?;
    let owner = s.db.owner().to_string();
    let tx = conn.transaction()?;
    let removed = tx.execute(
        "DELETE FROM wallet_tokens WHERE owner=?1 AND network=?2 AND lower(address)=?3",
        params![owner, network.key, address.to_ascii_lowercase()],
    )?;
    if removed == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "Token not found".into()));
    }
    let gone = registry::custom_key(network.key, &address);
    let mut recent = read(&tx, &owner)?.recent;
    recent.retain(|k| *k != gone);
    tx.execute(
        "UPDATE wallet_state SET active=CASE WHEN active=?2 THEN ?3 ELSE active END,recent=?4 WHERE owner=?1",
        params![owner, gone, registry::DEFAULT_COIN, serde_json::to_string(&recent).map_err(anyhow::Error::from)?],
    )?;
    tx.commit()?;
    Ok(Json(read(&conn, &owner)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardanoRead {
    network: String,
    path: String,
    address: String,
}
/// The Koios endpoint for a Cardano network; `CAUSEWAYBAY_RPC_<NETWORK>`
/// overrides it, as in the Causewaybay Wallet.
fn koios(network: &Net) -> String {
    let var = format!(
        "CAUSEWAYBAY_RPC_{}",
        network.key.to_uppercase().replace('-', "_")
    );
    std::env::var(var).unwrap_or_else(|_| network.endpoint.to_string())
}
/// Read a public Cardano address through Koios. Koios answers browsers'
/// preflight but omits CORS headers on the reply itself, so the page cannot
/// read it directly; this relays exactly two read-only queries for one
/// address, which is public information the backend already keeps.
pub async fn cardano(
    PrivateState(_): PrivateState,
    Json(v): Json<CardanoRead>,
) -> Api<Json<serde_json::Value>> {
    let network = net(&v.network)?;
    if network.chain != Chain::Cardano {
        return Err(bad("Only Cardano networks are read here."));
    }
    if !matches!(v.path.as_str(), "address_info" | "address_assets") {
        return Err(bad("Unsupported Cardano query."));
    }
    let address = v.address.trim();
    let prefix = if network.testnet {
        "addr_test1"
    } else {
        "addr1"
    };
    if !address.starts_with(prefix)
        || address.len() > 128
        || !address[prefix.len()..]
            .chars()
            .all(|c| "qpzry9x8gf2tvdw0s3jn54khce6mua7l".contains(c))
    {
        return Err(bad(format!("That is not a {} address.", network.name)));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let unreachable = |_| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("Could not reach {}.", network.name),
        )
    };
    let response = client
        .post(format!("{}/{}", koios(network), v.path))
        .json(&serde_json::json!({"_addresses": [address]}))
        .send()
        .await
        .map_err(unreachable)?;
    if !response.status().is_success() {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            format!("{} answered {}.", network.name, response.status().as_u16()),
        ));
    }
    let body = response.bytes().await.map_err(unreachable)?;
    // Every real reply is small; a huge one is a broken or hostile endpoint.
    if body.len() > 1 << 20 {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            format!("{} sent too much.", network.name),
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&body).map_err(|_| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("{} answered with something unexpected.", network.name),
        )
    })?;
    if !value.is_array() {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            format!("{} answered with something unexpected.", network.name),
        ));
    }
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use serde_json::json;

    const WCRO: &str = "0x5C7F8A570d578ED84E63fdFA7b1eE72dEae1AE23";
    const ALICE: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
    fn token(address: &str, symbol: &str) -> CustomToken {
        CustomToken {
            address: address.into(),
            symbol: symbol.into(),
            name: format!("{symbol} token"),
            decimals: 18,
        }
    }
    fn error<T>(result: Api<T>) -> String {
        match result {
            Err(ApiError(_, message)) => message,
            Ok(_) => panic!("expected an error"),
        }
    }
    fn change(v: serde_json::Value) -> Json<Update> {
        Json(serde_json::from_value(v).unwrap())
    }

    #[tokio::test]
    async fn choices_tokens_and_addresses_persist_per_owner_and_are_erased() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root =
            AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
        let alice = root.for_owner(ALICE);
        let bob = root.for_owner("0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc");
        let Json(fresh) = get(PrivateState(alice.clone())).await.unwrap();
        assert_eq!(fresh, WalletSettings::default());
        assert_eq!(fresh.active, "tcro-cronos-testnet");

        // Any registry coin on any chain can be active; recent keeps the order.
        for key in [
            "sol-solana-devnet",
            "cro-cronos-mainnet",
            "sol-solana-devnet",
        ] {
            let _ = update(PrivateState(alice.clone()), change(json!({"active":key})))
                .await
                .unwrap();
        }
        let Json(s) = get(PrivateState(alice.clone())).await.unwrap();
        assert_eq!(s.active, "sol-solana-devnet");
        assert_eq!(s.recent, ["sol-solana-devnet", "cro-cronos-mainnet"]);
        // Every choice counts; a send counts without changing what is shown.
        assert_eq!(s.uses["sol-solana-devnet"], 2);
        assert_eq!(s.uses["cro-cronos-mainnet"], 1);
        let Json(s) = update(
            PrivateState(alice.clone()),
            change(json!({"used":"cro-cronos-mainnet"})),
        )
        .await
        .unwrap();
        assert_eq!(
            (s.uses["cro-cronos-mainnet"], s.active.as_str()),
            (2, "sol-solana-devnet")
        );
        assert_eq!(s.ranked(registry::coins())[0].key, "sol-solana-devnet");
        assert_eq!(
            error(
                update(
                    PrivateState(alice.clone()),
                    change(json!({"used":"btc-bitcoin"}))
                )
                .await
            ),
            "Unknown coin."
        );

        // Public receive addresses, in each chain's own form.
        let Json(s) = update(
            PrivateState(alice.clone()),
            change(json!({"addresses":{
                "cronos-mainnet": ALICE,
                "solana-mainnet": "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk",
                "ecash-mainnet": "ecash:qqlyzuhlh9hvly4ntfx4rf7vzh7ervsvfgjr3ppsz6"
            }})),
        )
        .await
        .unwrap();
        assert_eq!(
            s.addresses["cronos-mainnet"],
            "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
        );
        assert_eq!(s.addresses.len(), 3);

        // An added token is checksummed, cleaned, made active and kept on its network.
        let Json(s) = add_token(
            PrivateState(alice.clone()),
            Json(NewToken {
                network: "cronos-mainnet".into(),
                token: CustomToken {
                    symbol: "WCRO\u{202E}".into(),
                    ..token(&WCRO.to_ascii_lowercase(), "WCRO")
                },
            }),
        )
        .await
        .unwrap();
        let key = registry::custom_key("cronos-mainnet", WCRO);
        assert_eq!(s.active, key);
        assert_eq!(s.active_coin().symbol, "WCRO");
        assert_eq!(s.tokens["cronos-mainnet"][0].address, WCRO);

        // Read back as a new sign-in would; bob sees none of it and cannot pick it.
        assert_eq!(get(PrivateState(alice.clone())).await.unwrap().0, s);
        assert_eq!(
            get(PrivateState(bob.clone())).await.unwrap().0,
            WalletSettings::default()
        );
        assert_eq!(
            error(update(PrivateState(bob.clone()), change(json!({"active":key}))).await),
            "Add this token before choosing it."
        );

        // Removing the active token falls back to the default and leaves recent.
        let Json(s) = remove_token(
            PrivateState(alice.clone()),
            Path(("cronos-mainnet".into(), WCRO.into())),
        )
        .await
        .unwrap();
        assert_eq!(s.active, "tcro-cronos-testnet");
        assert!(!s.recent.contains(&key));

        let _ = crate::chat::delete_all(PrivateState(alice.clone()))
            .await
            .unwrap();
        let conn = root.db.0.lock().unwrap();
        for table in [
            "wallet_state",
            "wallet_tokens",
            "wallet_addresses",
            "wallet_usage",
        ] {
            let n: i64 = conn.query_row(
                &format!("SELECT count(*) FROM {table} WHERE owner=?1"),
                [ALICE],
                |r| r.get(0),
            )?;
            assert_eq!(n, 0, "{table}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn only_public_valid_fields_are_kept() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?
            .for_owner(ALICE);
        let p = || PrivateState(s.clone());
        assert!(
            error(update(p(), change(json!({"active":"btc-bitcoin"}))).await)
                .contains("Add this token")
        );
        // A secret field is refused outright, at the top level or inside a token.
        assert!(
            serde_json::from_value::<Update>(
                json!({"active":"usdc-cronos-mainnet","mnemonic":"x"})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<NewToken>(json!({"network":"cronos-mainnet","token":{"address":WCRO,"symbol":"W","name":"","decimals":18,"private_key":"0x01"}})).is_err());
        // Addresses: known networks, this wallet's own EVM address, plausible shapes elsewhere.
        assert!(
            error(update(p(), change(json!({"addresses":{"bitcoin":"bc1qxyz"}}))).await)
                .contains("Unknown network")
        );
        assert!(error(update(p(), change(json!({"addresses":{"arbitrum-one":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"}}))).await).contains("this wallet's own"));
        assert!(
            error(
                update(
                    p(),
                    change(json!({"addresses":{"solana-mainnet":"<script>alert(1)</script>"}}))
                )
                .await
            )
            .contains("not a Solana")
        );
        // Tokens only on EVM networks, never a duplicate of a registry row, at most 50.
        let add = |network: &str, t: CustomToken| {
            add_token(
                p(),
                Json(NewToken {
                    network: network.into(),
                    token: t,
                }),
            )
        };
        assert!(
            error(add("solana-mainnet", token(WCRO, "W")).await).contains("Cronos and Arbitrum")
        );
        assert!(
            error(
                add(
                    "cronos-mainnet",
                    token("0x66e428c3f67a68878562e79A0234c1F83c208770", "USDT")
                )
                .await
            )
            .contains("already in the list")
        );
        assert!(
            error(
                add(
                    "arbitrum-one",
                    token(evm::ARBITRUM_ONE.usdc.address, "USDC")
                )
                .await
            )
            .contains("Choose USDC")
        );
        let _ = add("arbitrum-sepolia", token(WCRO, "W")).await.unwrap();
        let _ = add("arbitrum-sepolia", token(&WCRO.to_ascii_lowercase(), "W"))
            .await
            .unwrap();
        for i in 1..MAX_TOKENS {
            let _ = add("arbitrum-sepolia", token(&format!("0x{i:040x}"), "T"))
                .await
                .unwrap();
        }
        assert!(
            error(add("arbitrum-sepolia", token(&format!("0x{:040x}", 999), "T")).await)
                .contains("up to 50")
        );
        assert_eq!(
            get(p()).await.unwrap().0.tokens["arbitrum-sepolia"].len(),
            MAX_TOKENS
        );
        assert_eq!(
            error(remove_token(p(), Path(("cronos-mainnet".into(), WCRO.into()))).await),
            "Token not found"
        );
        Ok(())
    }

    #[tokio::test]
    async fn hard_limits_are_per_owner_validated_and_default_to_100_cro_and_10_usdc() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let root =
            AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
        let alice = root.for_owner(ALICE);
        let bob = root.for_owner("0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc");
        let Json(s) = get(PrivateState(alice.clone())).await.unwrap();
        assert!(s.limits.is_empty());
        assert_eq!(s.limit_text("cro-cronos-mainnet").as_deref(), Some("100"));
        assert_eq!(s.limit_text("usdc-cronos-mainnet").as_deref(), Some("10"));
        let Json(s) = update(
            PrivateState(alice.clone()),
            change(json!({"limits":{"usdc-cronos-mainnet":" 25.5 ","tcro-cronos-testnet":"7"}})),
        )
        .await
        .unwrap();
        assert_eq!(s.limits["usdc-cronos-mainnet"], "25.5");
        assert_eq!(s.limits["tcro-cronos-testnet"], "7");
        assert_eq!(s.limit_text("cro-cronos-mainnet").as_deref(), Some("100"));
        // Bob keeps the defaults; limits are per account.
        assert!(
            get(PrivateState(bob.clone()))
                .await
                .unwrap()
                .0
                .limits
                .is_empty()
        );
        for (body, message) in [
            (
                json!({"limits":{"sol-solana-mainnet":"1"}}),
                "CRO, TCRO and USDC",
            ),
            (json!({"limits":{"usdc-cronos-mainnet":"0"}}), "above zero"),
            (json!({"limits":{"usdc-cronos-mainnet":"abc"}}), ""),
            (
                json!({"limits":{"usdc-cronos-mainnet":"1.1234567"}}),
                "decimal",
            ),
            (
                json!({"limits":{"cro-cronos-mainnet":"1000000001"}}),
                "at most",
            ),
        ] {
            let error = error(update(PrivateState(alice.clone()), change(body.clone())).await);
            assert!(error.contains(message), "{body}: {error}");
        }
        // A refused limit changes nothing.
        let Json(s) = get(PrivateState(alice.clone())).await.unwrap();
        assert_eq!(s.limits.len(), 2);
        let _ = crate::chat::delete_all(PrivateState(alice.clone()))
            .await
            .unwrap();
        assert!(
            get(PrivateState(alice.clone()))
                .await
                .unwrap()
                .0
                .limits
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn prepared_transfers_are_checked_listed_and_resolved_once() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root =
            AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
        let alice = root.for_owner(ALICE);
        let recipient = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
        let settings = get(PrivateState(alice.clone())).await.unwrap().0;
        let refused = |coin: &str, to: &str, amount: &str| {
            check_request(&settings, ALICE, coin, to, amount).unwrap_err()
        };
        assert!(refused("devusdc.e-cronos-testnet", recipient, "1").contains("only"));
        assert!(refused("tcro-cronos-testnet", ALICE, "1").contains("own address"));
        assert!(
            refused(
                "tcro-cronos-testnet",
                "0xF39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                "1"
            )
            .contains("checksum")
        );
        assert!(refused("tcro-cronos-testnet", recipient, "0").contains("greater than zero"));
        assert!(
            refused("tcro-cronos-testnet", recipient, "100.5").contains("hard limit of 100 TCRO")
        );
        assert!(
            refused("usdc-cronos-mainnet", recipient, "10.000001")
                .contains("hard limit of 10 USDC")
        );
        let (coin, to, units) = check_request(
            &settings,
            ALICE,
            "usdc-cronos-mainnet",
            &recipient.to_ascii_lowercase(),
            "10",
        )
        .unwrap();
        assert_eq!((coin.symbol.as_str(), to.as_str()), ("USDC", recipient));
        assert_eq!(units, U256::from(10_000_000u64));

        let memo = alice.db.create("send 10 usdc", "question", "local", "")?;
        let request = {
            let conn = alice.db.0.lock().unwrap();
            create_request(&conn, ALICE, &memo.id, &coin, &to, "10")?
        };
        let Json(s) = get(PrivateState(alice.clone())).await.unwrap();
        assert_eq!(s.pending.len(), 1);
        assert_eq!(
            (
                s.pending[0].symbol.as_str(),
                s.pending[0].network.as_str(),
                s.pending[0].status.as_str()
            ),
            ("USDC", "cronos-mainnet", "pending")
        );
        // Another owner sees nothing and cannot resolve it.
        let bob = root.for_owner("0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc");
        assert!(
            get(PrivateState(bob.clone()))
                .await
                .unwrap()
                .0
                .pending
                .is_empty()
        );
        let outcome = |status: &str, hash: Option<&str>| {
            Json(RequestOutcome {
                status: status.into(),
                hash: hash.map(str::to_owned),
                ..Default::default()
            })
        };
        // Twenty-five newer requests push this one past the listing's cap;
        // resolving it still finds it by id.
        {
            let conn = alice.db.0.lock().unwrap();
            for _ in 0..25 {
                create_request(&conn, ALICE, &memo.id, &coin, &to, "1")?;
            }
        }
        let hash = format!("0x{}", "ab".repeat(32));
        assert!(
            resolve_request(
                PrivateState(bob.clone()),
                Path(request.id.clone()),
                outcome("sent", Some(&hash))
            )
            .await
            .is_err()
        );
        assert!(
            error(
                resolve_request(
                    PrivateState(alice.clone()),
                    Path(request.id.clone()),
                    outcome("sent", None)
                )
                .await
            )
            .contains("hash")
        );
        assert!(
            error(
                resolve_request(
                    PrivateState(alice.clone()),
                    Path(request.id.clone()),
                    outcome("sent", Some("0x12"))
                )
                .await
            )
            .contains("not a transaction hash")
        );
        assert!(
            error(
                resolve_request(
                    PrivateState(alice.clone()),
                    Path(request.id.clone()),
                    outcome("failed", None)
                )
                .await
            )
            .contains("sent or cancelled")
        );
        // Still pending: the browser reports the balances it read before sending.
        let balances =
            |status: &str, hash: Option<&str>, before: (&str, &str), after: (&str, &str)| {
                Json(RequestOutcome {
                    status: status.into(),
                    hash: hash.map(str::to_owned),
                    from_before: Some(before.0.into()).filter(|s: &String| !s.is_empty()),
                    to_before: Some(before.1.into()).filter(|s: &String| !s.is_empty()),
                    from_after: Some(after.0.into()).filter(|s: &String| !s.is_empty()),
                    to_after: Some(after.1.into()).filter(|s: &String| !s.is_empty()),
                })
            };
        assert!(
            error(
                resolve_request(
                    PrivateState(alice.clone()),
                    Path(request.id.clone()),
                    balances("pending", None, ("12e3", ""), ("", ""))
                )
                .await
            )
            .contains("not a balance")
        );
        let Json(read) = resolve_request(
            PrivateState(alice.clone()),
            Path(request.id.clone()),
            balances("pending", None, ("50", "0.5"), ("", "")),
        )
        .await
        .unwrap();
        assert_eq!(
            (
                read.status.as_str(),
                read.from.as_str(),
                read.from_before.as_str(),
                read.to_before.as_str()
            ),
            ("pending", ALICE, "50", "0.5")
        );
        let Json(done) = resolve_request(
            PrivateState(alice.clone()),
            Path(request.id.clone()),
            outcome("sent", Some(&hash.to_uppercase().replace("0X", "0x"))),
        )
        .await
        .unwrap();
        assert_eq!(
            (
                done.status.as_str(),
                done.hash.as_str(),
                done.from_before.as_str()
            ),
            ("sent", hash.as_str(), "50")
        );
        // Resolved once: a second outcome finds no pending request, and the
        // balances before cannot be rewritten.
        assert!(
            resolve_request(
                PrivateState(alice.clone()),
                Path(request.id.clone()),
                outcome("cancelled", None)
            )
            .await
            .is_err()
        );
        assert!(
            resolve_request(
                PrivateState(alice.clone()),
                Path(request.id.clone()),
                balances("pending", None, ("1", "1"), ("", ""))
            )
            .await
            .is_err()
        );
        // The same transaction's arrival adds the balances after; another hash cannot.
        let other = format!("0x{}", "cd".repeat(32));
        assert!(
            resolve_request(
                PrivateState(alice.clone()),
                Path(request.id.clone()),
                balances("sent", Some(&other), ("", ""), ("40", "10.5"))
            )
            .await
            .is_err()
        );
        let Json(arrived) = resolve_request(
            PrivateState(alice.clone()),
            Path(request.id.clone()),
            balances("sent", Some(&hash), ("", ""), ("40", "10.5")),
        )
        .await
        .unwrap();
        assert_eq!(
            (
                arrived.from_before.as_str(),
                arrived.from_after.as_str(),
                arrived.to_after.as_str()
            ),
            ("50", "40", "10.5")
        );
        {
            let conn = alice.db.0.lock().unwrap();
            conn.execute("DELETE FROM wallet_requests WHERE amount='1'", [])?;
        }
        assert!(
            get(PrivateState(alice.clone()))
                .await
                .unwrap()
                .0
                .pending
                .is_empty()
        );
        // The chat card followed: the memo carries the outcome as an event.
        let events = alice.db.events(&memo.id, 0)?;
        let last = events
            .iter()
            .rev()
            .find(|e| e.kind == "wallet_request")
            .unwrap();
        assert!(last.text.contains("\"sent\""));
        // A stale pending request is no longer offered.
        {
            let conn = alice.db.0.lock().unwrap();
            let old = create_request(&conn, ALICE, &memo.id, &coin, &to, "1")?;
            conn.execute(
                "UPDATE wallet_requests SET created_at=?2 WHERE id=?1",
                params![old.id, "2020-01-01T00:00:00+00:00"],
            )?;
        }
        assert!(
            get(PrivateState(alice.clone()))
                .await
                .unwrap()
                .0
                .pending
                .is_empty()
        );
        let _ = crate::chat::delete_all(PrivateState(alice.clone()))
            .await
            .unwrap();
        let conn = root.db.0.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM wallet_requests WHERE owner=?1",
            [ALICE],
            |r| r.get(0),
        )?;
        assert_eq!(n, 0);
        Ok(())
    }

    /// A database from before `wallet_requests.hash` and `wallet_limits`
    /// existed: opening it adds the column and the table, and the old row reads.
    #[test]
    fn an_older_wallet_schema_is_migrated_on_open() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("causewaybay.sqlite3");
        {
            let conn = Connection::open(&path)?;
            conn.execute_batch(
                "CREATE TABLE wallet_requests(id TEXT PRIMARY KEY,owner TEXT NOT NULL,memo_id TEXT NOT NULL,coin TEXT NOT NULL,to_address TEXT NOT NULL,amount TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
                 INSERT INTO wallet_requests VALUES('r1','0xabc','m1','tcro-cronos-testnet','0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266','5','sent','2026-01-01T00:00:00+00:00','2026-01-01T00:00:00+00:00');",
            )?;
        }
        let db = db::Database::open(&path)?;
        let conn = db.0.lock().unwrap();
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(wallet_requests)")?
            .query_map([], |r| r.get(1))?
            .collect::<rusqlite::Result<_>>()?;
        assert!(columns.iter().any(|c| c == "hash"), "{columns:?}");
        for added in ["from_before", "to_before", "from_after", "to_after"] {
            assert!(
                columns.iter().any(|c| c == added),
                "{added} missing: {columns:?}"
            );
        }
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'wallet_%' ORDER BY name")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        assert!(tables.iter().any(|t| t == "wallet_limits"), "{tables:?}");
        let found = requests(&conn, "0xabc", None)?;
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].hash.as_str(), found[0].symbol.as_str()),
            ("", "TCRO")
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_cardano_relay_reads_only_public_addresses_on_cardano() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?
            .for_owner(ALICE);
        let read = |network: &str, path: &str, address: &str| {
            cardano(
                PrivateState(s.clone()),
                Json(CardanoRead {
                    network: network.into(),
                    path: path.into(),
                    address: address.into(),
                }),
            )
        };
        let testnet = "addr_test1qq8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7sh927ysx5sftuw0dlft05dz3c7revpf7jx0xnlcjz3g69mqkt5dmn";
        assert!(
            error(read("solana-mainnet", "address_info", testnet).await).contains("Only Cardano")
        );
        assert!(error(read("cardano-preprod", "tx_submit", testnet).await).contains("Unsupported"));
        // A mainnet address on a test network, and anything that is not bech32, is refused.
        assert!(
            error(
                read(
                    "cardano-preprod",
                    "address_info",
                    &testnet.replace("addr_test1", "addr1")
                )
                .await
            )
            .contains("not a Cardano Preprod")
        );
        assert!(
            error(read("cardano-mainnet", "address_info", testnet).await)
                .contains("not a Cardano Mainnet")
        );
        assert!(
            error(read("cardano-preprod", "address_info", "addr_test1<script>").await)
                .contains("not a")
        );
        // Nothing but these three fields is accepted.
        assert!(serde_json::from_value::<CardanoRead>(json!({"network":"cardano-preprod","path":"address_info","address":testnet,"key":"x"})).is_err());
        Ok(())
    }
}
