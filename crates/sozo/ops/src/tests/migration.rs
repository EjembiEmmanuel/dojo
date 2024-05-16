use std::str;

use cainome::cairo_serde::ContractAddress;
use camino::Utf8Path;
use dojo_lang::compiler::{BASE_DIR, MANIFESTS_DIR, OVERLAYS_DIR};
use dojo_test_utils::migration::prepare_migration_with_world_and_seed;
use dojo_world::contracts::{WorldContract, WorldContractReader};
use dojo_world::manifest::{
    BaseManifest, DeploymentManifest, OverlayManifest, WORLD_CONTRACT_NAME,
};
use dojo_world::metadata::{
    dojo_metadata_from_workspace, ArtifactMetadata, DojoMetadata, Uri, WorldMetadata,
    IPFS_CLIENT_URL, IPFS_PASSWORD, IPFS_USERNAME,
};
use dojo_world::migration::strategy::prepare_for_migration;
use dojo_world::migration::world::WorldDiff;
use dojo_world::migration::TxnConfig;
use futures::TryStreamExt;
use ipfs_api_backend_hyper::{HyperBackend, IpfsApi, IpfsClient, TryFromUri};
use katana_runner::{KatanaRunner, KatanaRunnerConfig};
use starknet::core::types::{BlockId, BlockTag};
use starknet::core::utils::{
    cairo_short_string_to_felt, get_selector_from_name, parse_cairo_short_string,
};
use starknet::macros::felt;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet_crypto::FieldElement;

use super::setup;
use crate::migration::{auto_authorize, execute_strategy, upload_metadata};
use crate::utils::get_contract_address_from_reader;

#[tokio::test(flavor = "multi_thread")]
async fn migrate_with_auto_mine() {
    let config = setup::load_config();
    let ws = setup::setup_ws(&config);

    let migration = setup::setup_migration(&config).unwrap();

    let sequencer = KatanaRunner::new().expect("Fail to start runner");

    let mut account = sequencer.account(0);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    execute_strategy(&ws, &migration, &account, TxnConfig::default()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_with_block_time() {
    let config = setup::load_config();
    let ws = setup::setup_ws(&config);

    let migration = setup::setup_migration(&config).unwrap();

    let sequencer = KatanaRunner::new_with_config(KatanaRunnerConfig {
        block_time: Some(1000),
        ..Default::default()
    })
    .expect("Fail to start runner");

    let mut account = sequencer.account(0);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    execute_strategy(&ws, &migration, &account, TxnConfig::default()).await.unwrap();
}

#[should_panic]
#[tokio::test(flavor = "multi_thread")]
async fn migrate_with_small_fee_multiplier_will_fail() {
    let config = setup::load_config();
    let ws = setup::setup_ws(&config);

    let migration = setup::setup_migration(&config).unwrap();

    let sequencer = KatanaRunner::new_with_config(KatanaRunnerConfig {
        disable_fee: true,
        ..Default::default()
    })
    .expect("Fail to start runner");

    let account = sequencer.account(0);

    assert!(
        execute_strategy(
            &ws,
            &migration,
            &account,
            TxnConfig { fee_estimate_multiplier: Some(0.2f64), ..Default::default() },
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn migration_from_remote() {
    let config = setup::load_config();
    let ws = setup::setup_ws(&config);

    let base = config.manifest_path().parent().unwrap();
    let target_dir = format!("{}/target/dev", base);

    let sequencer = KatanaRunner::new().expect("Failed to start runner.");

    let account = sequencer.account(0);

    let profile_name = ws.current_profile().unwrap().to_string();

    let manifest = BaseManifest::load_from_path(
        &base.to_path_buf().join(MANIFESTS_DIR).join(&profile_name).join(BASE_DIR),
    )
    .unwrap();

    let world = WorldDiff::compute(manifest, None);

    let migration = prepare_for_migration(
        None,
        felt!("0x12345"),
        &Utf8Path::new(&target_dir).to_path_buf(),
        world,
    )
    .unwrap();

    execute_strategy(&ws, &migration, &account, TxnConfig::default()).await.unwrap();

    let local_manifest = BaseManifest::load_from_path(
        &base.to_path_buf().join(MANIFESTS_DIR).join(&profile_name).join(BASE_DIR),
    )
    .unwrap();

    let remote_manifest = DeploymentManifest::load_from_remote(
        JsonRpcClient::new(HttpTransport::new(sequencer.url())),
        migration.world_address().unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(local_manifest.world.inner.class_hash, remote_manifest.world.inner.class_hash);
    assert_eq!(local_manifest.models.len(), remote_manifest.models.len());
}

// TODO: remove ignore once IPFS node is running.
#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn migrate_with_metadata() {
    let config = setup::load_config();
    let ws = setup::setup_ws(&config);

    let migration = setup::setup_migration(&config).unwrap();

    let sequencer = KatanaRunner::new().expect("Fail to start runner");

    let mut account = sequencer.account(0);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    let output = execute_strategy(&ws, &migration, &account, TxnConfig::init_wait()).await.unwrap();

    let res = upload_metadata(&ws, &account, output.clone(), TxnConfig::init_wait()).await;
    assert!(res.is_ok());

    let provider = sequencer.provider();
    let world_reader = WorldContractReader::new(output.world_address, &provider);

    let client = IpfsClient::from_str(IPFS_CLIENT_URL)
        .unwrap_or_else(|_| panic!("Unable to initialize the IPFS Client"))
        .with_credentials(IPFS_USERNAME, IPFS_PASSWORD);

    let dojo_metadata = dojo_metadata_from_workspace(&ws);

    // check world metadata
    let resource = world_reader.metadata(&FieldElement::ZERO).call().await.unwrap();
    let element_name = WORLD_CONTRACT_NAME.to_string();

    let full_uri = get_and_check_metadata_uri(&element_name, &resource.metadata_uri);
    let resource_bytes = get_ipfs_resource_data(&client, &element_name, &full_uri).await;

    let metadata = resource_bytes_to_world_metadata(&resource_bytes, &element_name);

    assert_eq!(metadata.name, dojo_metadata.world.name, "");
    assert_eq!(metadata.description, dojo_metadata.world.description, "");
    assert_eq!(metadata.cover_uri, dojo_metadata.world.cover_uri, "");
    assert_eq!(metadata.icon_uri, dojo_metadata.world.icon_uri, "");
    assert_eq!(metadata.website, dojo_metadata.world.website, "");
    assert_eq!(metadata.socials, dojo_metadata.world.socials, "");

    check_artifact_fields(
        &client,
        &metadata.artifacts,
        &dojo_metadata.world.artifacts,
        &element_name,
    )
    .await;

    // check model metadata
    for m in migration.models {
        let selector = get_selector_from_name(&m.diff.name).unwrap();
        check_artifact_metadata(&client, &world_reader, selector, &m.diff.name, &dojo_metadata)
            .await;
    }

    // check contract metadata
    for c in migration.contracts {
        let contract_address =
            get_contract_address_from_reader(&world_reader, c.diff.name.clone()).await.unwrap();
        check_artifact_metadata(
            &client,
            &world_reader,
            contract_address,
            &c.diff.name,
            &dojo_metadata,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_with_auto_authorize() {
    let config = setup::load_config();
    let ws = setup::setup_ws(&config);

    let migration = setup::setup_migration(&config).unwrap();

    let manifest_base = config.manifest_path().parent().unwrap();
    let mut manifest =
        BaseManifest::load_from_path(&manifest_base.join(MANIFESTS_DIR).join("dev").join(BASE_DIR))
            .unwrap();

    let overlay_manifest = OverlayManifest::load_from_path(
        &manifest_base.join(MANIFESTS_DIR).join("dev").join(OVERLAYS_DIR),
    )
    .unwrap();

    manifest.merge(overlay_manifest);

    let sequencer = KatanaRunner::new().expect("Fail to start runner");

    let mut account = sequencer.account(0);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    let txn_config = TxnConfig {
        // make sure to test the assumption after transaction has been confirmed
        wait: true,
        ..Default::default()
    };

    let output = execute_strategy(&ws, &migration, &account, txn_config).await.unwrap();

    let world_address = migration.world_address().expect("must be present");
    let world = WorldContract::new(world_address, account);

    let res = auto_authorize(&ws, &world, &txn_config, &manifest, &output).await;
    assert!(res.is_ok());

    let provider = sequencer.provider();
    let world_reader = WorldContractReader::new(output.world_address, &provider);

    // check contract metadata
    for c in migration.contracts {
        let contract_address =
            get_contract_address_from_reader(&world_reader, c.diff.name.clone()).await.unwrap();

        let contract = manifest.contracts.iter().find(|a| a.name == c.diff.name).unwrap();

        for model in &contract.inner.writes {
            let model = cairo_short_string_to_felt(model).unwrap();
            let contract_address = ContractAddress(contract_address);
            let is_writer = world_reader.is_writer(&model, &contract_address).call().await.unwrap();
            assert!(is_writer);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn migration_with_mismatching_world_address_and_seed() {
    let config = setup::load_config();

    let base_dir = config.manifest_path().parent().unwrap().to_path_buf();
    let target_dir = base_dir.join("target").join("dev");

    let result = prepare_migration_with_world_and_seed(
        base_dir,
        target_dir,
        Some(felt!("0x1")),
        "sozo_test",
    );

    assert!(result.is_err());

    let error_message = result.unwrap_err().to_string();

    assert_eq!(
        error_message,
        "Calculated world address doesn't match provided world address.\nIf you are deploying \
         with custom seed make sure `world_address` is correctly configured (or not set) \
         `Scarb.toml`"
    );
}

/// Get the hash from a IPFS URI
///
/// # Arguments
///
/// * `uri` - a full IPFS URI
///
/// # Returns
///
/// A [`String`] containing the hash from the URI.
fn get_hash_from_uri(uri: &str) -> String {
    let hash = match uri.strip_prefix("ipfs://") {
        Some(s) => s.to_string(),
        None => uri.to_owned(),
    };
    match hash.strip_suffix('/') {
        Some(s) => s.to_string(),
        None => hash,
    }
}

/// Check a metadata field which refers to a file.
///
/// # Arguments
///
/// * `client` - a IPFS client.
/// * `uri` - the IPFS URI of the abi field.
/// * `expected_uri` - the URI of the expected file.
/// * `field_name` - the field name.
/// * `element_name` - the fully qualified name of the element linked to this field.
async fn check_file_field(
    client: &HyperBackend,
    uri: &Uri,
    expected_uri: &Uri,
    field_name: String,
    element_name: &String,
) {
    if let Uri::Ipfs(uri) = uri {
        let resource_data = get_ipfs_resource_data(client, element_name, uri).await;
        assert!(
            !resource_data.is_empty(),
            "{field_name} IPFS artifact for {} is empty",
            element_name
        );

        if let Uri::File(f) = expected_uri {
            let file_content = std::fs::read_to_string(f).unwrap();
            let resource_content = std::str::from_utf8(&resource_data).unwrap_or_else(|_| {
                panic!(
                    "Unable to stringify resource data for field '{}' of {}",
                    field_name, element_name
                )
            });

            assert!(
                file_content.eq(&resource_content),
                "local '{field_name}' content differs from the one uploaded on IPFS for {}",
                element_name
            );
        } else {
            panic!(
                "The field '{field_name}' of {} is not a file (Should never happen !)",
                element_name
            );
        }
    } else {
        panic!("The '{field_name}' field is not an IPFS artifact for {}", element_name);
    }
}

/// Convert resource bytes to a ArtifactMetadata object.
///
/// # Arguments
///
/// * `raw_data` - resource data as bytes.
/// * `element_name` - name of the element linked to this resource.
///
/// # Returns
///
/// A [`ArtifactMetadata`] object.
fn resource_bytes_to_metadata(raw_data: &[u8], element_name: &String) -> ArtifactMetadata {
    let data = std::str::from_utf8(raw_data)
        .unwrap_or_else(|_| panic!("Unable to stringify raw metadata for {}", element_name));
    serde_json::from_str(data)
        .unwrap_or_else(|_| panic!("Unable to deserialize metadata for {}", element_name))
}

/// Convert resource bytes to a WorldMetadata object.
///
/// # Arguments
///
/// * `raw_data` - resource data as bytes.
/// * `element_name` - name of the element linked to this resource.
///
/// # Returns
///
/// A [`WorldMetadata`] object.
fn resource_bytes_to_world_metadata(raw_data: &[u8], element_name: &String) -> WorldMetadata {
    let data = std::str::from_utf8(raw_data)
        .unwrap_or_else(|_| panic!("Unable to stringify raw metadata for {}", element_name));
    serde_json::from_str(data)
        .unwrap_or_else(|_| panic!("Unable to deserialize metadata for {}", element_name))
}

/// Read the content of a resource identified by its IPFS URI.
///
/// # Arguments
///
/// * `client` - a IPFS client.
/// * `element_name` - the name of the element (model or contract) linked to this artifact.
/// * `uri` - the IPFS resource URI.
///
/// # Returns
///
/// A [`Vec<u8>`] containing the resource content as bytes.
async fn get_ipfs_resource_data(
    client: &HyperBackend,
    element_name: &String,
    uri: &String,
) -> Vec<u8> {
    let hash = get_hash_from_uri(uri);

    let res = client.cat(&hash).map_ok(|chunk| chunk.to_vec()).try_concat().await;
    assert!(res.is_ok(), "Unable to read the IPFS artifact {} for {}", uri, element_name);

    res.unwrap()
}

/// Check the validity of artifact metadata fields.
///
/// # Arguments
///
/// * `client` - a IPFS client.
/// * `metadata` - the metadata to check.
/// * `expected_metadata` - the metadata values coming from local Dojo metadata.
/// * `element_name` - the name of the element linked to this metadata.
async fn check_artifact_fields(
    client: &HyperBackend,
    metadata: &ArtifactMetadata,
    expected_metadata: &ArtifactMetadata,
    element_name: &String,
) {
    assert!(metadata.abi.is_some(), "'abi' field not set for {}", element_name);
    let abi = metadata.abi.as_ref().unwrap();
    let expected_abi = expected_metadata.abi.as_ref().unwrap();
    check_file_field(client, abi, expected_abi, "abi".to_string(), element_name).await;

    assert!(metadata.source.is_some(), "'source' field not set for {}", element_name);
    let source = metadata.source.as_ref().unwrap();
    let expected_source = expected_metadata.source.as_ref().unwrap();
    check_file_field(client, source, expected_source, "source".to_string(), element_name).await;
}

/// Check the validity of a IPFS artifact metadata.
///
/// # Arguments
///
/// * `client` - a IPFS client.
/// * `element_name` - the fully qualified name of the element linked to the artifact.
/// * `uri` - the full metadata URI.
/// * `expected_metadata` - the expected metadata values coming from local Dojo metadata.
async fn check_ipfs_metadata(
    client: &HyperBackend,
    element_name: &String,
    uri: &String,
    expected_metadata: &ArtifactMetadata,
) {
    let resource_bytes = get_ipfs_resource_data(client, element_name, uri).await;
    let metadata = resource_bytes_to_metadata(&resource_bytes, element_name);

    check_artifact_fields(client, &metadata, expected_metadata, element_name).await;
}

/// Rebuild the full metadata URI from an array of 3 FieldElement.
///
/// # Arguments
///
/// * `element_name` - name of the element (model or contract) linked to the metadata URI.
/// * `uri` - uri as an array of 3 FieldElement.
///
/// # Returns
///
/// A [`String`] containing the full metadata URI.
fn get_and_check_metadata_uri(element_name: &String, uri: &Vec<FieldElement>) -> String {
    assert!(uri.len() == 3, "bad metadata URI length for {} ({:#?})", element_name, uri);

    let mut i = 0;
    let mut full_uri = "".to_string();

    while i < uri.len() && uri[i] != FieldElement::ZERO {
        let uri_str = parse_cairo_short_string(&uri[i]);
        assert!(
            uri_str.is_ok(),
            "unable to parse the part {} of the metadata URI for {}",
            i + 1,
            element_name
        );

        full_uri = format!("{}{}", full_uri, uri_str.unwrap());

        i += 1;
    }

    assert!(!full_uri.is_empty(), "metadata URI is empty for {}", element_name);

    assert!(
        full_uri.starts_with("ipfs://"),
        "metadata URI for {} is not an IPFS artifact",
        element_name
    );

    full_uri
}

/// Check an artifact metadata read from the resource registry against its value
/// in the local Dojo metadata.
///
/// # Arguments
///
/// * `client` - a IPFS client.
/// * `world_reader` - a world reader object.
/// * `resource_id` - the resource ID in the resource registry.
/// * `element_name` - the fully qualified name of the element linked to this metadata.
/// * `dojo_metadata` - local Dojo metadata.
async fn check_artifact_metadata<P: starknet::providers::Provider + Sync>(
    client: &HyperBackend,
    world_reader: &WorldContractReader<P>,
    resource_id: FieldElement,
    element_name: &String,
    dojo_metadata: &DojoMetadata,
) {
    let resource = world_reader.metadata(&resource_id).call().await.unwrap();

    let expected_artifact = dojo_metadata.artifacts.get(element_name);
    assert!(
        expected_artifact.is_some(),
        "Unable to find local artifact metadata for {}",
        element_name
    );
    let expected_artifact = expected_artifact.unwrap();

    let full_uri = get_and_check_metadata_uri(element_name, &resource.metadata_uri);
    check_ipfs_metadata(client, element_name, &full_uri, expected_artifact).await;
}