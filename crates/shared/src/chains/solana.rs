//! Solana: SLIP-0010 over ed25519 at `m/44'/501'/<index>'/0'`.
//! Ed25519 has no public-key derivation, so SLIP-0010 is hardened-only; the
//! path is fixed here rather than parsed, so an unhardened level cannot occur.
//! The address is the base58 of the raw public key.
use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::Zeroizing;

const HARDENED: u32 = 0x8000_0000;

pub fn path(index: u32) -> String {
    format!("m/44'/501'/{index}'/0'")
}

/// The base58 address at `m/44'/501'/<index>'/0'` of a BIP-39 seed.
pub fn address(seed: &[u8], index: u32) -> String {
    let key = derive(seed, &[44, 501, index, 0]);
    let (keypair, public) = cryptoxide::ed25519::keypair(&key);
    drop(Zeroizing::new(keypair));
    bs58::encode(public).into_string()
}

/// The 32-byte ed25519 seed at a path of hardened indices.
pub fn derive(seed: &[u8], path: &[u32]) -> Zeroizing<[u8; 32]> {
    // key ‖ chain code
    let mut node = hmac(b"ed25519 seed", &[seed]);
    for &index in path {
        let index = (index | HARDENED).to_be_bytes();
        node = hmac(&node[32..], &[&[0], &node[..32], &index]);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&node[..32]);
    key
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    for part in parts {
        mac.update(part);
    }
    Zeroizing::new(mac.finalize().into_bytes().into())
}

/// The SPL Token program and the Associated Token Account program.
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

fn key_bytes(address: &str) -> Result<[u8; 32], String> {
    bs58::decode(address)
        .into_vec()
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| format!("{address} is not a Solana address."))
}
/// The account that holds `mint` for `owner`: a program-derived address, the
/// highest bump whose hash is *not* an ed25519 point (a point could have a
/// private key). Reading this one account is what the public mainnet node
/// allows; scanning an owner's token accounts it refuses.
pub fn associated_token_address(owner: &str, mint: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let (owner, mint) = (key_bytes(owner)?, key_bytes(mint)?);
    let (token, program) = (
        key_bytes(TOKEN_PROGRAM)?,
        key_bytes(ASSOCIATED_TOKEN_PROGRAM)?,
    );
    // Counting down from 255 is the definition, not an optimisation: scanning
    // the other way would find a different, unreachable account.
    for bump in (0..=u8::MAX).rev() {
        let hash: [u8; 32] = Sha256::new()
            .chain_update(owner)
            .chain_update(token)
            .chain_update(mint)
            .chain_update([bump])
            .chain_update(program)
            .chain_update(b"ProgramDerivedAddress")
            .finalize()
            .into();
        if cryptoxide::curve25519::Ge::from_bytes(&hash).is_none() {
            return Ok(bs58::encode(hash).into_string());
        }
    }
    Err("No token account address exists for that owner and mint.".into())
}

#[cfg(test)]
mod ata_tests {
    use super::*;
    /// The Causewaybay Wallet's vectors, each read back from mainnet with
    /// `getAccountInfo` as the owner and mint it was derived from.
    #[test]
    fn associated_addresses_match_the_accounts_mainnet_holds() {
        const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        const USDT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
        for (owner, mint, expected) in [
            (
                "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
                USDC,
                "FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B",
            ),
            (
                "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
                USDT,
                "TB5FCqbNsnuLQgEjUuPaT9qtVPTT4U1A8rvi7qzEj2M",
            ),
            (
                "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9",
                USDC,
                "FzbcyEZ9m8xjtergWgWDq7mfPoHEbboBF791B6cTpzbq",
            ),
        ] {
            assert_eq!(
                associated_token_address(owner, mint).unwrap(),
                expected,
                "{owner}"
            );
        }
        assert!(associated_token_address("not-base58-0OIl", USDC).is_err());
    }
}
