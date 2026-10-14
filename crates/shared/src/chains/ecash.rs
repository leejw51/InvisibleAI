//! eCash: BIP-32 secp256k1 at `m/44'/1899'/0'/0/<index>` (coin type 1899 on
//! both networks), P2PKH of hash160 of the *compressed* public key — the
//! uncompressed form hashes to a different, valid, unfunded address — in
//! CashAddr under `ecash:` or `ectest:`.
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
pub const P2PKH: u8 = 0x00;
pub const P2SH: u8 = 0x08;

pub fn path(index: u32) -> String {
    format!("m/44'/1899'/0'/0/{index}")
}

pub fn address(seed: &[u8], index: u32, mainnet: bool) -> Result<String, String> {
    let public = super::secp256k1_public_key(seed, &path(index))?;
    let hash: [u8; 20] = Ripemd160::digest(Sha256::digest(public)).into();
    Ok(cashaddr(
        if mainnet { "ecash" } else { "ectest" },
        P2PKH,
        &hash,
    ))
}

/// `prefix:body`. The prefix is folded into the checksum, so `ecash:` and
/// `ectest:` forms of one hash do not decode as each other.
pub fn cashaddr(prefix: &str, version: u8, hash: &[u8; 20]) -> String {
    let mut payload = Vec::with_capacity(21);
    payload.push(version);
    payload.extend_from_slice(hash);
    let body = to_base32(&payload);

    let mut checksum_input: Vec<u8> = prefix.bytes().map(|c| c & 0x1f).collect();
    checksum_input.push(0);
    checksum_input.extend_from_slice(&body);
    checksum_input.extend_from_slice(&[0; 8]);
    let checksum = polymod(&checksum_input);

    let mut out = format!("{prefix}:");
    out.extend(body.iter().map(|&d| CHARSET[usize::from(d)] as char));
    out.extend((0..8).map(|i| CHARSET[((checksum >> (5 * (7 - i))) & 0x1f) as usize] as char));
    out
}

fn to_base32(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 8 / 5 + 1);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &byte in data {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(((acc >> bits) & 0x1f) as u8);
        }
    }
    if bits > 0 {
        out.push(((acc << (5 - bits)) & 0x1f) as u8);
    }
    out
}

/// CashAddr's 40-bit BCH checksum.
fn polymod(values: &[u8]) -> u64 {
    const GENERATORS: [u64; 5] = [
        0x98_f2bc_8e61,
        0x79_b76d_99e2,
        0xf3_3e5f_b3c4,
        0xae_2eab_e2a8,
        0x1e_4f43_e470,
    ];
    let mut c: u64 = 1;
    for &value in values {
        let c0 = c >> 35;
        c = ((c & 0x07_ffff_ffff) << 5) ^ u64::from(value);
        for (bit, generator) in GENERATORS.iter().enumerate() {
            if c0 & (1 << bit) != 0 {
                c ^= generator;
            }
        }
    }
    c ^ 1
}
