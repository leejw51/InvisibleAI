//! Standard mnemonic-derived Ethereum wallet identity.
//! BIP-39 → m/44'/60'/0'/0/index → Ethereum personal_sign. No network or persistence.
use bip32::{DerivationPath, XPrv};
use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use sha3::{Digest, Keccak256};
use zeroize::Zeroizing;

pub struct Identity(SigningKey);
impl Identity {
    pub fn from_mnemonic(phrase: &str) -> Result<Self, String> {
        Self::from_mnemonic_at(phrase, 0)
    }
    pub fn from_mnemonic_at(phrase: &str, index: u32) -> Result<Self, String> {
        validate_wallet_index(index)?;
        let normalized = Zeroizing::new(phrase.split_whitespace().collect::<Vec<_>>().join(" "));
        let mnemonic = bip39::Mnemonic::parse(normalized.as_str())
            .map_err(|_| "Enter a valid 12, 15, 18, 21 or 24 word recovery phrase.".to_string())?;
        let seed = Zeroizing::new(mnemonic.to_seed(""));
        let path: DerivationPath = format!("m/44'/60'/0'/0/{index}").parse().unwrap();
        let key = XPrv::derive_from_path(seed.as_ref(), &path)
            .map_err(|_| "Could not derive wallet identity".to_string())?;
        Ok(Self(key.private_key().clone()))
    }
    pub fn address(&self) -> String {
        address(self.0.verifying_key())
    }
    pub fn sign(&self, message: &str) -> Result<String, String> {
        let (sig, recovery) = self
            .0
            .sign_prehash_recoverable(&digest(message))
            .map_err(|_| "Could not sign login challenge".to_string())?;
        Ok(format!(
            "0x{}{:02x}",
            hex::encode(sig.to_bytes()),
            recovery.to_byte() + 27
        ))
    }
}
pub fn validate_wallet_index(index: u32) -> Result<(), String> {
    if index >= (1 << 31) {
        return Err("Wallet index must be a whole number from 0 to 2147483647.".into());
    }
    Ok(())
}
/// The caller must supply entropy from the operating system/browser CSPRNG.
pub fn mnemonic_from_entropy(entropy: &[u8; 16]) -> Result<Zeroizing<String>, String> {
    bip39::Mnemonic::from_entropy(entropy)
        .map(|mnemonic| Zeroizing::new(mnemonic.to_string()))
        .map_err(|_| "Could not generate recovery phrase".into())
}
pub fn normalize_address(value: &str) -> Result<String, String> {
    if value.len() != 42
        || !value.starts_with("0x")
        || !value[2..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Expected a 0x-prefixed Ethereum wallet address".into());
    }
    Ok(value.to_ascii_lowercase())
}
/// Stable display name; the normalized full address remains the identity key.
pub fn username(value: &str) -> Result<String, String> {
    let address = normalize_address(value)?;
    let hash = Keccak256::digest(address.as_bytes());
    const ADJECTIVES: [&str; 16] = [
        "Amber", "Bright", "Calm", "Clear", "Coral", "Dawn", "Gentle", "Golden", "Jade", "Lunar",
        "Quiet", "Silver", "Solar", "Swift", "Warm", "Wild",
    ];
    const NOUNS: [&str; 16] = [
        "Bay", "Cedar", "Crane", "Finch", "Harbour", "Heron", "Island", "Lotus", "Maple", "Otter",
        "Panda", "Pearl", "River", "Sparrow", "Willow", "Wren",
    ];
    Ok(format!(
        "{} {} · {}",
        ADJECTIVES[(hash[0] & 15) as usize],
        NOUNS[(hash[1] & 15) as usize],
        &address[2..10]
    ))
}
fn address(key: &VerifyingKey) -> String {
    let point = key.to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}
fn digest(message: &str) -> [u8; 32] {
    let mut hash = Keccak256::new();
    hash.update(b"\x19Ethereum Signed Message:\n");
    hash.update(message.len().to_string());
    hash.update(message.as_bytes());
    hash.finalize().into()
}
pub fn recover(message: &str, signature: &str) -> Result<String, String> {
    let invalid = || "Invalid login signature".to_string();
    let raw = signature.strip_prefix("0x").ok_or_else(invalid)?;
    if raw.len() != 130 {
        return Err(invalid());
    }
    let bytes = hex::decode(raw).map_err(|_| invalid())?;
    let v = match bytes[64] {
        27 | 28 => bytes[64] - 27,
        0 | 1 => bytes[64],
        _ => return Err(invalid()),
    };
    let id = RecoveryId::from_byte(v).ok_or_else(invalid)?;
    let sig = Signature::from_slice(&bytes[..64]).map_err(|_| invalid())?;
    if sig.normalize_s().is_some() {
        return Err(invalid());
    }
    let key =
        VerifyingKey::recover_from_prehash(&digest(message), &sig, id).map_err(|_| invalid())?;
    Ok(address(&key))
}

#[cfg(test)]
mod tests {
    use super::*;
    // Public BIP-39 test fixture; never use this wallet for assets or real access.
    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    #[test]
    fn matches_public_wallet_vector_and_signatures() {
        let identity = Identity::from_mnemonic(PHRASE).unwrap();
        assert_eq!(
            identity.address(),
            "0x9858effd232b4033e47d90003d41ec34ecaeda94"
        );
        let message = "Causewaybay Bot login — 香港";
        let sig = identity.sign(message).unwrap();
        assert_eq!(recover(message, &sig).unwrap(), identity.address());
        assert_ne!(
            recover("different challenge", &sig).unwrap(),
            identity.address()
        );
        assert!(recover(message, "0x1234").is_err());
        assert!(Identity::from_mnemonic("abandon abandon").is_err());
    }
    #[test]
    fn indexed_wallets_and_generated_mnemonics_match_public_vectors() {
        let phrase = mnemonic_from_entropy(&[0; 16]).unwrap();
        assert_eq!(phrase.as_str(), PHRASE);
        assert_eq!(
            Identity::from_mnemonic_at(PHRASE, 0).unwrap().address(),
            Identity::from_mnemonic(PHRASE).unwrap().address()
        );
        let test = "test test test test test test test test test test test junk";
        assert_eq!(
            Identity::from_mnemonic_at(test, 1).unwrap().address(),
            "0x70997970c51812dc3a010c7d01b50e0d17dc79c8"
        );
        assert_eq!(
            Identity::from_mnemonic_at(test, 2).unwrap().address(),
            "0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc"
        );
        assert!(Identity::from_mnemonic_at(test, 2147483647).is_ok());
        assert!(Identity::from_mnemonic_at(test, 2147483648).is_err());
        assert!(Identity::from_mnemonic_at(test, u32::MAX).is_err());
    }
    #[test]
    fn verifies_external_signature_vector() {
        let message = "FruitNation Encryption Key Derivation\n\nAddress: 0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266\nPurpose: End-to-end encryption only";
        let signature = "0xe98d833febe631493b5589273c82b3a66df264a47ca2c02754c0f9dffaeceaa601306b1edf3328bfe7f46b747def4fd6b4f4b400dcc76955a6fb33a47ac6ba531b";
        assert_eq!(
            recover(message, signature).unwrap(),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
    }
}
