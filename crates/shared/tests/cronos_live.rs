//! Live integration against the public Cronos nodes. Ignored by default:
//! `cargo test -p causewaybay-shared --test cronos_live -- --ignored`
//! (or `make test-wallet-live`). Read-only, except one transaction signed by a
//! fresh random key that can never hold funds — the node must refuse it for
//! insufficient funds, which it only says after accepting the encoding, the
//! EIP-155 chain id and the signature.
use causewaybay_shared::evm::{self, NETWORKS, Network, Quote, U256};
use causewaybay_shared::identity::Identity;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use std::time::{SystemTime, UNIX_EPOCH};

fn rpc(network: &Network, method: &str, params: Value) -> Value {
    let response: Value = reqwest::blocking::Client::new()
        .post(network.rpc)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .unwrap_or_else(|e| panic!("{} {method}: {e}", network.rpc))
        .json()
        .unwrap();
    response
}
fn result(network: &Network, method: &str, params: Value) -> Value {
    let response = rpc(network, method, params);
    assert!(response.get("error").is_none(), "{method}: {response}");
    response["result"].clone()
}
fn call(network: &Network, selector: &str) -> Vec<u8> {
    let out = result(
        network,
        "eth_call",
        json!([{"to":network.usdc.address,"data":selector},"latest"]),
    );
    hex::decode(out.as_str().unwrap().trim_start_matches("0x")).unwrap()
}
/// A key nobody has ever funded: 32 bytes of hashed wall-clock nanoseconds.
fn throwaway() -> Identity {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seed = Keccak256::digest(format!("invisible-ai wallet live test {nanos}"));
    Identity::from_private_key(&hex::encode(seed)).unwrap()
}

#[test]
#[ignore = "talks to the public Cronos nodes"]
fn each_network_is_the_chain_and_token_the_wallet_claims() {
    for network in NETWORKS {
        let chain =
            evm::parse_quantity(result(&network, "eth_chainId", json!([])).as_str().unwrap())
                .unwrap();
        assert_eq!(chain, U256::from(network.chain_id), "{}", network.name);
        // decimals() and symbol() answer from the contract itself.
        assert_eq!(
            *call(&network, "0x313ce567").last().unwrap(),
            network.usdc.decimals,
            "{}",
            network.name
        );
        let symbol = call(&network, "0x95d89b41");
        let len = U256::from_be_slice(&symbol[32..64]);
        let len = u64::try_from(len).unwrap() as usize;
        assert_eq!(
            std::str::from_utf8(&symbol[64..64 + len]).unwrap(),
            network.usdc.symbol,
            "{}",
            network.name
        );
        // balanceOf answers a 32-byte word for any holder.
        let owner = evm::parse_address("0x9858effd232b4033e47d90003d41ec34ecaeda94").unwrap();
        let data = format!("0x{}", hex::encode(evm::erc20_balance_of(&owner)));
        let word = result(
            &network,
            "eth_call",
            json!([{"to":network.usdc.address,"data":data},"latest"]),
        );
        assert_eq!(word.as_str().unwrap().len(), 66, "{}", network.name);
        // A gas price is quoted and sits well under the wallet's fee ceiling.
        let gas = evm::parse_quantity(
            result(&network, "eth_gasPrice", json!([]))
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert!(!gas.is_zero());
        assert!(
            gas.saturating_mul_u64(evm::FALLBACK_GAS) < U256::from(network.max_fee_wei),
            "{}: gas price {gas}",
            network.name
        );
    }
}

#[test]
#[ignore = "talks to the public Cronos nodes"]
fn a_signed_transfer_is_accepted_as_well_formed_and_refused_only_for_funds() {
    let identity = throwaway();
    let from = identity.address();
    for network in NETWORKS {
        let balance = evm::parse_quantity(
            result(&network, "eth_getBalance", json!([from, "latest"]))
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert!(
            balance.is_zero(),
            "the throwaway key unexpectedly holds funds on {}",
            network.name
        );
        let nonce = evm::parse_quantity(
            result(
                &network,
                "eth_getTransactionCount",
                json!([from, "pending"]),
            )
            .as_str()
            .unwrap(),
        )
        .unwrap();
        let gas_price = evm::parse_quantity(
            result(&network, "eth_gasPrice", json!([]))
                .as_str()
                .unwrap(),
        )
        .unwrap();
        // Plan as if the balances were there, so the transaction is the real shape.
        let request = evm::check_transfer(
            &network,
            &evm::Asset::Usdc,
            &from,
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "0.01",
            evm::parse_units("1", 6).unwrap(),
        )
        .unwrap();
        let tx = evm::build_transfer(
            &network,
            &request,
            Quote {
                nonce,
                gas_price,
                estimate: None,
            },
            U256::from(network.max_fee_wei),
        )
        .unwrap();
        let signed = tx.sign(identity.signing_key()).unwrap();
        let response = rpc(
            &network,
            "eth_sendRawTransaction",
            json!([signed.raw_hex()]),
        );
        let message = response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        assert!(
            message.contains("insufficient funds") || message.contains("insufficient fee"),
            "{}: expected a funds refusal, got {response}",
            network.name
        );
        for wrong in ["invalid chain id", "invalid sender", "rlp", "signature"] {
            assert!(!message.contains(wrong), "{}: {response}", network.name);
        }
    }
}
