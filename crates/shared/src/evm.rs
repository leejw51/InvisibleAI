//! Cronos EVM wallet core, ported from the Causewaybay Wallet (rustcli/core):
//! the network and stablecoin rows, the asset being sent (CRO, USDC or any
//! ERC-20), decimal amounts, EIP-55, ERC-20 calldata and legacy EIP-155
//! transactions. No network access and no persistence: the browser asks the
//! node, this module only encodes, signs and decodes.
use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::BTreeMap;
pub use u256::{U256, U256Error};

/// The only two networks the wallet harness serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Network {
    pub key: &'static str,
    pub name: &'static str,
    pub chain_id: u64,
    pub rpc: &'static str,
    pub explorer: &'static str,
    /// The network's own coin; it pays every fee, whatever is being sent.
    pub native: &'static str,
    pub native_name: &'static str,
    /// The fee ceiling in wei: a hostile or broken endpoint's gas price can make
    /// a fee almost anything, so nothing past this is signed.
    pub max_fee_wei: u128,
    pub usdc: Token,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub symbol: &'static str,
    pub name: &'static str,
    pub decimals: u8,
    pub address: &'static str,
}
// Both rows were read off their own chain: `symbol()` and `decimals()` answer
// "USDC"/6 on chain 25 and "devUSDC.e"/6 on chain 338.
pub const CRONOS_MAINNET: Network = Network {
    key: "cronos-mainnet",
    name: "Cronos Mainnet",
    chain_id: 25,
    rpc: "https://evm.cronos.org",
    explorer: "https://explorer.cronos.org",
    native: "CRO",
    native_name: "Cronos",
    max_fee_wei: 20_000_000_000_000_000_000, // 20 CRO
    usdc: Token {
        symbol: "USDC",
        name: "USD Coin",
        decimals: 6,
        address: "0xc21223249CA28397B4B6541dfFaEcC539BfF0c59",
    },
};
pub const CRONOS_TESTNET: Network = Network {
    key: "cronos-testnet",
    name: "Cronos Testnet",
    chain_id: 338,
    rpc: "https://evm-t3.cronos.org",
    explorer: "https://explorer.cronos.org/testnet",
    native: "TCRO",
    native_name: "Cronos test coin",
    max_fee_wei: 20_000_000_000_000_000_000, // 20 TCRO
    usdc: Token {
        symbol: "devUSDC.e",
        name: "USD Coin (Cronos testnet)",
        decimals: 6,
        address: "0xc01efAaF7C5C61bEbFAeb358E1161b537b8bC0e0",
    },
};
pub const NETWORKS: [Network; 2] = [CRONOS_TESTNET, CRONOS_MAINNET];
pub fn network(key: &str) -> Option<Network> {
    NETWORKS.into_iter().find(|n| n.key == key)
}

/// Parse a human decimal amount ("1.25") into base units, never via floats.
pub fn parse_units(amount: &str, decimals: u8) -> Result<U256, String> {
    let s = amount.trim();
    let s = s.strip_prefix('+').unwrap_or(s);
    let invalid = || format!("Enter an amount like 1.25 with at most {decimals} decimal places.");
    if s.is_empty() || s.starts_with('-') {
        return Err(invalid());
    }
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));
    if (int_part.is_empty() && frac_part.is_empty())
        || ![int_part, frac_part]
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_digit()))
        || frac_part.len() > decimals as usize
    {
        return Err(invalid());
    }
    let mut digits = String::from(if int_part.is_empty() { "0" } else { int_part });
    digits.push_str(frac_part);
    digits.extend(std::iter::repeat_n(
        '0',
        decimals as usize - frac_part.len(),
    ));
    let trimmed = digits.trim_start_matches('0');
    U256::from_str_radix(if trimmed.is_empty() { "0" } else { trimmed }, 10)
        .map_err(|_| "Amount is too large.".to_string())
}
/// Render base units as a decimal string without trailing zeros.
pub fn format_units(value: U256, decimals: u8) -> String {
    let digits = value.to_string();
    let decimals = decimals as usize;
    if decimals == 0 {
        return digits;
    }
    let padded = format!("{digits:0>width$}", width = decimals + 1);
    let (int_part, frac_part) = padded.split_at(padded.len() - decimals);
    let frac_part = frac_part.trim_end_matches('0');
    if frac_part.is_empty() {
        int_part.into()
    } else {
        format!("{int_part}.{frac_part}")
    }
}
/// A JSON-RPC quantity or 32-byte word ("0x…") as an integer.
pub fn parse_quantity(hex_value: &str) -> Result<U256, String> {
    let raw = hex_value
        .strip_prefix("0x")
        .ok_or("The node returned an unexpected number")?;
    if raw.is_empty() {
        return Ok(U256::ZERO);
    }
    U256::from_str_radix(raw, 16).map_err(|_| "The node returned an unexpected number".into())
}

pub fn checksum(address: &[u8; 20]) -> String {
    let lower = hex::encode(address);
    let hash = Keccak256::digest(lower.as_bytes());
    let mut out = String::from("0x");
    for (i, c) in lower.chars().enumerate() {
        let nibble = (hash[i / 2] >> if i % 2 == 0 { 4 } else { 0 }) & 0xf;
        out.push(if c.is_ascii_alphabetic() && nibble >= 8 {
            c.to_ascii_uppercase()
        } else {
            c
        });
    }
    out
}
/// All-lowercase and all-uppercase carry no checksum; mixed case must verify,
/// because it fails exactly when a character has been corrupted.
pub fn parse_address(value: &str) -> Result<[u8; 20], String> {
    let value = value.trim();
    let raw = value
        .strip_prefix("0x")
        .filter(|r| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Enter a 0x-prefixed address of 40 hex characters.")?;
    let mut out = [0u8; 20];
    hex::decode_to_slice(raw, &mut out).map_err(|_| "Invalid address")?;
    let mixed =
        raw.bytes().any(|b| b.is_ascii_lowercase()) && raw.bytes().any(|b| b.is_ascii_uppercase());
    if mixed && checksum(&out) != value {
        return Err("Address checksum does not match. Check for a mistyped character.".into());
    }
    Ok(out)
}

fn word(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    out
}
/// `transfer(address,uint256)`
pub fn erc20_transfer(to: &[u8; 20], amount: U256) -> Vec<u8> {
    let mut out = vec![0xa9, 0x05, 0x9c, 0xbb];
    out.extend_from_slice(&word(to));
    out.extend_from_slice(&amount.to_be_bytes());
    out
}
/// `balanceOf(address)`
pub fn erc20_balance_of(owner: &[u8; 20]) -> Vec<u8> {
    let mut out = vec![0x70, 0xa0, 0x82, 0x31];
    out.extend_from_slice(&word(owner));
    out
}

/// Any ERC-20 the user added by contract address, as its contract describes
/// itself. The symbol is the contract's own claim, so the address is always
/// shown beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomToken {
    /// EIP-55 checksummed contract address.
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
}
/// What the wallet shows and sends on one network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asset {
    /// CRO (TCRO on testnet): the network's coin, which also pays every fee.
    Native,
    Usdc,
    Erc20(CustomToken),
}
pub const NATIVE_KEY: &str = "native";
pub const USDC_KEY: &str = "usdc";
impl Asset {
    /// `native`, `usdc`, or the token's lowercase contract address.
    pub fn key(&self) -> String {
        match self {
            Asset::Native => NATIVE_KEY.into(),
            Asset::Usdc => USDC_KEY.into(),
            Asset::Erc20(t) => t.address.to_ascii_lowercase(),
        }
    }
    pub fn symbol(&self, network: &Network) -> String {
        match self {
            Asset::Native => network.native.into(),
            Asset::Usdc => network.usdc.symbol.into(),
            Asset::Erc20(t) => t.symbol.clone(),
        }
    }
    pub fn name(&self, network: &Network) -> String {
        match self {
            Asset::Native => network.native_name.into(),
            Asset::Usdc => network.usdc.name.into(),
            Asset::Erc20(t) => t.name.clone(),
        }
    }
    pub fn decimals(&self, network: &Network) -> u8 {
        match self {
            Asset::Native => 18,
            Asset::Usdc => network.usdc.decimals,
            Asset::Erc20(t) => t.decimals,
        }
    }
    /// The token contract, checksummed; `None` for the network's own coin.
    pub fn contract(&self, network: &Network) -> Option<String> {
        match self {
            Asset::Native => None,
            Asset::Usdc => Some(network.usdc.address.into()),
            Asset::Erc20(t) => Some(t.address.clone()),
        }
    }
}

/// A transfer that passed every check that needs no network: the recipient
/// parses (and verifies, if checksummed), is neither the sender nor the token's
/// own contract, and the amount is positive, within the asset's decimals and
/// within its balance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRequest {
    pub recipient: [u8; 20],
    /// The amount in the asset's base units.
    pub amount: U256,
    /// The transaction's destination: the recipient for CRO, else the contract.
    pub to: [u8; 20],
    /// Native value attached: the amount for CRO, zero for a token.
    pub value: U256,
    pub data: Vec<u8>,
}
pub fn check_transfer(
    network: &Network,
    asset: &Asset,
    from: &str,
    to: &str,
    amount: &str,
    balance: U256,
) -> Result<TransferRequest, String> {
    let recipient = parse_address(to)?;
    if from.trim().eq_ignore_ascii_case(to.trim()) {
        return Err(
            "That is your own address. Sending to yourself moves nothing and still costs a fee."
                .into(),
        );
    }
    let contract = asset
        .contract(network)
        .map(|c| parse_address(&c))
        .transpose()?;
    if contract == Some(recipient) {
        return Err(
            "That is the token's own contract address. Tokens sent there are usually lost.".into(),
        );
    }
    let decimals = asset.decimals(network);
    let value = parse_units(amount, decimals)?;
    if value.is_zero() {
        return Err("Enter an amount greater than zero.".into());
    }
    if value > balance {
        return Err(format!(
            "You have {} {}.",
            format_units(balance, decimals),
            asset.symbol(network)
        ));
    }
    Ok(match contract {
        Some(contract) => TransferRequest {
            data: erc20_transfer(&recipient, value),
            recipient,
            amount: value,
            to: contract,
            value: U256::ZERO,
        },
        None => TransferRequest {
            recipient,
            amount: value,
            to: recipient,
            value,
            data: Vec::new(),
        },
    })
}
/// What the node said about the sender and the chain right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quote {
    pub nonce: U256,
    pub gas_price: U256,
    /// `eth_estimateGas`, or `None` when the node could not estimate.
    pub estimate: Option<U256>,
}
/// Gas limit when the node cannot estimate an ERC-20 transfer.
pub const FALLBACK_GAS: u64 = 100_000;
/// Gas for a plain CRO payment to an ordinary address.
pub const NATIVE_GAS: u64 = 21_000;
/// Build the unsigned transaction and refuse it if the fee passes the
/// network's ceiling or, with any CRO being sent, the native balance, before
/// any key is touched.
pub fn build_transfer(
    network: &Network,
    request: &TransferRequest,
    quote: Quote,
    native_balance: U256,
) -> Result<LegacyTransaction, String> {
    let nonce = u64::try_from(quote.nonce)
        .map_err(|_| "The network returned an impossible nonce.".to_string())?;
    let native = request.data.is_empty();
    let estimate = quote
        .estimate
        .and_then(|e| u64::try_from(e).ok())
        .filter(|e| *e > 0);
    let gas_limit = match (native, estimate) {
        // An ordinary address costs exactly 21,000; a contract gets headroom.
        (true, Some(e)) if e <= NATIVE_GAS => NATIVE_GAS,
        (true, None) => NATIVE_GAS,
        (false, None) => FALLBACK_GAS,
        (_, Some(e)) => with_headroom(e),
    };
    let tx = LegacyTransaction {
        nonce,
        gas_price: quote.gas_price,
        gas_limit,
        to: request.to,
        value: request.value,
        data: request.data.clone(),
        chain_id: network.chain_id,
    };
    let fee = tx.fee();
    let ceiling = U256::from(network.max_fee_wei);
    if fee > ceiling {
        return Err(format!(
            "The network asked for a fee of {} {}, above this wallet's {} {} ceiling. Not signing.",
            format_units(fee, 18),
            network.native,
            format_units(ceiling, 18),
            network.native
        ));
    }
    let total = fee.checked_add(request.value).unwrap_or(U256::MAX);
    if total > native_balance {
        return Err(if native {
            format!(
                "You have {} {}. Sending {} plus a fee of about {} needs {}.",
                format_units(native_balance, 18),
                network.native,
                format_units(request.value, 18),
                format_units(fee, 18),
                format_units(total, 18)
            )
        } else {
            format!(
                "You need about {} {} for the network fee.",
                format_units(fee, 18),
                network.native
            )
        });
    }
    Ok(tx)
}
/// The most CRO that can be sent to an ordinary address at this gas price:
/// the balance less a 21,000-gas fee, never below zero.
pub fn max_native(balance: U256, gas_price: U256) -> U256 {
    balance
        .checked_sub(gas_price.saturating_mul_u64(NATIVE_GAS))
        .unwrap_or(U256::ZERO)
}

/// `symbol()`, `name()` and `decimals()`: how an ERC-20 describes itself.
pub const ERC20_SYMBOL: [u8; 4] = [0x95, 0xd8, 0x9b, 0x41];
pub const ERC20_NAME: [u8; 4] = [0x06, 0xfd, 0xde, 0x03];
pub const ERC20_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];
/// Decimals past this are refused: no real token uses them, and they would
/// make amounts unreadable.
pub const MAX_DECIMALS: u8 = 36;
pub const MAX_SYMBOL_CHARS: usize = 16;
pub const MAX_NAME_CHARS: usize = 64;
/// Saved tokens per network.
pub const MAX_TOKENS: usize = 50;

/// A string a token returned, made safe to show: no control characters and no
/// invisible or direction-changing marks that could disguise what is shown.
pub fn clean_label(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2069 | 0xFEFF)
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}
/// An ABI `string` return, or the older `bytes32` form some tokens use.
pub fn abi_string(bytes: &[u8]) -> Result<String, String> {
    let bad = || "The contract did not answer like an ERC-20 token.".to_string();
    let text = if bytes.len() == 32 {
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(32);
        &bytes[..end]
    } else {
        let read = |at: usize| -> Result<usize, String> {
            let word = bytes
                .get(at..at.checked_add(32).ok_or_else(bad)?)
                .ok_or_else(bad)?;
            if word[..24].iter().any(|b| *b != 0) {
                return Err(bad());
            }
            usize::try_from(u64::from_be_bytes(word[24..].try_into().unwrap())).map_err(|_| bad())
        };
        let offset = read(0)?;
        let len = read(offset)?;
        let start = offset.checked_add(32).ok_or_else(bad)?;
        bytes
            .get(start..start.checked_add(len).ok_or_else(bad)?)
            .ok_or_else(bad)?
    };
    String::from_utf8(text.to_vec()).map_err(|_| bad())
}
pub fn abi_decimals(bytes: &[u8]) -> Result<u8, String> {
    if bytes.len() != 32 {
        return Err("The contract did not answer like an ERC-20 token.".into());
    }
    u64::try_from(U256::from_be_slice(bytes))
        .ok()
        .and_then(|d| u8::try_from(d).ok())
        .filter(|d| *d <= MAX_DECIMALS)
        .ok_or_else(|| format!("This token uses more than {MAX_DECIMALS} decimals. Not supported."))
}
/// A token as it may be saved: the address checksummed, the labels cleaned,
/// a symbol present, the decimals in range, and not the built-in USDC.
pub fn validate_token(network: &Network, token: &CustomToken) -> Result<CustomToken, String> {
    let address = checksum(&parse_address(&token.address)?);
    if address.eq_ignore_ascii_case(network.usdc.address) {
        return Err(format!(
            "That is {} on {}. Choose USDC instead.",
            network.usdc.symbol, network.name
        ));
    }
    let symbol = clean_label(&token.symbol, MAX_SYMBOL_CHARS);
    if symbol.is_empty() {
        return Err("This token has no symbol.".into());
    }
    if token.decimals > MAX_DECIMALS {
        return Err(format!(
            "This token uses more than {MAX_DECIMALS} decimals. Not supported."
        ));
    }
    Ok(CustomToken {
        address,
        symbol,
        name: clean_label(&token.name, MAX_NAME_CHARS),
        decimals: token.decimals,
    })
}
/// A custom token whose symbol or name claims a built-in asset: worth a
/// warning, because any contract can call itself "USDC".
pub fn imitates_builtin(network: &Network, token: &CustomToken) -> bool {
    let squash = |s: &str| {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let (symbol, name) = (squash(&token.symbol), squash(&token.name));
    let usdc = squash(network.usdc.symbol);
    symbol.contains("USDC")
        || symbol.contains(&usdc)
        || name.contains("USDCOIN")
        || symbol == "CRO"
        || symbol == "TCRO"
}

/// The wallet choices kept on the backend for each account, so a new sign-in
/// opens the same network and asset with the same saved tokens. Never a key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSettings {
    /// The network key last opened.
    pub network: String,
    /// Network key → asset key (`native`, `usdc` or a saved token's address).
    pub selected: BTreeMap<String, String>,
    /// Network key → tokens saved there, oldest first.
    pub tokens: BTreeMap<String, Vec<CustomToken>>,
}
impl Default for WalletSettings {
    fn default() -> Self {
        Self {
            network: CRONOS_TESTNET.key.into(),
            selected: BTreeMap::new(),
            tokens: BTreeMap::new(),
        }
    }
}
impl WalletSettings {
    pub fn tokens_on(&self, network: &Network) -> &[CustomToken] {
        self.tokens
            .get(network.key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    /// The asset chosen on `network`: USDC until something else is picked, and
    /// USDC again if a picked token is no longer saved.
    pub fn asset_on(&self, network: &Network) -> Asset {
        match self.selected.get(network.key).map(String::as_str) {
            Some(NATIVE_KEY) => Asset::Native,
            Some(address) if address != USDC_KEY => self
                .tokens_on(network)
                .iter()
                .find(|t| t.address.eq_ignore_ascii_case(address))
                .cloned()
                .map(Asset::Erc20)
                .unwrap_or(Asset::Usdc),
            _ => Asset::Usdc,
        }
    }
}

mod u256 {
    //! Just enough 256-bit unsigned arithmetic for balances, amounts and fees:
    //! big-endian bytes, so the derived ordering is numeric ordering.
    use std::fmt;
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct U256([u8; 32]);
    /// Not a number in the given radix, or larger than 256 bits.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct U256Error;
    impl fmt::Display for U256Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("not a 256-bit unsigned number")
        }
    }
    impl std::error::Error for U256Error {}
    impl U256 {
        pub const ZERO: Self = Self([0; 32]);
        pub fn is_zero(&self) -> bool {
            self.0 == [0; 32]
        }
        pub fn to_be_bytes(&self) -> [u8; 32] {
            self.0
        }
        /// Up to 32 big-endian bytes.
        pub fn from_be_slice(bytes: &[u8]) -> Self {
            assert!(bytes.len() <= 32);
            let mut out = [0u8; 32];
            out[32 - bytes.len()..].copy_from_slice(bytes);
            Self(out)
        }
        /// `self * m + add`, or `None` on overflow.
        fn mul_add(&self, m: u64, add: u64) -> Option<Self> {
            let mut out = [0u8; 32];
            let mut carry = add as u128;
            for i in (0..32).rev() {
                let v = self.0[i] as u128 * m as u128 + carry;
                out[i] = v as u8;
                carry = v >> 8;
            }
            (carry == 0).then_some(Self(out))
        }
        pub const MAX: Self = Self([0xff; 32]);
        pub fn checked_add(&self, other: Self) -> Option<Self> {
            let mut out = [0u8; 32];
            let mut carry = 0u16;
            for i in (0..32).rev() {
                let v = self.0[i] as u16 + other.0[i] as u16 + carry;
                out[i] = v as u8;
                carry = v >> 8;
            }
            (carry == 0).then_some(Self(out))
        }
        pub fn checked_sub(&self, other: Self) -> Option<Self> {
            if *self < other {
                return None;
            }
            let mut out = [0u8; 32];
            let mut borrow = 0i16;
            for i in (0..32).rev() {
                let mut v = self.0[i] as i16 - other.0[i] as i16 - borrow;
                borrow = (v < 0) as i16;
                if v < 0 {
                    v += 256;
                }
                out[i] = v as u8;
            }
            Some(Self(out))
        }
        pub fn saturating_mul_u64(&self, m: u64) -> Self {
            self.mul_add(m, 0).unwrap_or(Self([0xff; 32]))
        }
        /// `(self / d, self % d)`, long division one byte at a time.
        fn div_rem_u64(&self, d: u64) -> (Self, u64) {
            assert!(d != 0);
            let mut out = [0u8; 32];
            let mut rem = 0u128;
            for (q, byte) in out.iter_mut().zip(self.0) {
                let v = (rem << 8) | byte as u128;
                *q = (v / d as u128) as u8;
                rem = v % d as u128;
            }
            (Self(out), rem as u64)
        }
        /// `self / d`, rounding down.
        pub fn div_floor_u64(&self, d: u64) -> Self {
            self.div_rem_u64(d).0
        }
        pub fn from_str_radix(s: &str, radix: u32) -> Result<Self, U256Error> {
            if s.is_empty() || !(radix == 10 || radix == 16) {
                return Err(U256Error);
            }
            s.chars().try_fold(Self::ZERO, |acc, c| {
                let digit = c.to_digit(radix).ok_or(U256Error)?;
                acc.mul_add(radix as u64, digit as u64).ok_or(U256Error)
            })
        }
    }
    impl From<u64> for U256 {
        fn from(v: u64) -> Self {
            Self::from_be_slice(&v.to_be_bytes())
        }
    }
    impl From<u128> for U256 {
        fn from(v: u128) -> Self {
            Self::from_be_slice(&v.to_be_bytes())
        }
    }
    impl TryFrom<U256> for u64 {
        type Error = U256Error;
        fn try_from(v: U256) -> Result<u64, U256Error> {
            if v.0[..24].iter().any(|b| *b != 0) {
                return Err(U256Error);
            }
            Ok(u64::from_be_bytes(v.0[24..].try_into().unwrap()))
        }
    }
    impl std::str::FromStr for U256 {
        type Err = U256Error;
        fn from_str(s: &str) -> Result<Self, U256Error> {
            Self::from_str_radix(s, 10)
        }
    }
    impl fmt::Display for U256 {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if self.is_zero() {
                return f.write_str("0");
            }
            let mut digits = Vec::new();
            let mut n = *self;
            while !n.is_zero() {
                let (q, r) = n.div_rem_u64(10);
                digits.push(b'0' + r as u8);
                n = q;
            }
            digits.reverse();
            f.write_str(std::str::from_utf8(&digits).unwrap())
        }
    }
}

mod rlp {
    use super::U256;
    fn trim(bytes: &[u8]) -> &[u8] {
        &bytes[bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len())..]
    }
    fn header(out: &mut Vec<u8>, short: u8, long: u8, len: usize) {
        if len <= 55 {
            out.push(short + len as u8);
        } else {
            let be = len.to_be_bytes();
            let be = trim(&be);
            out.push(long + be.len() as u8);
            out.extend_from_slice(be);
        }
    }
    pub fn bytes(out: &mut Vec<u8>, value: &[u8]) {
        if value.len() == 1 && value[0] < 0x80 {
            out.push(value[0]);
        } else {
            header(out, 0x80, 0xb7, value.len());
            out.extend_from_slice(value);
        }
    }
    pub fn uint(out: &mut Vec<u8>, value: U256) {
        bytes(out, trim(&value.to_be_bytes()));
    }
    pub fn list(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + 9);
        header(&mut out, 0xc0, 0xf7, payload.len());
        out.extend_from_slice(payload);
        out
    }
}

/// A legacy (type 0x0) transaction with EIP-155 replay protection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyTransaction {
    pub nonce: u64,
    pub gas_price: U256,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub value: U256,
    pub data: Vec<u8>,
    pub chain_id: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedTransaction {
    pub raw: Vec<u8>,
    pub hash: [u8; 32],
    pub v: u64,
}
impl SignedTransaction {
    pub fn raw_hex(&self) -> String {
        format!("0x{}", hex::encode(&self.raw))
    }
    pub fn hash_hex(&self) -> String {
        format!("0x{}", hex::encode(self.hash))
    }
}
impl LegacyTransaction {
    fn body(&self, out: &mut Vec<u8>) {
        rlp::uint(out, U256::from(self.nonce));
        rlp::uint(out, self.gas_price);
        rlp::uint(out, U256::from(self.gas_limit));
        rlp::bytes(out, &self.to);
        rlp::uint(out, self.value);
        rlp::bytes(out, &self.data);
    }
    pub fn fee(&self) -> U256 {
        self.gas_price.saturating_mul_u64(self.gas_limit)
    }
    pub fn signing_hash(&self) -> [u8; 32] {
        let mut payload = Vec::new();
        self.body(&mut payload);
        rlp::uint(&mut payload, U256::from(self.chain_id));
        rlp::uint(&mut payload, U256::ZERO);
        rlp::uint(&mut payload, U256::ZERO);
        Keccak256::digest(rlp::list(&payload)).into()
    }
    /// `v = recovery + chainId * 2 + 35`.
    pub fn sign(&self, key: &SigningKey) -> Result<SignedTransaction, String> {
        let (signature, recovery) = key
            .sign_prehash_recoverable(&self.signing_hash())
            .map_err(|_| "Could not sign the transaction".to_string())?;
        let v = recovery.to_byte() as u64 + self.chain_id * 2 + 35;
        let bytes = signature.to_bytes();
        let mut payload = Vec::new();
        self.body(&mut payload);
        rlp::uint(&mut payload, U256::from(v));
        rlp::uint(&mut payload, U256::from_be_slice(&bytes[..32]));
        rlp::uint(&mut payload, U256::from_be_slice(&bytes[32..]));
        let raw = rlp::list(&payload);
        Ok(SignedTransaction {
            hash: Keccak256::digest(&raw).into(),
            raw,
            v,
        })
    }
}
/// Gas limit for a contract call: the node's estimate plus 25 % headroom.
pub fn with_headroom(estimate: u64) -> u64 {
    estimate.saturating_add(estimate / 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn vectors(name: &str) -> Value {
        let path = format!("{}/testvectors/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }
    fn key(hex_key: &str) -> SigningKey {
        SigningKey::from_slice(&hex::decode(hex_key.trim_start_matches("0x")).unwrap()).unwrap()
    }
    #[test]
    fn signs_every_reference_transaction() {
        for v in vectors("transactions")["vectors"].as_array().unwrap() {
            let tx = LegacyTransaction {
                nonce: v["nonce"].as_u64().unwrap(),
                gas_price: v["gas_price"].as_str().unwrap().parse().unwrap(),
                gas_limit: v["gas_limit"].as_u64().unwrap(),
                to: parse_address(v["to"].as_str().unwrap()).unwrap(),
                value: v["value"].as_str().unwrap().parse().unwrap(),
                data: hex::decode(v["data"].as_str().unwrap().trim_start_matches("0x")).unwrap(),
                chain_id: v["chain_id"].as_u64().unwrap(),
            };
            let signed = tx.sign(&key(v["private_key"].as_str().unwrap())).unwrap();
            let name = v["name"].as_str().unwrap();
            assert_eq!(signed.raw_hex(), v["raw"].as_str().unwrap(), "{name}");
            assert_eq!(signed.hash_hex(), v["hash"].as_str().unwrap(), "{name}");
            assert_eq!(signed.v, v["v"].as_u64().unwrap(), "{name}");
        }
    }
    #[test]
    fn erc20_calldata_matches_the_reference_vector() {
        let v = &vectors("transactions")["vectors"][3];
        let to = parse_address("0x9858effd232b4033e47d90003d41ec34ecaeda94").unwrap();
        let data = erc20_transfer(&to, parse_units("1.5", 18).unwrap());
        assert_eq!(
            format!("0x{}", hex::encode(data)),
            v["data"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(erc20_balance_of(&to)),
            "70a082310000000000000000000000009858effd232b4033e47d90003d41ec34ecaeda94"
        );
    }
    #[test]
    fn eip55_round_trips_and_rejects_corruption() {
        for v in vectors("eip55")["vectors"].as_array().unwrap() {
            let lower = v["lowercase"].as_str().unwrap();
            let good = v["checksummed"].as_str().unwrap();
            let bytes = parse_address(lower).unwrap();
            assert_eq!(checksum(&bytes), good);
            assert_eq!(parse_address(good).unwrap(), bytes);
            assert_eq!(
                parse_address(&lower.to_uppercase().replacen("0X", "0x", 1)).unwrap(),
                bytes
            );
            // Flip the case of the first letter: still hex, no longer the checksum.
            let i = good
                .char_indices()
                .skip(2)
                .find(|(_, c)| c.is_ascii_alphabetic());
            if let Some((i, c)) = i {
                let mut bad = good.to_string();
                let flipped = if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c.to_ascii_uppercase()
                };
                bad.replace_range(i..i + 1, &flipped.to_string());
                if bad.chars().skip(2).any(|c| c.is_ascii_uppercase())
                    && bad.chars().skip(2).any(|c| c.is_ascii_lowercase())
                {
                    assert!(parse_address(&bad).is_err(), "{bad}");
                }
            }
        }
        assert!(parse_address("0x1234").is_err());
        assert!(parse_address("9858effd232b4033e47d90003d41ec34ecaeda94").is_err());
    }
    #[test]
    fn units_match_the_reference_vectors() {
        let v = vectors("units");
        for row in v["valid"].as_array().unwrap() {
            let decimals = row["decimals"].as_u64().unwrap() as u8;
            let value = parse_units(row["amount"].as_str().unwrap(), decimals).unwrap();
            assert_eq!(value.to_string(), row["value"].as_str().unwrap());
            assert_eq!(
                parse_units(&format_units(value, decimals), decimals).unwrap(),
                value
            );
        }
        for row in v["invalid"].as_array().unwrap() {
            let decimals = row["decimals"].as_u64().unwrap() as u8;
            assert!(
                parse_units(row["amount"].as_str().unwrap(), decimals).is_err(),
                "{row}"
            );
        }
        assert_eq!(format_units(U256::from(1_500_000u64), 6), "1.5");
        assert_eq!(format_units(U256::from(5u64), 6), "0.000005");
        assert_eq!(format_units(U256::ZERO, 6), "0");
    }
    #[test]
    fn quantities_and_networks() {
        assert_eq!(parse_quantity("0x").unwrap(), U256::ZERO);
        assert_eq!(
            parse_quantity("0x0f4240").unwrap(),
            U256::from(1_000_000u64)
        );
        assert!(parse_quantity("12").is_err());
        assert_eq!(network("cronos-mainnet").unwrap().chain_id, 25);
        assert_eq!(network("cronos-testnet").unwrap().chain_id, 338);
        assert!(network("ethereum").is_none());
        for n in NETWORKS {
            assert_eq!(
                parse_address(n.usdc.address).map(|a| checksum(&a)).unwrap(),
                n.usdc.address
            );
            assert_eq!(n.usdc.decimals, 6);
        }
        assert_eq!(with_headroom(40_000), 50_000);
    }
    const ME: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
    const YOU: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    fn usdc(amount: &str) -> U256 {
        parse_units(amount, 6).unwrap()
    }
    fn cro(amount: &str) -> U256 {
        parse_units(amount, 18).unwrap()
    }
    fn quote(nonce: u64, gwei: u64, estimate: Option<u64>) -> Quote {
        Quote {
            nonce: U256::from(nonce),
            gas_price: U256::from(gwei).saturating_mul_u64(1_000_000_000),
            estimate: estimate.map(U256::from),
        }
    }
    #[test]
    fn check_transfer_accepts_a_valid_request() {
        let r =
            check_transfer(&CRONOS_TESTNET, &Asset::Usdc, ME, YOU, " 1.25 ", usdc("10")).unwrap();
        assert_eq!(r.amount, usdc("1.25"));
        assert_eq!(checksum(&r.to), CRONOS_TESTNET.usdc.address);
        assert!(r.value.is_zero());
        assert_eq!(checksum(&r.recipient), YOU);
        assert_eq!(r.data, erc20_transfer(&r.recipient, usdc("1.25")));
        // The whole balance is allowed; lowercase needs no checksum.
        assert!(
            check_transfer(
                &CRONOS_TESTNET,
                &Asset::Usdc,
                ME,
                &YOU.to_lowercase(),
                "10",
                usdc("10")
            )
            .is_ok()
        );
    }
    #[test]
    fn check_transfer_refuses_before_any_network_call() {
        let n = &CRONOS_MAINNET;
        let err = |to: &str, amount: &str| {
            check_transfer(n, &Asset::Usdc, ME, to, amount, usdc("10")).unwrap_err()
        };
        assert!(err(ME, "1").contains("your own address"));
        assert!(err(&ME.to_uppercase().replacen("0X", "0x", 1), "1").contains("your own address"));
        assert!(err("0xF39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "1").contains("checksum"));
        assert!(err("0x1234", "1").contains("40 hex characters"));
        assert!(err(YOU, "0").contains("greater than zero"));
        assert!(err(YOU, "0.000000").contains("greater than zero"));
        assert!(err(YOU, "1.0000001").contains("at most 6 decimal places"));
        assert!(err(YOU, "-1").contains("at most 6"));
        assert!(err(YOU, "1e3").contains("at most 6"));
        assert_eq!(err(YOU, "10.000001"), "You have 10 USDC.");
        assert!(err(n.usdc.address, "1").contains("token's own contract"));
        assert_eq!(
            check_transfer(&CRONOS_TESTNET, &Asset::Usdc, ME, YOU, "11", usdc("10")).unwrap_err(),
            "You have 10 devUSDC.e."
        );
    }
    #[test]
    fn build_transfer_targets_the_network_contract_with_headroom() {
        for n in NETWORKS {
            let r = check_transfer(&n, &Asset::Usdc, ME, YOU, "2", usdc("5")).unwrap();
            let tx = build_transfer(&n, &r, quote(7, 5_000, Some(50_000)), cro("1")).unwrap();
            assert_eq!(tx.chain_id, n.chain_id);
            assert_eq!(checksum(&tx.to), n.usdc.address);
            assert_eq!(tx.value, U256::ZERO);
            assert_eq!(tx.nonce, 7);
            assert_eq!(tx.gas_limit, 62_500);
            assert_eq!(tx.data, r.data);
            assert_eq!(format_units(tx.fee(), 18), "0.3125");
        }
    }
    #[test]
    fn build_transfer_falls_back_and_refuses_unaffordable_fees() {
        let n = &CRONOS_TESTNET;
        let r = check_transfer(n, &Asset::Usdc, ME, YOU, "1", usdc("5")).unwrap();
        // No estimate, a zero estimate, or an absurd one: the fixed fallback.
        for estimate in [None, Some(0)] {
            assert_eq!(
                build_transfer(n, &r, quote(0, 1, estimate), cro("1"))
                    .unwrap()
                    .gas_limit,
                FALLBACK_GAS
            );
        }
        let huge = Quote {
            estimate: Some(U256::from(u128::MAX)),
            ..quote(0, 1, None)
        };
        assert_eq!(
            build_transfer(n, &r, huge, cro("1")).unwrap().gas_limit,
            FALLBACK_GAS
        );
        // Exactly at the 20 TCRO ceiling is allowed; one wei over is not.
        let at = Quote {
            gas_price: cro("20").div_floor_u64(FALLBACK_GAS),
            ..quote(0, 0, None)
        };
        assert!(build_transfer(n, &r, at, cro("20")).is_ok());
        let over = Quote {
            gas_price: U256::from(200_000_000_000_001u64),
            ..quote(0, 0, None)
        };
        assert!(
            build_transfer(n, &r, over, cro("100"))
                .unwrap_err()
                .ends_with("ceiling. Not signing.")
        );
        // Affordable by the ceiling but not by the balance.
        assert_eq!(
            build_transfer(n, &r, quote(0, 5_000, Some(50_000)), cro("0.3")).unwrap_err(),
            "You need about 0.3125 TCRO for the network fee."
        );
        assert!(build_transfer(n, &r, quote(0, 5_000, Some(50_000)), cro("0.3125")).is_ok());
        // A nonce that cannot be a u64 is the node lying.
        let bad = Quote {
            nonce: U256::from(u128::MAX),
            ..quote(0, 1, None)
        };
        assert!(
            build_transfer(n, &r, bad, cro("1"))
                .unwrap_err()
                .contains("impossible nonce")
        );
    }
    #[test]
    fn u256_parses_formats_and_bounds() {
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(max.parse::<U256>().unwrap().to_string(), max);
        assert!(
            "115792089237316195423570985008687907853269984665640564039457584007913129639936"
                .parse::<U256>()
                .is_err()
        );
        assert_eq!(U256::from_str_radix("ff", 16).unwrap(), U256::from(255u64));
        assert!(U256::from_str_radix("fg", 16).is_err());
        assert!(U256::from(2u64) > U256::from(1u64));
        assert!(U256::from(u128::MAX) > U256::from(u64::MAX));
        assert_eq!(u64::try_from(U256::from(7u64)), Ok(7));
        assert!(u64::try_from(U256::from(u128::MAX)).is_err());
        assert_eq!(U256::from(3u64).saturating_mul_u64(5), U256::from(15u64));
        assert_eq!(
            max.parse::<U256>()
                .unwrap()
                .saturating_mul_u64(2)
                .to_string(),
            max
        );
    }

    fn token(address: &str, symbol: &str, decimals: u8) -> CustomToken {
        CustomToken {
            address: address.into(),
            symbol: symbol.into(),
            name: format!("{symbol} token"),
            decimals,
        }
    }
    const WCRO: &str = "0x5C7F8A570d578ED84E63fdFA7b1eE72dEae1AE23";
    #[test]
    fn a_native_transfer_signs_to_the_reference_vector() {
        // The Cronos testnet vector: 1 TCRO to 0x3535…, 21,000 gas, no data.
        let v = &vectors("transactions")["vectors"][1];
        let from = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
        let r = check_transfer(
            &CRONOS_TESTNET,
            &Asset::Native,
            from,
            v["to"].as_str().unwrap(),
            "1",
            cro("5"),
        )
        .unwrap();
        assert_eq!(r.value, cro("1"));
        assert_eq!(r.to, r.recipient);
        assert!(r.data.is_empty());
        let q = Quote {
            nonce: U256::from(v["nonce"].as_u64().unwrap()),
            gas_price: v["gas_price"].as_str().unwrap().parse().unwrap(),
            estimate: Some(U256::from(21_000u64)),
        };
        let tx = build_transfer(&CRONOS_TESTNET, &r, q, cro("5")).unwrap();
        assert_eq!(tx.gas_limit, NATIVE_GAS);
        let signed = tx.sign(&key(v["private_key"].as_str().unwrap())).unwrap();
        assert_eq!(signed.raw_hex(), v["raw"].as_str().unwrap());
    }
    #[test]
    fn native_transfers_need_the_amount_plus_the_fee() {
        let n = &CRONOS_MAINNET;
        let r = check_transfer(n, &Asset::Native, ME, YOU, "1", cro("1")).unwrap();
        // 21,000 gas × 5,000 gwei = 0.105 CRO.
        let q = quote(0, 5_000, None);
        assert_eq!(
            build_transfer(n, &r, q, cro("1")).unwrap_err(),
            "You have 1 CRO. Sending 1 plus a fee of about 0.105 needs 1.105."
        );
        assert!(build_transfer(n, &r, q, cro("1.105")).is_ok());
        // A contract recipient estimated above 21,000 gets headroom.
        assert_eq!(
            build_transfer(n, &r, quote(0, 1, Some(40_000)), cro("2"))
                .unwrap()
                .gas_limit,
            50_000
        );
        assert_eq!(max_native(cro("1.105"), q.gas_price), cro("1"));
        assert_eq!(max_native(cro("0.1"), q.gas_price), U256::ZERO);
        assert_eq!(
            check_transfer(
                n,
                &Asset::Native,
                ME,
                YOU,
                "1.0000000000000000001",
                cro("9")
            )
            .unwrap_err(),
            "Enter an amount like 1.25 with at most 18 decimal places."
        );
        assert_eq!(
            check_transfer(n, &Asset::Native, ME, YOU, "2", cro("1")).unwrap_err(),
            "You have 1 CRO."
        );
    }
    #[test]
    fn any_erc20_uses_its_own_contract_and_decimals() {
        let n = &CRONOS_MAINNET;
        let asset = Asset::Erc20(token(WCRO, "WCRO", 18));
        let r = check_transfer(n, &asset, ME, YOU, "0.5", cro("1")).unwrap();
        assert_eq!(checksum(&r.to), WCRO);
        assert_eq!(r.data, erc20_transfer(&r.recipient, cro("0.5")));
        let tx = build_transfer(n, &r, quote(1, 5_000, Some(40_000)), cro("1")).unwrap();
        assert_eq!((tx.gas_limit, tx.value), (50_000, U256::ZERO));
        assert_eq!(
            check_transfer(n, &asset, ME, YOU, "2", cro("1")).unwrap_err(),
            "You have 1 WCRO."
        );
        assert!(
            check_transfer(n, &asset, ME, WCRO, "0.1", cro("1"))
                .unwrap_err()
                .contains("own contract")
        );
        assert_eq!(asset.key(), WCRO.to_ascii_lowercase());
        assert_eq!(
            (Asset::Native.symbol(n), Asset::Native.decimals(n)),
            ("CRO".into(), 18)
        );
        assert_eq!(Asset::Usdc.contract(n).unwrap(), n.usdc.address);
        assert!(Asset::Native.contract(n).is_none());
    }
    fn abi_word(n: u64) -> Vec<u8> {
        U256::from(n).to_be_bytes().to_vec()
    }
    #[test]
    fn erc20_metadata_decodes_both_string_forms_and_refuses_junk() {
        let mut dynamic = abi_word(32);
        dynamic.extend(abi_word(4));
        let mut text = b"WCRO".to_vec();
        text.resize(32, 0);
        dynamic.extend(&text);
        assert_eq!(abi_string(&dynamic).unwrap(), "WCRO");
        // The older bytes32 form, zero-padded.
        assert_eq!(abi_string(&text).unwrap(), "WCRO");
        // A length past the end, a huge offset, or nothing at all.
        let mut long = abi_word(32);
        long.extend(abi_word(400));
        assert!(abi_string(&long).is_err());
        assert!(abi_string(&[0xff; 64]).is_err());
        assert!(abi_string(&[]).is_err());
        assert_eq!(abi_decimals(&abi_word(6)).unwrap(), 6);
        assert_eq!(abi_decimals(&abi_word(36)).unwrap(), 36);
        assert!(abi_decimals(&abi_word(37)).is_err());
        assert!(abi_decimals(&[0; 31]).is_err());
        assert_eq!(ERC20_SYMBOL, Keccak256::digest(b"symbol()")[..4]);
        assert_eq!(ERC20_NAME, Keccak256::digest(b"name()")[..4]);
        assert_eq!(ERC20_DECIMALS, Keccak256::digest(b"decimals()")[..4]);
    }
    #[test]
    fn saved_tokens_are_cleaned_and_lookalikes_flagged() {
        let n = &CRONOS_MAINNET;
        let messy = CustomToken {
            address: WCRO.to_ascii_lowercase(),
            symbol: " W\u{202E}CRO\n".into(),
            name: "Wrapped\u{200B}   CRO".into(),
            decimals: 18,
        };
        let clean = validate_token(n, &messy).unwrap();
        assert_eq!(
            (
                clean.address.as_str(),
                clean.symbol.as_str(),
                clean.name.as_str()
            ),
            (WCRO, "WCRO", "Wrapped CRO")
        );
        assert!(
            validate_token(n, &token(n.usdc.address, "USDC", 6))
                .unwrap_err()
                .contains("Choose USDC")
        );
        assert!(validate_token(n, &token(WCRO, " \u{200B} ", 18)).is_err());
        assert!(validate_token(n, &token(WCRO, "X", 37)).is_err());
        assert!(validate_token(n, &token("0x12", "X", 6)).is_err());
        assert_eq!(
            validate_token(n, &token(WCRO, &"Y".repeat(40), 6))
                .unwrap()
                .symbol
                .len(),
            MAX_SYMBOL_CHARS
        );
        for fake in ["USDC", "usdc.e", "U.S.D.C", "CRO", "tcro"] {
            assert!(imitates_builtin(n, &token(WCRO, fake, 6)), "{fake}");
        }
        assert!(imitates_builtin(
            &CRONOS_TESTNET,
            &token(WCRO, "devUSDC.e", 6)
        ));
        assert!(!imitates_builtin(n, &token(WCRO, "VVS", 18)));
        // Wrapped CRO is a real, different asset, not a disguise.
        assert!(!imitates_builtin(n, &token(WCRO, "WCRO", 18)));
    }
    #[test]
    fn settings_fall_back_to_usdc() {
        let mut s = WalletSettings::default();
        assert_eq!(s.network, "cronos-testnet");
        assert_eq!(s.asset_on(&CRONOS_TESTNET), Asset::Usdc);
        s.selected
            .insert("cronos-mainnet".into(), NATIVE_KEY.into());
        assert_eq!(s.asset_on(&CRONOS_MAINNET), Asset::Native);
        s.selected
            .insert("cronos-testnet".into(), WCRO.to_ascii_lowercase());
        assert_eq!(s.asset_on(&CRONOS_TESTNET), Asset::Usdc, "not saved there");
        s.tokens
            .insert("cronos-testnet".into(), vec![token(WCRO, "WCRO", 18)]);
        assert_eq!(
            s.asset_on(&CRONOS_TESTNET),
            Asset::Erc20(token(WCRO, "WCRO", 18))
        );
        assert!(s.tokens_on(&CRONOS_MAINNET).is_empty());
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(serde_json::from_value::<WalletSettings>(json).unwrap(), s);
    }
    #[test]
    fn u256_adds_and_subtracts_with_carries() {
        let a = U256::from(u128::MAX);
        assert_eq!(
            a.checked_add(U256::from(1u64)).unwrap().to_string(),
            "340282366920938463463374607431768211456"
        );
        assert!(U256::MAX.checked_add(U256::from(1u64)).is_none());
        assert_eq!(
            a.checked_add(U256::from(1u64))
                .unwrap()
                .checked_sub(U256::from(1u64)),
            Some(a)
        );
        assert_eq!(
            U256::from(1000u64).checked_sub(U256::from(1u64)),
            Some(U256::from(999u64))
        );
        assert!(U256::from(1u64).checked_sub(U256::from(2u64)).is_none());
    }
}
