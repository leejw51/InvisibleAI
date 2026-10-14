//! Every network and coin the wallet knows, after the Causewaybay Wallet's
//! tables (SPEC §2, §2.4), plus Arbitrum. A coin is a token *on a network*:
//! USDC on Cronos and USDC on Solana share a name and nothing else, so a row is
//! the pair and is named as the pair. One search rule (SPEC §2.1) finds rows.
use crate::evm::{self, Asset, CustomToken};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chain {
    Evm,
    Solana,
    Cardano,
    Midnight,
    Ecash,
}
impl Chain {
    pub fn key(self) -> &'static str {
        match self {
            Chain::Evm => "evm",
            Chain::Solana => "solana",
            Chain::Cardano => "cardano",
            Chain::Midnight => "midnight",
            Chain::Ecash => "ecash",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Chain::Evm => "EVM",
            Chain::Solana => "Solana",
            Chain::Cardano => "Cardano",
            Chain::Midnight => "Midnight",
            Chain::Ecash => "eCash",
        }
    }
}

/// One network: its chain, its own coin, where to read it and where to look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Net {
    pub key: &'static str,
    pub name: &'static str,
    pub chain: Chain,
    /// The coin that pays every fee here.
    pub symbol: &'static str,
    pub coin_name: &'static str,
    pub decimals: u8,
    pub testnet: bool,
    /// What the name does not say (SPEC §2.1).
    pub tags: &'static [&'static str],
    pub endpoint: &'static str,
    pub explorer: &'static str,
    pub faucet: Option<&'static str>,
    /// Set for EVM networks, which this wallet can also send on.
    pub evm: Option<evm::Network>,
}
impl Net {
    /// Explorer link for an address; Solana keeps its cluster in a query string.
    pub fn address_url(&self, address: &str) -> String {
        self.link("address", address)
    }
    pub fn tx_url(&self, hash: &str) -> String {
        self.link("tx", hash)
    }
    fn link(&self, kind: &str, value: &str) -> String {
        match self.explorer.split_once('?') {
            Some((base, query)) => format!("{}/{kind}/{value}?{query}", base.trim_end_matches('/')),
            None => format!("{}/{kind}/{value}", self.explorer.trim_end_matches('/')),
        }
    }
}

// One row per call keeps the table readable as a table.
#[allow(clippy::too_many_arguments)]
const fn net(
    key: &'static str,
    name: &'static str,
    chain: Chain,
    (symbol, coin_name, decimals): (&'static str, &'static str, u8),
    testnet: bool,
    tags: &'static [&'static str],
    (endpoint, explorer): (&'static str, &'static str),
    faucet: Option<&'static str>,
    evm: Option<evm::Network>,
) -> Net {
    Net {
        key,
        name,
        chain,
        symbol,
        coin_name,
        decimals,
        testnet,
        tags,
        endpoint,
        explorer,
        faucet,
        evm,
    }
}

// Solana mainnet is read through PublicNode: api.mainnet-beta.solana.com
// answers 403 to any request carrying a browser Origin (checked 2026-10-13),
// and the page checks the genesis hash, so a wrong cluster is refused.
pub const NETS: [Net; 14] = [
    net(
        "cronos-mainnet",
        "Cronos Mainnet",
        Chain::Evm,
        ("CRO", "Cronos", 18),
        false,
        &["evm", "smart-contracts", "erc20"],
        (evm::CRONOS_MAINNET.rpc, evm::CRONOS_MAINNET.explorer),
        None,
        Some(evm::CRONOS_MAINNET),
    ),
    net(
        "cronos-testnet",
        "Cronos Testnet",
        Chain::Evm,
        ("TCRO", "Cronos test coin", 18),
        true,
        &["evm", "testnet", "faucet", "smart-contracts", "erc20"],
        (evm::CRONOS_TESTNET.rpc, evm::CRONOS_TESTNET.explorer),
        Some("https://faucet.cronos.com/"),
        Some(evm::CRONOS_TESTNET),
    ),
    net(
        "arbitrum-one",
        "Arbitrum One",
        Chain::Evm,
        ("ETH", "Ether on Arbitrum", 18),
        false,
        &[
            "evm",
            "mainnet",
            "l2",
            "rollup",
            "ethereum",
            "smart-contracts",
            "erc20",
        ],
        (evm::ARBITRUM_ONE.rpc, evm::ARBITRUM_ONE.explorer),
        None,
        Some(evm::ARBITRUM_ONE),
    ),
    net(
        "arbitrum-sepolia",
        "Arbitrum Sepolia",
        Chain::Evm,
        ("ETH", "Sepolia Ether on Arbitrum", 18),
        true,
        &[
            "evm",
            "testnet",
            "faucet",
            "l2",
            "rollup",
            "ethereum",
            "smart-contracts",
            "erc20",
        ],
        (evm::ARBITRUM_SEPOLIA.rpc, evm::ARBITRUM_SEPOLIA.explorer),
        Some("https://faucet.quicknode.com/arbitrum/sepolia"),
        Some(evm::ARBITRUM_SEPOLIA),
    ),
    net(
        "solana-mainnet",
        "Solana Mainnet Beta",
        Chain::Solana,
        ("SOL", "Solana", 9),
        false,
        &["svm", "spl"],
        (
            "https://solana-rpc.publicnode.com",
            "https://explorer.solana.com",
        ),
        None,
        None,
    ),
    net(
        "solana-devnet",
        "Solana Devnet",
        Chain::Solana,
        ("SOL", "Devnet SOL", 9),
        true,
        &["svm", "testnet", "faucet", "spl"],
        (
            "https://api.devnet.solana.com",
            "https://explorer.solana.com/?cluster=devnet",
        ),
        Some("https://faucet.solana.com/"),
        None,
    ),
    net(
        "solana-testnet",
        "Solana Testnet",
        Chain::Solana,
        ("SOL", "Testnet SOL", 9),
        true,
        &["svm", "testnet", "faucet", "spl"],
        (
            "https://api.testnet.solana.com",
            "https://explorer.solana.com/?cluster=testnet",
        ),
        Some("https://faucet.solana.com/"),
        None,
    ),
    net(
        "cardano-mainnet",
        "Cardano Mainnet",
        Chain::Cardano,
        ("ADA", "Cardano ada", 6),
        false,
        &["utxo", "native-assets"],
        ("https://api.koios.rest/api/v1", "https://cardanoscan.io"),
        None,
        None,
    ),
    net(
        "cardano-preprod",
        "Cardano Preprod",
        Chain::Cardano,
        ("tADA", "Preprod test ada", 6),
        true,
        &["utxo", "testnet", "faucet", "native-assets"],
        (
            "https://preprod.koios.rest/api/v1",
            "https://preprod.cardanoscan.io",
        ),
        Some("https://docs.cardano.org/cardano-testnets/tools/faucet"),
        None,
    ),
    net(
        "cardano-preview",
        "Cardano Preview",
        Chain::Cardano,
        ("tADA", "Preview test ada", 6),
        true,
        &["utxo", "testnet", "faucet", "native-assets"],
        (
            "https://preview.koios.rest/api/v1",
            "https://preview.cardanoscan.io",
        ),
        Some("https://docs.cardano.org/cardano-testnets/tools/faucet"),
        None,
    ),
    net(
        "midnight-preview",
        "Midnight Preview",
        Chain::Midnight,
        ("NIGHT", "Midnight NIGHT", 6),
        true,
        &["privacy", "testnet", "faucet", "shielded", "zk"],
        (
            "https://indexer.preview.midnight.network/api/v4/graphql",
            "https://preview.midnightexplorer.com",
        ),
        Some("https://midnight-tmnight-preview.nethermind.dev/"),
        None,
    ),
    net(
        "midnight-devnet",
        "Midnight Devnet",
        Chain::Midnight,
        ("NIGHT", "Midnight NIGHT (devnet)", 6),
        true,
        &["privacy", "testnet", "faucet", "shielded", "zk"],
        (
            "https://indexer.devnet.midnight.network/api/v4/graphql",
            "https://devnet.midnightexplorer.com",
        ),
        Some("https://midnight.network/test-faucet"),
        None,
    ),
    net(
        "ecash-mainnet",
        "eCash Mainnet",
        Chain::Ecash,
        ("XEC", "eCash", 2),
        false,
        &["utxo", "bitcoin-fork"],
        ("https://chronik.e.cash", "https://explorer.e.cash"),
        None,
        None,
    ),
    net(
        "ecash-testnet",
        "eCash Testnet",
        Chain::Ecash,
        ("tXEC", "eCash test coin", 2),
        true,
        &["utxo", "testnet", "faucet", "bitcoin-fork"],
        (
            "https://chronik-testnet.fabien.cash",
            "https://texplorer.e.cash",
        ),
        Some("https://texplorer.e.cash/testnet-faucet"),
        None,
    ),
];
pub fn net_of(key: &str) -> Option<&'static Net> {
    NETS.iter().find(|n| n.key == key)
}

/// How a coin is held, and so how it is read and moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Standard {
    Native,
    Erc20,
    SplToken,
    CardanoAsset,
}

/// A registry token: (key, name, symbol, network, standard, decimals, on-chain id, tags).
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Standard,
    u8,
    &'static str,
    &'static [&'static str],
);
// Every id was read off the chain it names: the EVM contracts answered
// `symbol()` and `decimals()`, the mints and assets were read by the
// Causewaybay Wallet (SPEC §2.4), and the Arbitrum rows on 2026-10-13.
const TOKENS: [Row; 13] = [
    (
        "usdc-cronos-mainnet",
        "USDC Cronos Mainnet",
        "USDC",
        "cronos-mainnet",
        Standard::Erc20,
        6,
        "0xc21223249CA28397B4B6541dfFaEcC539BfF0c59",
        &["stablecoin", "usd", "circle"],
    ),
    (
        "usdt-cronos-mainnet",
        "USDT Cronos Mainnet",
        "USDT",
        "cronos-mainnet",
        Standard::Erc20,
        6,
        "0x66e428c3f67a68878562e79A0234c1F83c208770",
        &["stablecoin", "usd", "tether"],
    ),
    (
        "dai-cronos-mainnet",
        "DAI Cronos Mainnet",
        "DAI",
        "cronos-mainnet",
        Standard::Erc20,
        18,
        "0xF2001B145b43032AAF5Ee2884e456CCd805F677D",
        &["stablecoin", "usd", "maker"],
    ),
    (
        "devusdc.e-cronos-testnet",
        "devUSDC.e Cronos Testnet",
        "devUSDC.e",
        "cronos-testnet",
        Standard::Erc20,
        6,
        "0xc01efAaF7C5C61bEbFAeb358E1161b537b8bC0e0",
        &["stablecoin", "usd", "usdc", "faucet"],
    ),
    (
        "usdc-arbitrum-one",
        "USDC Arbitrum One",
        "USDC",
        "arbitrum-one",
        Standard::Erc20,
        6,
        "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
        &["stablecoin", "usd", "circle"],
    ),
    (
        "usdc-arbitrum-sepolia",
        "USDC Arbitrum Sepolia",
        "USDC",
        "arbitrum-sepolia",
        Standard::Erc20,
        6,
        "0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d",
        &["stablecoin", "usd", "circle", "faucet"],
    ),
    (
        "usdc-solana-mainnet",
        "USDC Solana Mainnet",
        "USDC",
        "solana-mainnet",
        Standard::SplToken,
        6,
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        &["stablecoin", "usd", "circle"],
    ),
    (
        "usdt-solana-mainnet",
        "USDT Solana Mainnet",
        "USDT",
        "solana-mainnet",
        Standard::SplToken,
        6,
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        &["stablecoin", "usd", "tether"],
    ),
    (
        "usdc-solana-devnet",
        "USDC Solana Devnet",
        "USDC",
        "solana-devnet",
        Standard::SplToken,
        6,
        "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
        &["stablecoin", "usd", "circle", "faucet"],
    ),
    (
        "usdc-cardano-mainnet",
        "USDC Cardano Mainnet",
        "USDC",
        "cardano-mainnet",
        Standard::CardanoAsset,
        8,
        "25c5de5f5b286073c593edfd77b48abc7a48e5a4f3d4cd9d428ff93555534443",
        &["stablecoin", "usd", "bridged", "wanchain"],
    ),
    (
        "usdm-cardano-mainnet",
        "USDM Cardano Mainnet",
        "USDM",
        "cardano-mainnet",
        Standard::CardanoAsset,
        6,
        "c48cbb3d5e57ed56e276bc45f99ab39abe94e6cd7ac39fb402da47ad0014df105553444d",
        &["stablecoin", "usd", "moneta"],
    ),
    (
        "djed-cardano-mainnet",
        "DJED Cardano Mainnet",
        "DJED",
        "cardano-mainnet",
        Standard::CardanoAsset,
        6,
        "8db269c3ec630e06ae29f74bc39edd1f87c819f1056206e879a1cd61446a65644d6963726f555344",
        &["stablecoin", "usd", "overcollateralised"],
    ),
    (
        "iusd-cardano-mainnet",
        "iUSD Cardano Mainnet",
        "iUSD",
        "cardano-mainnet",
        Standard::CardanoAsset,
        6,
        "f66d78b4a3cb3d37afa0ec36461e51ecbde00f26c8f0a68f94b6988069555344",
        &["stablecoin", "usd", "indigo"],
    ),
];

/// Anything the wallet can show: a network's own coin, a registry token, or an
/// ERC-20 the user added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coin {
    pub key: String,
    pub name: String,
    pub symbol: String,
    pub network: String,
    pub standard: Standard,
    pub decimals: u8,
    /// Contract, mint or policy+asset; empty for a network's own coin.
    pub id: String,
    pub tags: Vec<String>,
    /// Added by the user rather than shipped in the registry.
    pub custom: bool,
}
impl Coin {
    pub fn net(&self) -> &'static Net {
        net_of(&self.network).expect("coins are only built for known networks")
    }
    /// Sending is EVM-only for now; everything else is view and receive.
    pub fn can_send(&self) -> bool {
        self.net().evm.is_some() && matches!(self.standard, Standard::Native | Standard::Erc20)
    }
    /// The EVM network and asset this coin sends as, when it is on EVM.
    pub fn evm_asset(&self) -> Option<(evm::Network, Asset)> {
        let network = self.net().evm?;
        let asset = match self.standard {
            Standard::Native => Asset::Native,
            Standard::Erc20 if self.id.eq_ignore_ascii_case(network.usdc.address) => Asset::Usdc,
            Standard::Erc20 => Asset::Erc20(CustomToken {
                address: self.id.clone(),
                symbol: self.symbol.clone(),
                name: self.name.clone(),
                decimals: self.decimals,
            }),
            _ => return None,
        };
        Some((network, asset))
    }
}
fn native(n: &Net) -> Coin {
    Coin {
        key: native_key(n),
        name: n.coin_name.into(),
        symbol: n.symbol.into(),
        network: n.key.into(),
        standard: Standard::Native,
        decimals: n.decimals,
        id: String::new(),
        tags: vec!["native".into(), "fees".into()],
        custom: false,
    }
}
pub fn native_key(n: &Net) -> String {
    format!("{}-{}", n.symbol.to_ascii_lowercase(), n.key)
}
/// The key a user-added ERC-20 is stored and chosen by.
pub fn custom_key(network: &str, address: &str) -> String {
    format!("erc20-{network}-{}", address.to_ascii_lowercase())
}
pub fn custom_coin(network: &str, token: &CustomToken) -> Coin {
    Coin {
        key: custom_key(network, &token.address),
        name: token.name.clone(),
        symbol: token.symbol.clone(),
        network: network.into(),
        standard: Standard::Erc20,
        decimals: token.decimals,
        id: token.address.clone(),
        tags: vec!["custom".into()],
        custom: true,
    }
}
/// Every built-in coin: each network's own coin first, then the tokens.
pub fn coins() -> Vec<Coin> {
    let mut all: Vec<Coin> = NETS.iter().map(native).collect();
    all.extend(TOKENS.iter().map(
        |(key, name, symbol, network, standard, decimals, id, tags)| Coin {
            key: (*key).into(),
            name: (*name).into(),
            symbol: (*symbol).into(),
            network: (*network).into(),
            standard: *standard,
            decimals: *decimals,
            id: (*id).into(),
            tags: tags.iter().map(|t| (*t).into()).collect(),
            custom: false,
        },
    ));
    all
}
pub fn coin(key: &str) -> Option<Coin> {
    coins().into_iter().find(|c| c.key == key)
}

/// The two main buttons: Cronos Mainnet's own coin, and USDC on Cronos Mainnet.
pub const MAIN_COINS: [&str; 2] = ["cro-cronos-mainnet", "usdc-cronos-mainnet"];
/// A new account starts where nothing real is at stake: Cronos testnet.
pub const DEFAULT_COIN: &str = "tcro-cronos-testnet";

/// Case, `-`, `_` and spaces are one difference (SPEC §2.1).
fn fold(text: &str) -> String {
    text.to_lowercase().replace(['_', ' '], "-")
}
/// SPEC §2.1: empty matches all; every whitespace- or comma-separated term
/// must be a substring of some field; no ranking, so rows keep their order.
pub fn search<'a>(query: &str, coins: &'a [Coin]) -> Vec<&'a Coin> {
    let terms: Vec<String> = query
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(fold)
        .collect();
    coins
        .iter()
        .filter(|c| {
            let n = c.net();
            let mut fields = vec![
                fold(&c.key),
                fold(&c.name),
                fold(&c.symbol),
                fold(&c.network),
                fold(n.name),
                n.chain.key().into(),
                fold(n.chain.name()),
                c.id.to_lowercase(),
            ];
            fields.extend(c.tags.iter().map(|t| fold(t)));
            fields.extend(n.tags.iter().map(|t| fold(t)));
            terms
                .iter()
                .all(|t| fields.iter().any(|f| f.contains(t.as_str())))
        })
        .collect()
}

/// An amount as typed, with or without its denomination: "25", "25 TCRO",
/// "25tcro". A different denomination is refused, never converted: typing
/// "25 USDC" while sending CRO is a mistake to stop, not a request to honour.
pub fn amount_in(input: &str, symbol: &str) -> Result<String, String> {
    let text = input.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '+'))
        .unwrap_or(text.len());
    let (number, denom) = (text[..split].trim(), text[split..].trim());
    if !denom.is_empty() && !denom.eq_ignore_ascii_case(symbol) {
        return Err(format!(
            "That amount is in {denom}, but this sends {symbol}."
        ));
    }
    Ok(number.to_string())
}
/// Above 20 CRO, above 1 US dollar, or any amount of a coin whose dollar value
/// the wallet does not know: the transfer is confirmed twice, the second time
/// by typing it.
pub fn needs_double_confirm(coin: &Coin, amount: evm::U256) -> bool {
    let whole =
        |n: u64| evm::U256::from(n).saturating_mul_u64(10u64.pow(coin.decimals.min(19) as u32));
    let cronos_coin =
        coin.standard == Standard::Native && matches!(coin.symbol.as_str(), "CRO" | "TCRO");
    let usd = coin.tags.iter().any(|t| t == "usd");
    if coin.decimals > 19 {
        return true;
    }
    if cronos_coin {
        amount > whole(20)
    } else if usd {
        amount > whole(1)
    } else {
        true
    }
}
/// What the user types to confirm: the amount and the denomination.
pub fn confirmation_text(amount: &str, symbol: &str) -> String {
    format!("{amount} {symbol}")
}
/// Whether `typed` names exactly this amount of this coin. Case and spacing
/// are forgiven; the number and the denomination are not.
pub fn confirms(typed: &str, amount: evm::U256, coin: &Coin) -> bool {
    let words: Vec<&str> = typed.split_whitespace().collect();
    let joined = words.join(" ");
    let Some(denom_at) = joined.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ' '))
    else {
        return false; // the denomination is required
    };
    let (number, denom) = (joined[..denom_at].trim(), joined[denom_at..].trim());
    denom.eq_ignore_ascii_case(&coin.symbol)
        && evm::parse_units(number, coin.decimals).is_ok_and(|v| v == amount)
}

/// Most recently chosen coins kept for the search box's empty state.
pub const MAX_RECENT: usize = 8;

/// The public wallet choices kept on the backend for each account, so a new
/// sign-in opens the wallet as it was left: the active coin, recent picks,
/// user-added ERC-20s and the receive addresses derived in the browser.
/// Never a key or a phrase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSettings {
    pub active: String,
    pub recent: Vec<String>,
    /// Coin key → how often it was chosen or sent: the list's order.
    #[serde(default)]
    pub uses: std::collections::BTreeMap<String, u32>,
    /// Network key → ERC-20s added there, oldest first.
    pub tokens: std::collections::BTreeMap<String, Vec<CustomToken>>,
    /// Network key → this account's receive address there.
    pub addresses: std::collections::BTreeMap<String, String>,
}
impl Default for WalletSettings {
    fn default() -> Self {
        Self {
            active: DEFAULT_COIN.into(),
            recent: Vec::new(),
            uses: Default::default(),
            tokens: Default::default(),
            addresses: Default::default(),
        }
    }
}
impl WalletSettings {
    /// Built-in coins and this account's own ERC-20s, in registry order.
    pub fn all_coins(&self) -> Vec<Coin> {
        let mut all = coins();
        for (network, tokens) in &self.tokens {
            if net_of(network).is_some_and(|n| n.evm.is_some()) {
                all.extend(tokens.iter().map(|t| custom_coin(network, t)));
            }
        }
        all
    }
    /// The active coin, or the default if it is gone (a removed token).
    /// Coins in the order the list shows them: most used first, then the
    /// most recently picked, then registry order. Sorting is stable, so ties
    /// keep the order a user has learned.
    pub fn ranked(&self, mut coins: Vec<Coin>) -> Vec<Coin> {
        let recent = |key: &str| {
            self.recent
                .iter()
                .position(|k| k == key)
                .unwrap_or(usize::MAX)
        };
        coins.sort_by(|a, b| {
            let (ua, ub) = (
                self.uses.get(&a.key).copied().unwrap_or(0),
                self.uses.get(&b.key).copied().unwrap_or(0),
            );
            ub.cmp(&ua)
                .then_with(|| recent(&a.key).cmp(&recent(&b.key)))
        });
        coins
    }
    pub fn active_coin(&self) -> Coin {
        self.all_coins()
            .into_iter()
            .find(|c| c.key == self.active)
            .or_else(|| coin(DEFAULT_COIN))
            .expect("the default coin is in the registry")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn keys(found: Vec<&Coin>) -> Vec<String> {
        found.into_iter().map(|c| c.key.clone()).collect()
    }
    #[test]
    fn every_network_has_one_native_coin_and_tokens_name_known_networks() {
        let all = coins();
        assert_eq!(NETS.len(), 14);
        for n in NETS {
            assert_eq!(
                all.iter()
                    .filter(|c| c.network == n.key && c.standard == Standard::Native)
                    .count(),
                1,
                "{}",
                n.key
            );
            assert_eq!(n.evm.is_some(), n.chain == Chain::Evm);
            assert_eq!(
                n.faucet.is_some(),
                n.testnet,
                "{}: faucets are for test networks",
                n.key
            );
        }
        let mut seen = std::collections::HashSet::new();
        for c in &all {
            assert!(seen.insert(c.key.clone()), "duplicate {}", c.key);
            assert!(net_of(&c.network).is_some());
            if c.standard == Standard::Erc20 {
                let address = evm::parse_address(&c.id).unwrap();
                assert_eq!(evm::checksum(&address), c.id, "{} is checksummed", c.key);
            }
        }
        for key in MAIN_COINS {
            assert_eq!(coin(key).unwrap().network, "cronos-mainnet");
        }
        assert_eq!(
            coin("usdc-cronos-mainnet").unwrap().evm_asset().unwrap().1,
            Asset::Usdc
        );
        assert_eq!(
            coin("cro-cronos-mainnet").unwrap().evm_asset().unwrap().1,
            Asset::Native
        );
        assert!(
            matches!(coin("usdt-cronos-mainnet").unwrap().evm_asset().unwrap().1, Asset::Erc20(t) if t.decimals == 6)
        );
        assert_eq!(
            coin("usdc-arbitrum-one")
                .unwrap()
                .evm_asset()
                .unwrap()
                .0
                .chain_id,
            42161
        );
        assert_eq!(
            coin("eth-arbitrum-sepolia")
                .unwrap()
                .evm_asset()
                .unwrap()
                .0
                .chain_id,
            421614
        );
    }
    #[test]
    fn only_evm_coins_can_be_sent_for_now() {
        for c in coins() {
            assert_eq!(c.can_send(), c.net().chain == Chain::Evm, "{}", c.key);
        }
        assert!(coin("sol-solana-mainnet").unwrap().evm_asset().is_none());
    }
    #[test]
    fn search_follows_the_spec() {
        let all = coins();
        assert_eq!(search("", &all).len(), all.len());
        // Every term must match: USDC *and* Cronos.
        assert_eq!(
            keys(search("usdc cronos", &all)),
            ["usdc-cronos-mainnet", "devusdc.e-cronos-testnet"]
        );
        assert_eq!(
            keys(search("usdc, arbitrum", &all)),
            ["usdc-arbitrum-one", "usdc-arbitrum-sepolia"]
        );
        // Case and -/_/space are one difference.
        assert_eq!(
            search("CRONOS_MAINNET", &all),
            search("cronos-mainnet", &all)
        );
        assert!(
            search("Cronos Mainnet", &all)
                .iter()
                .all(|c| c.network == "cronos-mainnet")
        );
        // Tags find what names do not say.
        assert!(search("testnet", &all).iter().all(|c| c.net().testnet));
        assert!(
            search("testnet", &all)
                .iter()
                .any(|c| c.network == "cardano-preview")
        );
        assert_eq!(keys(search("sol devnet", &all))[0], "sol-solana-devnet");
        assert!(keys(search("night", &all)).contains(&"night-midnight-preview".to_string()));
        assert_eq!(
            keys(search("xec", &all)),
            ["xec-ecash-mainnet", "txec-ecash-testnet"]
        );
        assert_eq!(
            keys(search("l2 eth native", &all)),
            ["eth-arbitrum-one", "eth-arbitrum-sepolia"]
        );
        assert!(search("nothing-matches-this", &all).is_empty());
        // A contract address finds its token.
        assert_eq!(keys(search("0xaf88d065", &all)), ["usdc-arbitrum-one"]);
    }
    #[test]
    fn custom_tokens_join_the_search_and_send_as_erc20() {
        let token = CustomToken {
            address: "0x5C7F8A570d578ED84E63fdFA7b1eE72dEae1AE23".into(),
            symbol: "WCRO".into(),
            name: "Wrapped CRO".into(),
            decimals: 18,
        };
        let c = custom_coin("cronos-mainnet", &token);
        assert_eq!(
            c.key,
            "erc20-cronos-mainnet-0x5c7f8a570d578ed84e63fdfa7b1ee72deae1ae23"
        );
        assert!(c.can_send() && c.custom);
        assert_eq!(c.evm_asset().unwrap().1, Asset::Erc20(token));
        let mut all = coins();
        all.push(c);
        assert_eq!(search("wrapped", &all).len(), 1);
        assert_eq!(search("custom", &all).len(), 1);
    }
    #[test]
    fn explorer_links_keep_solanas_cluster_query() {
        assert_eq!(
            net_of("solana-devnet").unwrap().address_url("Abc"),
            "https://explorer.solana.com/address/Abc?cluster=devnet"
        );
        assert_eq!(
            net_of("cronos-mainnet").unwrap().tx_url("0x1"),
            "https://explorer.cronos.org/tx/0x1"
        );
        assert_eq!(
            net_of("arbitrum-one").unwrap().address_url("0x2"),
            "https://arbiscan.io/address/0x2"
        );
    }
    #[test]
    fn settings_fall_back_to_the_default_coin() {
        let mut s = WalletSettings::default();
        assert_eq!(s.active_coin().key, "tcro-cronos-testnet");
        s.active = "sol-solana-devnet".into();
        assert_eq!(s.active_coin().symbol, "SOL");
        s.active = custom_key(
            "cronos-mainnet",
            "0x5C7F8A570d578ED84E63fdFA7b1eE72dEae1AE23",
        );
        assert_eq!(s.active_coin().key, DEFAULT_COIN, "not added yet");
        s.tokens.insert(
            "cronos-mainnet".into(),
            vec![CustomToken {
                address: "0x5C7F8A570d578ED84E63fdFA7b1eE72dEae1AE23".into(),
                symbol: "WCRO".into(),
                name: "Wrapped CRO".into(),
                decimals: 18,
            }],
        );
        assert_eq!(s.active_coin().symbol, "WCRO");
        // Tokens filed under a non-EVM network are ignored, never shown.
        s.tokens.insert(
            "solana-mainnet".into(),
            vec![CustomToken {
                address: "x".into(),
                symbol: "X".into(),
                name: String::new(),
                decimals: 0,
            }],
        );
        assert!(!s.all_coins().iter().any(|c| c.symbol == "X"));
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(serde_json::from_value::<WalletSettings>(json).unwrap(), s);
    }
    #[test]
    fn the_list_is_ranked_by_use_then_recency_then_registry_order() {
        let mut s = WalletSettings::default();
        assert_eq!(s.active_coin().key, "tcro-cronos-testnet");
        let order = |s: &WalletSettings| {
            s.ranked(coins())
                .into_iter()
                .map(|c| c.key)
                .take(4)
                .collect::<Vec<_>>()
        };
        // Nothing used yet: registry order.
        assert_eq!(
            order(&s),
            [
                "cro-cronos-mainnet",
                "tcro-cronos-testnet",
                "eth-arbitrum-one",
                "eth-arbitrum-sepolia"
            ]
        );
        s.uses.insert("sol-solana-devnet".into(), 5);
        s.uses.insert("usdc-cronos-mainnet".into(), 9);
        s.uses.insert("xec-ecash-mainnet".into(), 5);
        // Ties on use are broken by recency.
        s.recent = vec!["xec-ecash-mainnet".into(), "sol-solana-devnet".into()];
        assert_eq!(
            order(&s),
            [
                "usdc-cronos-mainnet",
                "xec-ecash-mainnet",
                "sol-solana-devnet",
                "cro-cronos-mainnet"
            ]
        );
        // Older settings without counts still read.
        let old: WalletSettings = serde_json::from_value(serde_json::json!({"active":"usdc-cronos-mainnet","recent":[],"tokens":{},"addresses":{}})).unwrap();
        assert!(old.uses.is_empty());
    }
    #[test]
    fn amounts_take_their_denomination_and_refuse_another() {
        assert_eq!(amount_in(" 25 TCRO ", "TCRO").unwrap(), "25");
        assert_eq!(amount_in("25tcro", "TCRO").unwrap(), "25");
        assert_eq!(amount_in("0.5", "USDC").unwrap(), "0.5");
        assert_eq!(amount_in("1.5 devUSDC.e", "devUSDC.e").unwrap(), "1.5");
        assert_eq!(
            amount_in("25 USDC", "TCRO").unwrap_err(),
            "That amount is in USDC, but this sends TCRO."
        );
    }
    #[test]
    fn large_or_unknown_value_transfers_are_confirmed_twice() {
        let units = |c: &Coin, a: &str| evm::parse_units(a, c.decimals).unwrap();
        let tcro = coin("tcro-cronos-testnet").unwrap();
        let usdc = coin("usdc-cronos-mainnet").unwrap();
        let eth = coin("eth-arbitrum-one").unwrap();
        assert!(!needs_double_confirm(&tcro, units(&tcro, "20")));
        assert!(needs_double_confirm(
            &tcro,
            units(&tcro, "20.000000000000000001")
        ));
        assert!(!needs_double_confirm(&usdc, units(&usdc, "1")));
        assert!(needs_double_confirm(&usdc, units(&usdc, "1.000001")));
        // The wallet knows no dollar value for ETH or SOL: always twice.
        assert!(needs_double_confirm(&eth, units(&eth, "0.000001")));
        let wcro = custom_coin(
            "cronos-mainnet",
            &CustomToken {
                address: "0x5C7F8A570d578ED84E63fdFA7b1eE72dEae1AE23".into(),
                symbol: "WCRO".into(),
                name: "Wrapped CRO".into(),
                decimals: 18,
            },
        );
        assert!(needs_double_confirm(&wcro, units(&wcro, "1")));
    }
    #[test]
    fn the_typed_confirmation_must_name_the_amount_and_the_coin() {
        let tcro = coin("tcro-cronos-testnet").unwrap();
        let amount = evm::parse_units("25", 18).unwrap();
        assert_eq!(confirmation_text("25", "TCRO"), "25 TCRO");
        for ok in ["25 TCRO", "25 tcro", "  25   TCRO ", "25TCRO", "25.0 TCRO"] {
            assert!(confirms(ok, amount, &tcro), "{ok}");
        }
        for wrong in [
            "25", "TCRO", "2.5 TCRO", "250 TCRO", "25 CRO", "25 USDC", "",
        ] {
            assert!(!confirms(wrong, amount, &tcro), "{wrong}");
        }
    }
}
