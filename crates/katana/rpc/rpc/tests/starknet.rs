use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{address, Uint, U256};
use alloy::sol;
use cainome::cairo_serde::EthAddress;
use cainome::rs::abigen;
use dojo_test_utils::sequencer::{get_default_test_starknet_config, TestSequencer};
use dojo_world::utils::TransactionWaiter;
use katana_core::sequencer::SequencerConfig;
use katana_runner::{AnvilRunner, KatanaRunner, KatanaRunnerConfig};
use serde_json::json;
use starknet::accounts::{Account, Call, ConnectedAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::contract::legacy::LegacyContractClass;
use starknet::core::types::{
    BlockId, BlockTag, DeclareTransactionReceipt, FieldElement, MaybePendingTransactionReceipt,
    Transaction, TransactionFinalityStatus, TransactionReceipt,
};
use starknet::core::utils::{get_contract_address, get_selector_from_name};
use starknet::macros::felt;
use starknet::providers::Provider;
use tempfile::tempdir;

mod common;

const WAIT_TX_DELAY_MILLIS: u64 = 1000;

#[tokio::test(flavor = "multi_thread")]
async fn test_send_declare_and_deploy_contract() {
    let sequencer =
        TestSequencer::start(SequencerConfig::default(), get_default_test_starknet_config()).await;
    let account = sequencer.account();

    let path: PathBuf = PathBuf::from("tests/test_data/cairo1_contract.json");
    let (contract, compiled_class_hash) =
        common::prepare_contract_declaration_params(&path).unwrap();

    let class_hash = contract.class_hash();
    let res = account.declare(Arc::new(contract), compiled_class_hash).send().await.unwrap();

    // wait for the tx to be mined
    tokio::time::sleep(Duration::from_millis(WAIT_TX_DELAY_MILLIS)).await;

    let receipt = account.provider().get_transaction_receipt(res.transaction_hash).await.unwrap();

    match receipt {
        MaybePendingTransactionReceipt::Receipt(TransactionReceipt::Declare(
            DeclareTransactionReceipt { finality_status, .. },
        )) => {
            assert_eq!(finality_status, TransactionFinalityStatus::AcceptedOnL2);
        }
        _ => panic!("invalid tx receipt"),
    }

    assert!(account.provider().get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.is_ok());

    let constructor_calldata = vec![FieldElement::from(1_u32), FieldElement::from(2_u32)];

    let calldata = [
        vec![
            res.class_hash,                                 // class hash
            FieldElement::ZERO,                             // salt
            FieldElement::ZERO,                             // unique
            FieldElement::from(constructor_calldata.len()), // constructor calldata len
        ],
        constructor_calldata.clone(),
    ]
    .concat();

    let contract_address = get_contract_address(
        FieldElement::ZERO,
        res.class_hash,
        &constructor_calldata,
        FieldElement::ZERO,
    );

    account
        .execute(vec![Call {
            calldata,
            // devnet UDC address
            to: FieldElement::from_hex_be(
                "0x41a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf",
            )
            .unwrap(),
            selector: get_selector_from_name("deployContract").unwrap(),
        }])
        .send()
        .await
        .unwrap();

    // wait for the tx to be mined
    tokio::time::sleep(Duration::from_millis(WAIT_TX_DELAY_MILLIS)).await;

    assert_eq!(
        account
            .provider()
            .get_class_hash_at(BlockId::Tag(BlockTag::Latest), contract_address)
            .await
            .unwrap(),
        class_hash
    );

    sequencer.stop().expect("failed to stop sequencer");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_declare_and_deploy_legacy_contract() {
    let sequencer =
        TestSequencer::start(SequencerConfig::default(), get_default_test_starknet_config()).await;
    let account = sequencer.account();

    let path = PathBuf::from("tests/test_data/cairo0_contract.json");

    let legacy_contract: LegacyContractClass =
        serde_json::from_reader(fs::File::open(path).unwrap()).unwrap();
    let contract_class = Arc::new(legacy_contract);

    let class_hash = contract_class.class_hash().unwrap();
    let res = account.declare_legacy(contract_class).send().await.unwrap();
    // wait for the tx to be mined
    tokio::time::sleep(Duration::from_millis(WAIT_TX_DELAY_MILLIS)).await;

    let receipt = account.provider().get_transaction_receipt(res.transaction_hash).await.unwrap();

    match receipt {
        MaybePendingTransactionReceipt::Receipt(TransactionReceipt::Declare(
            DeclareTransactionReceipt { finality_status, .. },
        )) => {
            assert_eq!(finality_status, TransactionFinalityStatus::AcceptedOnL2);
        }
        _ => panic!("invalid tx receipt"),
    }

    assert!(account.provider().get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.is_ok());

    let constructor_calldata = vec![FieldElement::ONE];

    let calldata = [
        vec![
            res.class_hash,                                 // class hash
            FieldElement::ZERO,                             // salt
            FieldElement::ZERO,                             // unique
            FieldElement::from(constructor_calldata.len()), // constructor calldata len
        ],
        constructor_calldata.clone(),
    ]
    .concat();

    let contract_address = get_contract_address(
        FieldElement::ZERO,
        res.class_hash,
        &constructor_calldata.clone(),
        FieldElement::ZERO,
    );

    account
        .execute(vec![Call {
            calldata,
            // devnet UDC address
            to: FieldElement::from_hex_be(
                "0x41a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf",
            )
            .unwrap(),
            selector: get_selector_from_name("deployContract").unwrap(),
        }])
        .send()
        .await
        .unwrap();

    // wait for the tx to be mined
    tokio::time::sleep(Duration::from_millis(WAIT_TX_DELAY_MILLIS)).await;

    assert_eq!(
        account
            .provider()
            .get_class_hash_at(BlockId::Tag(BlockTag::Latest), contract_address)
            .await
            .unwrap(),
        class_hash
    );

    sequencer.stop().expect("failed to stop sequencer");
}

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StarknetContract,
    "tests/test_data/solidity/StarknetMessagingLocalCompiled.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    Contract1,
    "tests/test_data/solidity/Contract1Compiled.json"
);

abigen!(CairoMessagingContract, "crates/katana/rpc/rpc/tests/test_data/cairo_l1_msg_contract.json");

#[tokio::test(flavor = "multi_thread")]
async fn test_messaging_l1_l2() {
    // Prepare Anvil + Messaging Contracts
    let anvil_runner = AnvilRunner::new().await.unwrap();
    let anvil_provider = anvil_runner.provider();

    let contract_strk = StarknetContract::deploy(anvil_runner.provider()).await.unwrap();
    assert_eq!(contract_strk.address(), &address!("5fbdb2315678afecb367f032d93f642f64180aa3"));

    let contract_c1 = Contract1::deploy(anvil_provider, *contract_strk.address()).await.unwrap();
    assert_eq!(contract_c1.address(), &address!("e7f1725e7734ce288f8367e1bb143e90bb3f0512"));

    // Prepare Katana + Messaging Contract
    let messagin_config = json!({
        "chain": "ethereum",
        "rpc_url": anvil_runner.endpoint,
        "contract_address": contract_strk.address().to_string(),
        "sender_address": anvil_runner.address(),
        "private_key": anvil_runner.secret_key(),
        "interval": 2,
        "from_block": 0
    });
    let serialized_json = &messagin_config.to_string();

    let dir = tempdir().expect("Error creating temp dir");
    let file_path = dir.path().join("temp-anvil-messaging.json");

    // Write JSON string to a tempfile
    let mut file = File::create(&file_path).expect("Error creating temp file");
    file.write_all(serialized_json.as_bytes()).expect("Failed to write to file");

    let katana_runner = KatanaRunner::new_with_config(KatanaRunnerConfig {
        n_accounts: 2,
        disable_fee: false,
        block_time: None,
        port: None,
        program_name: None,
        run_name: None,
        messaging: Some(file_path.to_str().unwrap().to_string()),
    })
    .unwrap();
    let starknet_account = katana_runner.account(0);

    let path: PathBuf = PathBuf::from("tests/test_data/cairo_l1_msg_contract.json");
    let (contract, compiled_class_hash) =
        common::prepare_contract_declaration_params(&path).unwrap();

    let class_hash = contract.class_hash();
    let res =
        starknet_account.declare(Arc::new(contract), compiled_class_hash).send().await.unwrap();

    let receipt = TransactionWaiter::new(res.transaction_hash, starknet_account.provider())
        .with_tx_status(TransactionFinalityStatus::AcceptedOnL2)
        .await
        .expect("Invalid tx receipt");

    // Following 2 asserts are to make sure contract declaration went through and was processed
    // successfully
    assert_eq!(receipt.finality_status(), &TransactionFinalityStatus::AcceptedOnL2);

    assert!(
        starknet_account
            .provider()
            .get_class(BlockId::Tag(BlockTag::Latest), class_hash)
            .await
            .is_ok()
    );

    let contract_factory = ContractFactory::new(class_hash, &starknet_account);

    let transaction = contract_factory
        .deploy(vec![], FieldElement::ZERO, false)
        .send()
        .await
        .expect("Unable to deploy contract");

    // wait for the tx to be mined
    TransactionWaiter::new(transaction.transaction_hash, starknet_account.provider())
        .with_tx_status(TransactionFinalityStatus::AcceptedOnL2)
        .await
        .expect("Invalid tx receipt");

    let contract_address =
        get_contract_address(FieldElement::ZERO, res.class_hash, &[], FieldElement::ZERO);

    assert_eq!(
        contract_address,
        felt!("0x033d18fcfd3ae75ae4e8a275ce649220ed718b68dc53425b388fedcdbeab5097")
    );

    // Messaging between L1 -> L2
    let builder = contract_c1
        .sendMessage(
            U256::from_str("0x033d18fcfd3ae75ae4e8a275ce649220ed718b68dc53425b388fedcdbeab5097")
                .unwrap(),
            U256::from_str("0x005421de947699472df434466845d68528f221a52fce7ad2934c5dae2e1f1cdc")
                .unwrap(),
            vec![U256::from(123)],
        )
        .gas(12000000)
        .value(Uint::from(1));

    let receipt = builder
        .send()
        .await
        .expect("Error Await pending transaction")
        .get_receipt()
        .await
        .expect("Error getting transaction receipt");

    assert!(receipt.status());

    // wait for the tx to be mined (Using delay cause the transaction is sent from L1 and is
    // received in L2)
    tokio::time::sleep(Duration::from_millis(WAIT_TX_DELAY_MILLIS)).await;

    let tx = starknet_account
        .provider()
        .get_transaction_by_block_id_and_index(BlockId::Tag(BlockTag::Latest), 0)
        .await
        .unwrap();

    match tx {
        Transaction::L1Handler(ref l1_handler_transaction) => {
            let calldata = &l1_handler_transaction.calldata;

            assert_eq!(
                tx.transaction_hash(),
                &felt!("0x00c33cc113afc56bc878034908472770cb13eda6ad8ad91feb25fd4e5c9196a0")
            );

            assert_eq!(FieldElement::to_string(&calldata[1]), "123")
        }
        _ => {
            panic!("Error, No L1handler transaction")
        }
    }

    // Messaging between L2 -> L1
    let cairo_messaging_contract = CairoMessagingContract::new(contract_address, &starknet_account);
    let tx = cairo_messaging_contract
        .send_message_value(
            &EthAddress::from(
                FieldElement::from_str(contract_c1.address().to_string().as_str()).unwrap(),
            ),
            &FieldElement::from(2u8),
        )
        .send()
        .await
        .expect("Call to send_message_value failed");

    TransactionWaiter::new(tx.transaction_hash, starknet_account.provider())
        .with_tx_status(TransactionFinalityStatus::AcceptedOnL2)
        .await
        .expect("Invalid tx receipt");

    let builder = contract_c1
        .consumeMessage(
            U256::from_str(contract_address.to_string().as_str()).unwrap(),
            vec![U256::from(2)],
        )
        .value(Uint::from(1))
        .gas(12000000)
        .nonce(4);

    // Wait for the message to reach L1
    tokio::time::sleep(Duration::from_millis(8000)).await;

    let tx_receipt = builder.send().await.unwrap().get_receipt().await.unwrap();
    assert!(tx_receipt.status());
}
