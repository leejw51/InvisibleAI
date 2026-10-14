//! Receive addresses for the non-EVM chains, derived from the wallet mnemonic.
//! Derivation only: no signing, transactions or network access.
//!
//! Each chain derives differently and the differences are load-bearing: Cardano
//! starts from the BIP-39 *entropy*, the others from the seed. A wrong turn
//! yields a plausible, valid, unfunded address rather than an error.
pub mod cardano;
pub mod ecash;
pub mod midnight;
pub mod solana;
#[cfg(test)]
mod tests;

use crate::identity::validate_wallet_index;
use zeroize::Zeroizing;

/// Every network key [`address`] derives for.
pub const NETWORKS: &[&str] = &[
    "solana-devnet",
    "solana-testnet",
    "solana-mainnet",
    "cardano-preprod",
    "cardano-preview",
    "cardano-mainnet",
    "midnight-preview",
    "midnight-devnet",
    "ecash-testnet",
    "ecash-mainnet",
];

/// The receive address for `network_key` at wallet `index`, derived from the
/// mnemonic. Err(String) for an unknown or non-derivable network key.
pub fn address(mnemonic: &str, index: u32, network_key: &str) -> Result<String, String> {
    validate_wallet_index(index)?;
    if !NETWORKS.contains(&network_key) {
        return Err(format!(
            "No receive address can be derived for network `{network_key}`."
        ));
    }
    let mnemonic = parse_mnemonic(mnemonic)?;
    if network_key.starts_with("cardano-") {
        let entropy = Zeroizing::new(mnemonic.to_entropy());
        return cardano::address(&entropy, index, network_key == "cardano-mainnet");
    }
    let seed = Zeroizing::new(mnemonic.to_seed(""));
    match network_key {
        "solana-devnet" | "solana-testnet" | "solana-mainnet" => Ok(solana::address(&*seed, index)),
        // Core names the HRP segment after the network key: `mn_addr_preview1…`.
        "midnight-preview" => midnight::address(&*seed, index, Some("preview")),
        "midnight-devnet" => midnight::address(&*seed, index, Some("devnet")),
        "ecash-testnet" => ecash::address(&*seed, index, false),
        "ecash-mainnet" => ecash::address(&*seed, index, true),
        _ => unreachable!("checked against NETWORKS"),
    }
}

/// Parsed exactly as [`crate::identity::Identity::from_mnemonic_at`] does, so
/// a phrase accepted for login derives the same wallet here.
pub fn parse_mnemonic(phrase: &str) -> Result<bip39::Mnemonic, String> {
    let normalized = Zeroizing::new(phrase.split_whitespace().collect::<Vec<_>>().join(" "));
    bip39::Mnemonic::parse(normalized.as_str())
        .map_err(|_| "Enter a valid 12, 15, 18, 21 or 24 word recovery phrase.".to_string())
}

/// BIP-32 over secp256k1 (Midnight, eCash): the 33-byte compressed public key.
fn secp256k1_public_key(seed: &[u8], path: &str) -> Result<[u8; 33], String> {
    let path: bip32::DerivationPath = path.parse().map_err(|_| "Invalid derivation path")?;
    let key = bip32::XPrv::derive_from_path(seed, &path)
        .map_err(|_| "Could not derive wallet key".to_string())?;
    Ok(key.public_key().to_bytes())
}
