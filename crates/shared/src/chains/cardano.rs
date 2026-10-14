//! Cardano: Icarus (CIP-3) master key from the BIP-39 *entropy*, then
//! BIP32-Ed25519 along CIP-1852. A base address joins the payment key at
//! `m/1852'/1815'/0'/0/<index>` with the account's staking key at `…/2/0`.
use bech32::{Bech32, Hrp};
use cryptoxide::hashing::blake2b::Blake2b;
use ed25519_bip32::{DerivationScheme, XPrv};
use hmac::Hmac;
use sha2::Sha512;
use zeroize::Zeroizing;

const HARDENED: u32 = 0x8000_0000;

pub fn path(index: u32) -> String {
    format!("m/1852'/1815'/0'/0/{index}")
}

/// The base address (`addr1…` on mainnet, `addr_test1…` on preprod/preview,
/// which share network id 0) for an empty passphrase.
pub fn address(entropy: &[u8], index: u32, mainnet: bool) -> Result<String, String> {
    let account = root(entropy, b"")?
        .derive(DerivationScheme::V2, 1852 | HARDENED)
        .derive(DerivationScheme::V2, 1815 | HARDENED)
        .derive(DerivationScheme::V2, HARDENED);
    let payment = account
        .derive(DerivationScheme::V2, 0)
        .derive(DerivationScheme::V2, index);
    let stake = account
        .derive(DerivationScheme::V2, 2)
        .derive(DerivationScheme::V2, 0);
    let mut bytes = Vec::with_capacity(57);
    // Header: type 0000 (base) ‖ network id.
    bytes.push(u8::from(mainnet));
    bytes.extend_from_slice(&key_hash(&payment));
    bytes.extend_from_slice(&key_hash(&stake));
    let hrp = Hrp::parse_unchecked(if mainnet { "addr" } else { "addr_test" });
    bech32::encode::<Bech32>(hrp, &bytes).map_err(|_| "Could not encode Cardano address".into())
}

/// PBKDF2-HMAC-SHA512 with the passphrase as *password* and the entropy as
/// *salt*. It reads backwards and is what every Cardano wallet does.
fn root(entropy: &[u8], passphrase: &[u8]) -> Result<XPrv, String> {
    let mut out = Zeroizing::new([0u8; 96]);
    pbkdf2::pbkdf2::<Hmac<Sha512>>(passphrase, entropy, 4096, &mut out[..])
        .map_err(|_| "Could not derive Cardano root key".to_string())?;
    // Clamp for ed25519, and clear bit 5 of byte 31 so `8·zL + kL` never overflows.
    out[0] &= 0b1111_1000;
    out[31] &= 0b0001_1111;
    out[31] |= 0b0100_0000;
    XPrv::from_bytes_verified(*out).map_err(|_| "Invalid Cardano root key".to_string())
}

/// blake2b-224 of the raw ed25519 public key.
fn key_hash(key: &XPrv) -> [u8; 28] {
    let mut out = [0u8; 28];
    Blake2b::<224>::new()
        .update(key.public().public_key_bytes())
        .finalize_at(&mut out);
    out
}
