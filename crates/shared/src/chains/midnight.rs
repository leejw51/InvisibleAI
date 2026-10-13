//! Midnight unshielded (Night) addresses: BIP-32 secp256k1 at
//! `m/44'/2400'/0'/0/<index>`, the BIP-340 x-only public key, then bech32m of
//! its SHA-256 under `mn_addr[_<network>]`.
//!
//! BIP-340 negates about half of all secret keys to get an even y, which
//! matters for exporting the scalar but not here: negation keeps x, so the
//! x-only key is the compressed key without its parity byte.
use bech32::{Bech32m, Hrp};
use sha2::{Digest, Sha256};

pub fn path(index: u32) -> String {
    format!("m/44'/2400'/0'/0/{index}")
}

/// The raw 32-byte user address: SHA-256 of the x-only public key.
pub fn address_bytes(seed: &[u8], index: u32) -> Result<[u8; 32], String> {
    let public = super::secp256k1_public_key(seed, &path(index))?;
    Ok(Sha256::digest(&public[1..]).into())
}

/// `network` is the HRP segment (`preview`, `devnet`, `test`, `undeployed`);
/// mainnet (`None`) omits it, giving `mn_addr1…`.
pub fn address(seed: &[u8], index: u32, network: Option<&str>) -> Result<String, String> {
    encode(&address_bytes(seed, index)?, network)
}

pub fn encode(address: &[u8; 32], network: Option<&str>) -> Result<String, String> {
    let hrp = match network {
        Some(name) => format!("mn_addr_{name}"),
        None => "mn_addr".to_string(),
    };
    let hrp = Hrp::parse(&hrp).map_err(|_| format!("`{hrp}` is not a valid address prefix"))?;
    bech32::encode::<Bech32m>(hrp, address).map_err(|_| "Could not encode Midnight address".into())
}
