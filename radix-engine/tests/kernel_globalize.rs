use radix_engine::engine::RuntimeError;
use radix_engine::ledger::TypedInMemorySubstateStore;
use scrypto::core::NetworkDefinition;
use scrypto::engine::types::RENodeId;
use scrypto::prelude::*;
use scrypto_unit::*;
use transaction::builder::ManifestBuilder;

#[test]
fn should_not_be_able_to_globalize_key_value_store() {
    // Arrange
    let mut store = TypedInMemorySubstateStore::with_bootstrap();
    let mut test_runner = TestRunner::new(true, &mut store);
    let package_address = test_runner.extract_and_publish_package("kernel");

    // Act
    let manifest = ManifestBuilder::new(NetworkDefinition::local_simulator())
        .lock_fee(10.into(), SYSTEM_COMPONENT)
        .call_function(package_address, "Globalize", "globalize_kv_store", args!())
        .build();
    let receipt = test_runner.execute_manifest(manifest, vec![]);

    // Assert
    receipt.expect_failure(|e| {
        matches!(
            e,
            RuntimeError::RENodeGlobalizeTypeNotAllowed(RENodeId::KeyValueStore(..))
        )
    });
}

#[test]
fn should_not_be_able_to_globalize_bucket() {
    // Arrange
    let mut store = TypedInMemorySubstateStore::with_bootstrap();
    let mut test_runner = TestRunner::new(true, &mut store);
    let package_address = test_runner.extract_and_publish_package("kernel");

    // Act
    let manifest = ManifestBuilder::new(NetworkDefinition::local_simulator())
        .lock_fee(10.into(), SYSTEM_COMPONENT)
        .call_function(package_address, "Globalize", "globalize_bucket", args!())
        .build();
    let receipt = test_runner.execute_manifest(manifest, vec![]);

    // Assert
    receipt.expect_failure(|e| {
        matches!(
            e,
            RuntimeError::RENodeGlobalizeTypeNotAllowed(RENodeId::Bucket(..))
        )
    });
}

#[test]
fn should_not_be_able_to_globalize_proof() {
    // Arrange
    let mut store = TypedInMemorySubstateStore::with_bootstrap();
    let mut test_runner = TestRunner::new(true, &mut store);
    let package_address = test_runner.extract_and_publish_package("kernel");

    // Act
    let manifest = ManifestBuilder::new(NetworkDefinition::local_simulator())
        .lock_fee(10.into(), SYSTEM_COMPONENT)
        .call_function(package_address, "Globalize", "globalize_proof", args!())
        .build();
    let receipt = test_runner.execute_manifest(manifest, vec![]);

    // Assert
    receipt.expect_failure(|e| {
        matches!(
            e,
            RuntimeError::RENodeGlobalizeTypeNotAllowed(RENodeId::Proof(..))
        )
    });
}

#[test]
fn should_not_be_able_to_globalize_vault() {
    // Arrange
    let mut store = TypedInMemorySubstateStore::with_bootstrap();
    let mut test_runner = TestRunner::new(true, &mut store);
    let package_address = test_runner.extract_and_publish_package("kernel");

    // Act
    let manifest = ManifestBuilder::new(NetworkDefinition::local_simulator())
        .lock_fee(10.into(), SYSTEM_COMPONENT)
        .call_function(package_address, "Globalize", "globalize_vault", args!())
        .build();
    let receipt = test_runner.execute_manifest(manifest, vec![]);

    // Assert
    receipt.expect_failure(|e| {
        matches!(
            e,
            RuntimeError::RENodeGlobalizeTypeNotAllowed(RENodeId::Vault(..))
        )
    });
}
