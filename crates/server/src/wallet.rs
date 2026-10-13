//! Wallet choices per account: the network last opened, the asset picked on
//! each network (CRO, USDC or a saved ERC-20) and the tokens saved there, so a
//! later sign-in opens the wallet as it was left. Only public information is
//! kept: keys and phrases never reach the backend.
use crate::{Api, ApiError, PrivateState, db};
use anyhow::Result;
use axum::{Json, extract::Path, http::StatusCode};
use causewaybay_shared::evm::{
    self, CustomToken, MAX_TOKENS, NATIVE_KEY, Network, USDC_KEY, WalletSettings,
};
use rusqlite::{Connection, params};
use serde::Deserialize;

pub fn init(conn: &Connection) -> Result<()> {
    crate::schema::ensure(
        conn,
        "CREATE TABLE IF NOT EXISTS wallet_settings(owner TEXT PRIMARY KEY,network TEXT NOT NULL,updated_at TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS wallet_selections(owner TEXT NOT NULL,network TEXT NOT NULL,asset TEXT NOT NULL,PRIMARY KEY(owner,network));
         CREATE TABLE IF NOT EXISTS wallet_tokens(owner TEXT NOT NULL,network TEXT NOT NULL,address TEXT NOT NULL,symbol TEXT NOT NULL,name TEXT NOT NULL,decimals INTEGER NOT NULL,added_at TEXT NOT NULL,PRIMARY KEY(owner,network,address));",
    )
}

/// Erasing an account leaves no wallet choice or saved token behind.
pub fn forget_owner(conn: &Connection, owner: &str) -> Result<()> {
    for table in ["wallet_settings", "wallet_selections", "wallet_tokens"] {
        conn.execute(&format!("DELETE FROM {table} WHERE owner=?1"), [owner])?;
    }
    Ok(())
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}
fn network(key: &str) -> Result<Network, ApiError> {
    evm::network(key).ok_or_else(|| bad("Choose Cronos Mainnet or Cronos Testnet."))
}

pub fn read(conn: &Connection, owner: &str) -> Result<WalletSettings> {
    let mut settings = WalletSettings::default();
    if let Some(network) = conn
        .prepare("SELECT network FROM wallet_settings WHERE owner=?1")?
        .query_map([owner], |r| r.get::<_, String>(0))?
        .next()
        .transpose()?
        .filter(|n| evm::network(n).is_some())
    {
        settings.network = network;
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
        .prepare("SELECT network,asset FROM wallet_selections WHERE owner=?1")?
        .query_map([owner], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
    {
        let (network, asset) = row?;
        settings.selected.insert(network, asset);
    }
    Ok(settings)
}

/// `native`, `usdc` or the address of a token saved on that network.
fn valid_asset(conn: &Connection, owner: &str, network: &Network, asset: &str) -> Api<String> {
    let asset = asset.trim().to_ascii_lowercase();
    if asset == NATIVE_KEY || asset == USDC_KEY {
        return Ok(asset);
    }
    let saved: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wallet_tokens WHERE owner=?1 AND network=?2 AND lower(address)=?3)",
        params![owner, network.key, asset],
        |r| r.get(0),
    )?;
    if !saved {
        return Err(bad("Add this token before choosing it."));
    }
    Ok(asset)
}
fn select(conn: &Connection, owner: &str, network: &Network, asset: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO wallet_selections(owner,network,asset) VALUES(?1,?2,?3) ON CONFLICT(owner,network) DO UPDATE SET asset=excluded.asset",
        params![owner, network.key, asset],
    )?;
    Ok(())
}
fn open_network(conn: &Connection, owner: &str, network: &Network) -> Result<()> {
    conn.execute(
        "INSERT INTO wallet_settings(owner,network,updated_at) VALUES(?1,?2,?3) ON CONFLICT(owner) DO UPDATE SET network=excluded.network,updated_at=excluded.updated_at",
        params![owner, network.key, db::now()],
    )?;
    Ok(())
}

pub async fn get(PrivateState(s): PrivateState) -> Api<Json<WalletSettings>> {
    let conn = s.db.0.lock().unwrap();
    Ok(Json(read(&conn, s.db.owner())?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    network: String,
    #[serde(default)]
    asset: Option<String>,
}
/// Open a network and, optionally, pick the asset shown on it.
pub async fn update(
    PrivateState(s): PrivateState,
    Json(v): Json<Choice>,
) -> Api<Json<WalletSettings>> {
    let network = network(&v.network)?;
    let mut conn = s.db.0.lock().unwrap();
    s.db.writable(&conn)?;
    let owner = s.db.owner();
    let tx = conn.transaction()?;
    open_network(&tx, owner, &network)?;
    if let Some(asset) = v.asset {
        let asset = valid_asset(&tx, owner, &network, &asset)?;
        select(&tx, owner, &network, &asset)?;
    }
    tx.commit()?;
    Ok(Json(read(&conn, owner)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewToken {
    network: String,
    token: CustomToken,
}
/// Save an ERC-20 the browser read from its contract, and choose it.
pub async fn add_token(
    PrivateState(s): PrivateState,
    Json(v): Json<NewToken>,
) -> Api<Json<WalletSettings>> {
    let network = network(&v.network)?;
    let token = evm::validate_token(&network, &v.token).map_err(bad)?;
    let mut conn = s.db.0.lock().unwrap();
    s.db.writable(&conn)?;
    let owner = s.db.owner();
    let tx = conn.transaction()?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM wallet_tokens WHERE owner=?1 AND network=?2 AND lower(address)!=?3",
        params![owner, network.key, token.address.to_ascii_lowercase()],
        |r| r.get(0),
    )?;
    if count as usize >= MAX_TOKENS {
        return Err(bad(format!(
            "You can save up to {MAX_TOKENS} tokens per network. Remove one first."
        )));
    }
    tx.execute(
        "INSERT INTO wallet_tokens(owner,network,address,symbol,name,decimals,added_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(owner,network,address) DO UPDATE SET symbol=excluded.symbol,name=excluded.name,decimals=excluded.decimals",
        params![owner, network.key, token.address, token.symbol, token.name, token.decimals, db::now()],
    )?;
    open_network(&tx, owner, &network)?;
    select(&tx, owner, &network, &token.address.to_ascii_lowercase())?;
    tx.commit()?;
    Ok(Json(read(&conn, owner)?))
}

/// Forget a saved token; if it was chosen, USDC is shown instead.
pub async fn remove_token(
    PrivateState(s): PrivateState,
    Path((network_key, address)): Path<(String, String)>,
) -> Api<Json<WalletSettings>> {
    let network = network(&network_key)?;
    let address = checksum_of(&address)?.to_ascii_lowercase();
    let mut conn = s.db.0.lock().unwrap();
    s.db.writable(&conn)?;
    let owner = s.db.owner();
    let tx = conn.transaction()?;
    let removed = tx.execute(
        "DELETE FROM wallet_tokens WHERE owner=?1 AND network=?2 AND lower(address)=?3",
        params![owner, network.key, address],
    )?;
    if removed == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "Token not found".into()));
    }
    tx.execute(
        "DELETE FROM wallet_selections WHERE owner=?1 AND network=?2 AND asset=?3",
        params![owner, network.key, address],
    )?;
    tx.commit()?;
    Ok(Json(read(&conn, owner)?))
}
fn checksum_of(address: &str) -> Api<String> {
    evm::parse_address(address)
        .map(|a| evm::checksum(&a))
        .map_err(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use serde_json::json;

    const WCRO: &str = "0x5C7F8A570d578ED84E63fdFA7b1eE72dEae1AE23";
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

    #[tokio::test]
    async fn choices_and_tokens_persist_per_owner_and_are_erased_with_the_account() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root =
            AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
        let alice = root.for_owner("alice");
        let bob = root.for_owner("bob");
        // Nothing chosen yet: testnet, USDC.
        let Json(fresh) = get(PrivateState(alice.clone())).await.unwrap();
        assert_eq!(fresh, WalletSettings::default());

        let Json(s) = update(
            PrivateState(alice.clone()),
            Json(serde_json::from_value(
                json!({"network":"cronos-mainnet","asset":"native"}),
            )?),
        )
        .await
        .unwrap();
        assert_eq!(s.network, "cronos-mainnet");
        assert_eq!(s.asset_on(&evm::CRONOS_MAINNET), evm::Asset::Native);

        // A token is checksummed, cleaned, saved on its network only, and chosen.
        let Json(s) = add_token(
            PrivateState(alice.clone()),
            Json(NewToken {
                network: "cronos-mainnet".into(),
                token: CustomToken {
                    address: WCRO.to_ascii_lowercase(),
                    symbol: "WCRO\u{202E}".into(),
                    name: "Wrapped CRO".into(),
                    decimals: 18,
                },
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            s.tokens_on(&evm::CRONOS_MAINNET),
            [CustomToken {
                name: "Wrapped CRO".into(),
                ..token(WCRO, "WCRO")
            }]
        );
        assert!(s.tokens_on(&evm::CRONOS_TESTNET).is_empty());
        assert!(
            matches!(s.asset_on(&evm::CRONOS_MAINNET), evm::Asset::Erc20(t) if t.address == WCRO)
        );

        // Read back as a new sign-in would; bob sees none of it.
        let Json(again) = get(PrivateState(alice.clone())).await.unwrap();
        assert_eq!(again, s);
        let Json(other) = get(PrivateState(bob.clone())).await.unwrap();
        assert_eq!(other, WalletSettings::default());
        assert_eq!(
            error(
                update(
                    PrivateState(bob.clone()),
                    Json(Choice {
                        network: "cronos-mainnet".into(),
                        asset: Some(WCRO.into())
                    })
                )
                .await
            ),
            "Add this token before choosing it."
        );

        // Removing the chosen token falls back to USDC.
        let Json(s) = remove_token(
            PrivateState(alice.clone()),
            Path(("cronos-mainnet".into(), WCRO.into())),
        )
        .await
        .unwrap();
        assert!(s.tokens_on(&evm::CRONOS_MAINNET).is_empty());
        assert_eq!(s.asset_on(&evm::CRONOS_MAINNET), evm::Asset::Usdc);

        // Erasure forgets every row.
        let _ = add_token(
            PrivateState(alice.clone()),
            Json(NewToken {
                network: "cronos-testnet".into(),
                token: token(WCRO, "WCRO"),
            }),
        )
        .await
        .unwrap();
        let _ = crate::chat::delete_all(PrivateState(alice.clone()))
            .await
            .unwrap();
        let conn = root.db.0.lock().unwrap();
        for table in ["wallet_settings", "wallet_selections", "wallet_tokens"] {
            let n: i64 = conn.query_row(
                &format!("SELECT count(*) FROM {table} WHERE owner='alice'"),
                [],
                |r| r.get(0),
            )?;
            assert_eq!(n, 0, "{table}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn only_valid_networks_assets_and_tokens_are_kept() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?
            .for_owner("alice");
        let choose = |network: &str, asset: Option<&str>| Choice {
            network: network.into(),
            asset: asset.map(Into::into),
        };
        assert!(
            error(update(PrivateState(s.clone()), Json(choose("ethereum", None))).await)
                .contains("Cronos")
        );
        assert!(
            error(
                update(
                    PrivateState(s.clone()),
                    Json(choose("cronos-testnet", Some("btc")))
                )
                .await
            )
            .contains("Add this token")
        );
        // Only the two fields are accepted; a key in the payload is refused outright.
        assert!(
            serde_json::from_value::<Choice>(
                json!({"network":"cronos-testnet","private_key":"0x01"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<NewToken>(
                json!({"network":"cronos-testnet","token":token(WCRO,"W"),"mnemonic":"x"})
            )
            .is_err()
        );
        // Nor can one ride inside the token.
        assert!(
            serde_json::from_value::<NewToken>(json!({"network":"cronos-testnet","token":{
                "address":WCRO,"symbol":"W","name":"","decimals":18,"private_key":"0x01"
            }}))
            .is_err()
        );
        let add = |network: &str, t: CustomToken| {
            add_token(
                PrivateState(s.clone()),
                Json(NewToken {
                    network: network.into(),
                    token: t,
                }),
            )
        };
        assert!(
            error(
                add(
                    "cronos-mainnet",
                    token(evm::CRONOS_MAINNET.usdc.address, "USDC")
                )
                .await
            )
            .contains("Choose USDC")
        );
        assert!(error(add("cronos-mainnet", token("0x1234", "X")).await).contains("40 hex"));
        assert!(
            error(
                add(
                    "cronos-mainnet",
                    CustomToken {
                        decimals: 77,
                        ..token(WCRO, "X")
                    }
                )
                .await
            )
            .contains("decimals")
        );
        // Saving the same token twice keeps one row; the cap counts others only.
        let _ = add("cronos-testnet", token(WCRO, "WCRO")).await.unwrap();
        let Json(s2) = add("cronos-testnet", token(&WCRO.to_ascii_lowercase(), "WCRO"))
            .await
            .unwrap();
        assert_eq!(s2.tokens_on(&evm::CRONOS_TESTNET).len(), 1);
        for i in 1..MAX_TOKENS {
            let _ = add("cronos-testnet", token(&format!("0x{i:040x}"), "T"))
                .await
                .unwrap();
        }
        assert!(
            error(add("cronos-testnet", token(&format!("0x{:040x}", 999), "T")).await)
                .contains("up to 50")
        );
        assert_eq!(
            error(
                remove_token(
                    PrivateState(s.clone()),
                    Path(("cronos-mainnet".into(), WCRO.into()))
                )
                .await
            ),
            "Token not found"
        );
        Ok(())
    }
}
