//! Cronos EVM wallet core, ported from the Causewaybay Wallet (rustcli/core):
//! the network and stablecoin rows, decimal amounts, EIP-55, ERC-20 calldata
//! and legacy EIP-155 transactions. No network access and no persistence: the
//! browser asks the node, this module only encodes, signs and decodes.
use k256::ecdsa::SigningKey;
pub use u256::U256;
use sha3::{Digest, Keccak256};

/// The only two networks the wallet harness serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Network {
    pub key: &'static str,
    pub name: &'static str,
    pub chain_id: u64,
    pub rpc: &'static str,
    pub explorer: &'static str,
    pub native: &'static str,
    /// The fee ceiling in wei: a hostile or broken endpoint's gas price can make
    /// a fee almost anything, so nothing past this is signed.
    pub max_fee_wei: u128,
    pub usdc: Token,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub symbol: &'static str,
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
    max_fee_wei: 20_000_000_000_000_000_000, // 20 CRO
    usdc: Token {
        symbol: "USDC",
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
    max_fee_wei: 20_000_000_000_000_000_000, // 20 TCRO
    usdc: Token {
        symbol: "devUSDC.e",
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
        || ![int_part, frac_part].iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
        || frac_part.len() > decimals as usize
    {
        return Err(invalid());
    }
    let mut digits = String::from(if int_part.is_empty() { "0" } else { int_part });
    digits.push_str(frac_part);
    digits.extend(std::iter::repeat_n('0', decimals as usize - frac_part.len()));
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
    let mixed = raw.bytes().any(|b| b.is_ascii_lowercase()) && raw.bytes().any(|b| b.is_ascii_uppercase());
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

mod u256 {
    //! Just enough 256-bit unsigned arithmetic for balances, amounts and fees:
    //! big-endian bytes, so the derived ordering is numeric ordering.
    use std::fmt;
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct U256([u8; 32]);
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
        pub fn saturating_mul_u64(&self, m: u64) -> Self {
            self.mul_add(m, 0).unwrap_or(Self([0xff; 32]))
        }
        fn div_rem_small(&self, d: u32) -> (Self, u32) {
            let mut out = [0u8; 32];
            let mut rem = 0u32;
            for i in 0..32 {
                let v = (rem << 8) | self.0[i] as u32;
                out[i] = (v / d) as u8;
                rem = v % d;
            }
            (Self(out), rem)
        }
        pub fn from_str_radix(s: &str, radix: u32) -> Result<Self, ()> {
            if s.is_empty() || !(radix == 10 || radix == 16) {
                return Err(());
            }
            s.chars().try_fold(Self::ZERO, |acc, c| {
                let digit = c.to_digit(radix).ok_or(())?;
                acc.mul_add(radix as u64, digit as u64).ok_or(())
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
        type Error = ();
        fn try_from(v: U256) -> Result<u64, ()> {
            if v.0[..24].iter().any(|b| *b != 0) {
                return Err(());
            }
            Ok(u64::from_be_bytes(v.0[24..].try_into().unwrap()))
        }
    }
    impl std::str::FromStr for U256 {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, ()> {
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
                let (q, r) = n.div_rem_small(10);
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
/// Gas limit for an ERC-20 transfer: the node's estimate plus 25 % headroom.
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
        assert_eq!(format!("0x{}", hex::encode(data)), v["data"].as_str().unwrap());
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
            assert_eq!(parse_address(&lower.to_uppercase().replacen("0X", "0x", 1)).unwrap(), bytes);
            // Flip the case of the first letter: still hex, no longer the checksum.
            let i = good.char_indices().skip(2).find(|(_, c)| c.is_ascii_alphabetic());
            if let Some((i, c)) = i {
                let mut bad = good.to_string();
                let flipped = if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() };
                bad.replace_range(i..i + 1, &flipped.to_string());
                if bad.chars().skip(2).any(|c| c.is_ascii_uppercase()) && bad.chars().skip(2).any(|c| c.is_ascii_lowercase()) {
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
            assert_eq!(parse_units(&format_units(value, decimals), decimals).unwrap(), value);
        }
        for row in v["invalid"].as_array().unwrap() {
            let decimals = row["decimals"].as_u64().unwrap() as u8;
            assert!(parse_units(row["amount"].as_str().unwrap(), decimals).is_err(), "{row}");
        }
        assert_eq!(format_units(U256::from(1_500_000u64), 6), "1.5");
        assert_eq!(format_units(U256::from(5u64), 6), "0.000005");
        assert_eq!(format_units(U256::ZERO, 6), "0");
    }
    #[test]
    fn quantities_and_networks() {
        assert_eq!(parse_quantity("0x").unwrap(), U256::ZERO);
        assert_eq!(parse_quantity("0x0f4240").unwrap(), U256::from(1_000_000u64));
        assert!(parse_quantity("12").is_err());
        assert_eq!(network("cronos-mainnet").unwrap().chain_id, 25);
        assert_eq!(network("cronos-testnet").unwrap().chain_id, 338);
        assert!(network("ethereum").is_none());
        for n in NETWORKS {
            assert_eq!(parse_address(n.usdc.address).map(|a| checksum(&a)).unwrap(), n.usdc.address);
            assert_eq!(n.usdc.decimals, 6);
        }
        assert_eq!(with_headroom(40_000), 50_000);
    }
    #[test]
    fn u256_parses_formats_and_bounds() {
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(max.parse::<U256>().unwrap().to_string(), max);
        assert!("115792089237316195423570985008687907853269984665640564039457584007913129639936".parse::<U256>().is_err());
        assert_eq!(U256::from_str_radix("ff", 16).unwrap(), U256::from(255u64));
        assert!(U256::from_str_radix("fg", 16).is_err());
        assert!(U256::from(2u64) > U256::from(1u64));
        assert!(U256::from(u128::MAX) > U256::from(u64::MAX));
        assert_eq!(u64::try_from(U256::from(7u64)), Ok(7));
        assert!(u64::try_from(U256::from(u128::MAX)).is_err());
        assert_eq!(U256::from(3u64).saturating_mul_u64(5), U256::from(15u64));
        assert_eq!(max.parse::<U256>().unwrap().saturating_mul_u64(2).to_string(), max);
    }
}
