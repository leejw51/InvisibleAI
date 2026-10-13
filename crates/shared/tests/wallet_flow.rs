//! Integration: the wallet's whole send path through the crate's public API —
//! phrase → key → checks → transaction → signature — then the raw bytes are
//! decoded independently and the signer is recovered from the signature, the
//! way a Cronos node would. No network.
use causewaybay_shared::evm::{self, CRONOS_MAINNET, CRONOS_TESTNET, NETWORKS, Quote, U256};
use causewaybay_shared::identity::Identity;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};

// Public BIP-39 / Hardhat vectors; never use these wallets for assets.
const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const RECIPIENT: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

/// A minimal RLP decoder, written separately from the encoder under test.
#[derive(Debug, PartialEq)]
enum Item {
    Bytes(Vec<u8>),
    List(Vec<Item>),
}
fn decode(bytes: &[u8]) -> (Item, usize) {
    let be = |b: &[u8]| b.iter().fold(0usize, |n, x| n * 256 + *x as usize);
    let p = bytes[0];
    match p {
        0x00..=0x7f => (Item::Bytes(vec![p]), 1),
        0x80..=0xb7 => {
            let n = (p - 0x80) as usize;
            (Item::Bytes(bytes[1..1 + n].to_vec()), 1 + n)
        }
        0xb8..=0xbf => {
            let l = (p - 0xb7) as usize;
            let n = be(&bytes[1..1 + l]);
            (Item::Bytes(bytes[1 + l..1 + l + n].to_vec()), 1 + l + n)
        }
        _ => {
            let (l, n) = if p <= 0xf7 {
                (0, (p - 0xc0) as usize)
            } else {
                let l = (p - 0xf7) as usize;
                (l, be(&bytes[1..1 + l]))
            };
            let (mut at, end, mut items) = (1 + l, 1 + l + n, Vec::new());
            while at < end {
                let (item, used) = decode(&bytes[at..]);
                items.push(item);
                at += used;
            }
            assert_eq!(at, end, "list length mismatch");
            (Item::List(items), end)
        }
    }
}
fn bytes(item: &Item) -> &[u8] {
    match item {
        Item::Bytes(b) => b,
        Item::List(_) => panic!("expected bytes"),
    }
}
fn uint(item: &Item) -> u128 {
    let b = bytes(item);
    assert!(b.first() != Some(&0), "RLP integers are minimal");
    b.iter().fold(0u128, |n, x| n * 256 + *x as u128)
}
fn address_of(key: &VerifyingKey) -> String {
    let point = key.to_encoded_point(false);
    format!(
        "0x{}",
        hex::encode(&Keccak256::digest(&point.as_bytes()[1..])[12..])
    )
}

#[test]
fn phrase_to_signed_usdc_transfer_recovers_to_the_sender_on_both_networks() {
    let identity = Identity::from_mnemonic(PHRASE).unwrap();
    let from = identity.address();
    // Logging in by key yields the very same wallet.
    let by_key = Identity::from_private_key(&identity.private_key_hex()).unwrap();
    assert_eq!(by_key.address(), from);

    for network in NETWORKS {
        let request = evm::check_transfer(
            &network,
            &evm::Asset::Usdc,
            &from,
            RECIPIENT,
            "12.34",
            evm::parse_units("100", 6).unwrap(),
        )
        .unwrap();
        let quote = Quote {
            nonce: U256::from(42u64),
            gas_price: evm::parse_units("5000", 9).unwrap(),
            estimate: Some(U256::from(51_000u64)),
        };
        let tx = evm::build_transfer(
            &network,
            &request,
            quote,
            evm::parse_units("10", 18).unwrap(),
        )
        .unwrap();
        let signed = tx.sign(by_key.signing_key()).unwrap();

        // The hash a node reports is keccak of exactly these bytes.
        assert_eq!(
            signed.hash_hex(),
            format!("0x{}", hex::encode(Keccak256::digest(&signed.raw)))
        );

        let (item, used) = decode(&signed.raw);
        assert_eq!(used, signed.raw.len());
        let Item::List(fields) = item else {
            panic!("a transaction is a list")
        };
        assert_eq!(fields.len(), 9);
        assert_eq!(uint(&fields[0]), 42);
        assert_eq!(uint(&fields[1]), 5_000_000_000_000);
        assert_eq!(uint(&fields[2]), 63_750);
        assert_eq!(
            evm::checksum(bytes(&fields[3]).try_into().unwrap()),
            network.usdc.address
        );
        assert!(
            bytes(&fields[4]).is_empty(),
            "no CRO moves with a token transfer"
        );
        let data = bytes(&fields[5]);
        assert_eq!(&data[..4], &[0xa9, 0x05, 0x9c, 0xbb]);
        assert_eq!(&data[16..36], &evm::parse_address(RECIPIENT).unwrap());
        assert_eq!(
            U256::from_be_slice(&data[36..68]),
            U256::from(12_340_000u64)
        );

        // EIP-155: v carries the chain id; recover the signer like a node does.
        let v = uint(&fields[6]) as u64;
        let recovery = v
            .checked_sub(network.chain_id * 2 + 35)
            .expect("v below the EIP-155 range");
        assert!(recovery <= 1);
        let mut rs = [0u8; 64];
        rs[32 - bytes(&fields[7]).len()..32].copy_from_slice(bytes(&fields[7]));
        rs[64 - bytes(&fields[8]).len()..].copy_from_slice(bytes(&fields[8]));
        let signature = Signature::from_slice(&rs).unwrap();
        assert!(signature.normalize_s().is_none(), "low-s signatures only");
        let key = VerifyingKey::recover_from_prehash(
            &tx.signing_hash(),
            &signature,
            RecoveryId::from_byte(recovery as u8).unwrap(),
        )
        .unwrap();
        assert_eq!(address_of(&key), from);

        // The same transaction on the other chain is a different, non-replayable one.
        let mut other = tx.clone();
        other.chain_id = if network == CRONOS_MAINNET {
            CRONOS_TESTNET.chain_id
        } else {
            CRONOS_MAINNET.chain_id
        };
        assert_ne!(other.sign(by_key.signing_key()).unwrap().raw, signed.raw);
    }
}

#[test]
fn a_refused_transfer_never_reaches_signing() {
    let identity = Identity::from_mnemonic(PHRASE).unwrap();
    let from = identity.address();
    let balance = evm::parse_units("1", 6).unwrap();
    for (to, amount) in [
        (from.as_str(), "0.5"),
        (RECIPIENT, "2"),
        (RECIPIENT, "0"),
        ("0xF39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "0.5"),
    ] {
        assert!(
            evm::check_transfer(
                &CRONOS_TESTNET,
                &evm::Asset::Usdc,
                &from,
                to,
                amount,
                balance
            )
            .is_err(),
            "{to} {amount}"
        );
    }
    let request = evm::check_transfer(
        &CRONOS_TESTNET,
        &evm::Asset::Usdc,
        &from,
        RECIPIENT,
        "0.5",
        balance,
    )
    .unwrap();
    let greedy = Quote {
        nonce: U256::ZERO,
        gas_price: evm::parse_units("1", 18).unwrap(),
        estimate: None,
    };
    assert!(
        evm::build_transfer(
            &CRONOS_TESTNET,
            &request,
            greedy,
            evm::parse_units("1000000", 18).unwrap()
        )
        .is_err()
    );
}

#[test]
fn a_cro_transfer_carries_the_amount_as_value_with_no_calldata() {
    let identity = Identity::from_mnemonic(PHRASE).unwrap();
    let from = identity.address();
    let request = evm::check_transfer(
        &CRONOS_MAINNET,
        &evm::Asset::Native,
        &from,
        RECIPIENT,
        "1.5",
        evm::parse_units("3", 18).unwrap(),
    )
    .unwrap();
    let quote = Quote {
        nonce: U256::from(3u64),
        gas_price: evm::parse_units("5000", 9).unwrap(),
        estimate: Some(U256::from(21_000u64)),
    };
    let tx = evm::build_transfer(
        &CRONOS_MAINNET,
        &request,
        quote,
        evm::parse_units("3", 18).unwrap(),
    )
    .unwrap();
    let signed = tx.sign(identity.signing_key()).unwrap();
    let Item::List(fields) = decode(&signed.raw).0 else {
        panic!("a transaction is a list")
    };
    assert_eq!(uint(&fields[2]), 21_000);
    assert_eq!(bytes(&fields[3]), &evm::parse_address(RECIPIENT).unwrap());
    assert_eq!(uint(&fields[4]), 1_500_000_000_000_000_000);
    assert!(
        bytes(&fields[5]).is_empty(),
        "plain CRO carries no calldata"
    );
    let recovery = uint(&fields[6]) as u64 - (CRONOS_MAINNET.chain_id * 2 + 35);
    let mut rs = [0u8; 64];
    rs[32 - bytes(&fields[7]).len()..32].copy_from_slice(bytes(&fields[7]));
    rs[64 - bytes(&fields[8]).len()..].copy_from_slice(bytes(&fields[8]));
    let key = VerifyingKey::recover_from_prehash(
        &tx.signing_hash(),
        &Signature::from_slice(&rs).unwrap(),
        RecoveryId::from_byte(recovery as u8).unwrap(),
    )
    .unwrap();
    assert_eq!(address_of(&key), from);
}
