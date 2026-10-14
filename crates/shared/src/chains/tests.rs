use super::*;
use serde_json::Value;

fn multichain() -> Value {
    serde_json::from_str(include_str!("../../testvectors/multichain.json")).unwrap()
}
fn ecash_vectors() -> Value {
    serde_json::from_str(include_str!("../../testvectors/ecash.json")).unwrap()
}
fn accounts<'a>(vectors: &'a Value, chain: &str) -> &'a [Value] {
    let accounts = vectors[chain]["accounts"].as_array().unwrap();
    assert!(!accounts.is_empty(), "{chain} has no vectors");
    accounts
}
fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("missing {field}"))
}

#[test]
fn solana_matches_web3js_on_every_network() {
    let v = multichain();
    let mnemonic = text(&v, "mnemonic");
    for (index, expected) in accounts(&v, "solana").iter().enumerate() {
        let index = index as u32;
        assert_eq!(solana::path(index), text(expected, "path"));
        for network in ["solana-devnet", "solana-testnet", "solana-mainnet"] {
            assert_eq!(
                address(mnemonic, index, network).unwrap(),
                text(expected, "address"),
                "{network} index {index}"
            );
        }
    }
}

#[test]
fn cardano_matches_serialization_lib() {
    let v = multichain();
    let mnemonic = text(&v, "mnemonic");
    for (index, expected) in accounts(&v, "cardano").iter().enumerate() {
        let index = index as u32;
        assert_eq!(cardano::path(index), text(expected, "path"));
        for network in ["cardano-preprod", "cardano-preview"] {
            assert_eq!(
                address(mnemonic, index, network).unwrap(),
                text(expected, "base_addr_testnet"),
                "{network} index {index}"
            );
        }
        assert_eq!(
            address(mnemonic, index, "cardano-mainnet").unwrap(),
            text(expected, "base_addr_mainnet"),
            "mainnet index {index}"
        );
    }
}

#[test]
fn midnight_matches_wallet_sdk_hd() {
    let v = multichain();
    let seed = parse_mnemonic(text(&v, "mnemonic")).unwrap().to_seed("");
    for (index, expected) in accounts(&v, "midnight").iter().enumerate() {
        let index = index as u32;
        assert_eq!(midnight::path(index), text(expected, "path"));
        assert_eq!(
            hex::encode(midnight::address_bytes(&seed, index).unwrap()),
            text(expected, "address_hex")
        );
        for (network, field) in [
            (Some("undeployed"), "addr_undeployed"),
            (Some("test"), "addr_testnet"),
            (None, "addr_mainnet"),
        ] {
            assert_eq!(
                midnight::address(&seed, index, network).unwrap(),
                text(expected, field),
                "{field} index {index}"
            );
        }
    }
}

/// The vectors carry no preview/devnet prefix, so check the shipped keys
/// carry the vector's payload under the prefix core uses for them.
#[test]
fn midnight_network_keys_use_core_prefixes() {
    let v = multichain();
    let mnemonic = text(&v, "mnemonic");
    for (index, expected) in accounts(&v, "midnight").iter().enumerate() {
        for (network, hrp) in [
            ("midnight-preview", "mn_addr_preview"),
            ("midnight-devnet", "mn_addr_devnet"),
        ] {
            let encoded = address(mnemonic, index as u32, network).unwrap();
            let (decoded_hrp, payload) = bech32::decode(&encoded).unwrap();
            assert_eq!(decoded_hrp.as_str(), hrp);
            assert_eq!(hex::encode(payload), text(expected, "address_hex"));
            assert!(
                bech32::primitives::decode::CheckedHrpstring::new::<bech32::Bech32m>(&encoded)
                    .is_ok()
            );
        }
    }
}

#[test]
fn ecash_matches_bip44_and_cashaddr() {
    let v = ecash_vectors();
    let mnemonic = text(&v, "mnemonic");
    let accounts = v["accounts"].as_array().unwrap();
    assert!(!accounts.is_empty());
    for expected in accounts {
        let index = expected["index"].as_u64().unwrap() as u32;
        assert_eq!(ecash::path(index), text(expected, "path"));
        assert_eq!(
            address(mnemonic, index, "ecash-testnet").unwrap(),
            text(expected, "address"),
            "testnet index {index}"
        );
        assert_eq!(
            address(mnemonic, index, "ecash-mainnet").unwrap(),
            text(expected, "address_mainnet"),
            "mainnet index {index}"
        );
    }
}

#[test]
fn cashaddr_matches_the_published_vectors() {
    for vector in ecash_vectors()["spec_vectors"].as_array().unwrap() {
        let version = match text(vector, "kind") {
            "p2pkh" => ecash::P2PKH,
            "p2sh" => ecash::P2SH,
            kind => panic!("unexpected kind {kind}"),
        };
        let mut hash = [0u8; 20];
        hex::decode_to_slice(text(vector, "payload"), &mut hash).unwrap();
        assert_eq!(
            ecash::cashaddr(text(vector, "prefix"), version, &hash),
            text(vector, "address")
        );
    }
}

#[test]
fn every_listed_network_derives() {
    let mnemonic = text(&multichain(), "mnemonic").to_string();
    for network in NETWORKS {
        assert!(address(&mnemonic, 0, network).is_ok(), "{network}");
    }
}

#[test]
fn an_invalid_mnemonic_is_refused() {
    let words = "abandon ".repeat(12);
    for phrase in ["", "not a mnemonic", words.trim()] {
        for network in NETWORKS {
            assert!(address(phrase, 0, network).is_err(), "{network}");
        }
    }
}

#[test]
fn unknown_networks_and_indices_are_refused() {
    let mnemonic = text(&multichain(), "mnemonic").to_string();
    for network in [
        "",
        "cronos-testnet",
        "midnight-mainnet",
        "solana",
        "Solana-Mainnet",
    ] {
        assert!(address(&mnemonic, 0, network).is_err(), "{network}");
    }
    assert!(address(&mnemonic, 1 << 31, "solana-mainnet").is_err());
}

#[test]
fn whitespace_in_the_phrase_does_not_change_the_wallet() {
    let v = multichain();
    let mnemonic = text(&v, "mnemonic");
    let spaced = format!("  {}\n", mnemonic.replace(' ', "   "));
    assert_eq!(
        address(&spaced, 0, "ecash-mainnet").unwrap(),
        address(mnemonic, 0, "ecash-mainnet").unwrap()
    );
}
