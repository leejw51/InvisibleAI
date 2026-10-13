//! Wallet choices per account: the active coin, recent picks, ERC-20s the user
//! added and the receive address on each network, so a later sign-in opens the
//! wallet as it was left. Only public information is kept: keys and phrases
//! stay in the browser and never reach the backend.
use crate::{Api, ApiError, PrivateState, db};
use anyhow::Result;
use axum::{Json, extract::Path, http::StatusCode};
use causewaybay_shared::evm::{self, CustomToken, MAX_TOKENS};
use causewaybay_shared::registry::{self, Chain, MAX_RECENT, Net, WalletSettings};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use std::collections::BTreeMap;

pub fn init(conn: &Connection) -> Result<()> {
    crate::schema::ensure(
        conn,
        "CREATE TABLE IF NOT EXISTS wallet_state(owner TEXT PRIMARY KEY,active TEXT NOT NULL,recent TEXT NOT NULL DEFAULT '[]',updated_at TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS wallet_tokens(owner TEXT NOT NULL,network TEXT NOT NULL,address TEXT NOT NULL,symbol TEXT NOT NULL,name TEXT NOT NULL,decimals INTEGER NOT NULL,added_at TEXT NOT NULL,PRIMARY KEY(owner,network,address));
         CREATE TABLE IF NOT EXISTS wallet_addresses(owner TEXT NOT NULL,network TEXT NOT NULL,address TEXT NOT NULL,PRIMARY KEY(owner,network));",
    )
}

/// Erasing an account leaves no wallet choice, token or address behind,
/// including rows an earlier build of this page kept.
pub fn forget_owner(conn: &Connection, owner: &str) -> Result<()> {
    for table in [
        "wallet_state",
        "wallet_tokens",
        "wallet_addresses",
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
        .prepare("SELECT network,address FROM wallet_addresses WHERE owner=?1")?
        .query_map([owner], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
    {
        let (network, address) = row?;
        settings.addresses.insert(network, address);
    }
    // A coin that is no longer offered (a removed token) reads as the default.
    settings.active = settings.active_coin().key;
    Ok(settings)
}

/// Make `key` the active coin and the newest recent pick.
fn activate(conn: &Connection, owner: &str, key: &str) -> Result<()> {
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
    if let Some(key) = v.active {
        let offered = read(&tx, &owner)?.all_coins().iter().any(|c| c.key == key);
        if !offered {
            return Err(bad("Add this token before choosing it."));
        }
        activate(&tx, &owner, &key)?;
    }
    tx.commit()?;
    Ok(Json(read(&conn, &owner)?))
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
        assert_eq!(fresh.active, "usdc-cronos-mainnet");

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
        assert_eq!(s.active, "usdc-cronos-mainnet");
        assert!(!s.recent.contains(&key));

        let _ = crate::chat::delete_all(PrivateState(alice.clone()))
            .await
            .unwrap();
        let conn = root.db.0.lock().unwrap();
        for table in ["wallet_state", "wallet_tokens", "wallet_addresses"] {
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
