#[rustfmt::skip]
pub mod test_runner;

use crate::test_runner::TestRunner;
use radix_engine::ledger::*;
use radix_engine::model::extract_package;
use scrypto::call_data;
use scrypto::prelude::*;
use transaction::builder::ManifestBuilder;
use transaction::signing::EcdsaPrivateKey;

#[test]
fn create_non_fungible_mutable() {
    // Arrange
    let mut test_runner = TestRunner::new(true);
    let (_, _, account) = test_runner.new_account();
    let package = test_runner.publish_package("non_fungible");

    // Act
    let manifest = ManifestBuilder::new()
        .call_function(
            package,
            "NonFungibleTest",
            call_data!(create_non_fungible_mutable()),
        )
        .call_method_with_all_resources(account, "deposit_batch")
        .build();
    let signers = vec![];
    let receipt = test_runner.execute_manifest(manifest, signers);

    // Assert
    receipt.result.expect("It should work");
}

#[test]
fn can_burn_non_fungible() {
    // Arrange
    let mut test_runner = TestRunner::new(true);
    let (pk, sk, account) = test_runner.new_account();
    let package = test_runner.publish_package("non_fungible");
    let manifest = ManifestBuilder::new()
        .call_function(
            package,
            "NonFungibleTest",
            call_data!(create_burnable_non_fungible()),
        )
        .call_method_with_all_resources(account, "deposit_batch")
        .build();
    let signers = vec![];
    let receipt = test_runner.execute_manifest(manifest, signers);
    receipt.result.expect("Should be okay.");
    let resource_address = receipt.new_resource_addresses[0];
    let non_fungible_address =
        NonFungibleAddress::new(resource_address, NonFungibleId::from_u32(0));
    let mut ids = BTreeSet::new();
    ids.insert(NonFungibleId::from_u32(0));

    // Act
    let manifest = ManifestBuilder::new()
        .withdraw_from_account(resource_address, account)
        .burn_non_fungible(non_fungible_address.clone())
        .call_function(
            package,
            "NonFungibleTest",
            call_data![verify_does_not_exist(non_fungible_address)],
        )
        .call_method_with_all_resources(account, "deposit_batch")
        .build();
    let signers = vec![pk];
    let receipt = test_runner.execute_manifest(manifest, signers);

    // Assert
    receipt.result.expect("Should be okay.");
}

#[test]
fn test_non_fungible() {
    let mut test_runner = TestRunner::new(true);
    let (pk, sk, account) = test_runner.new_account();
    let package_address = test_runner.publish_package("non_fungible");

    let manifest = ManifestBuilder::new()
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(create_non_fungible_fixed()),
        )
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(update_and_get_non_fungible()),
        )
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(non_fungible_exists()),
        )
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(take_and_put_bucket()),
        )
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(take_and_put_vault()),
        )
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(get_non_fungible_ids_bucket()),
        )
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(get_non_fungible_ids_vault()),
        )
        .call_method_with_all_resources(account, "deposit_batch")
        .build();
    let signers = vec![pk];
    let receipt = test_runner.execute_manifest(manifest, signers);
    println!("{:?}", receipt);
    receipt.result.expect("It should work");
}

#[test]
fn test_singleton_non_fungible() {
    let mut test_runner = TestRunner::new(true);
    let (pk, sk, account) = test_runner.new_account();
    let package_address = test_runner.publish_package("non_fungible");

    let manifest = ManifestBuilder::new()
        .call_function(
            package_address,
            "NonFungibleTest",
            call_data!(singleton_non_fungible()),
        )
        .call_method_with_all_resources(account, "deposit_batch")
        .build();
    let signers = vec![pk];
    let receipt = test_runner.execute_manifest(manifest, signers);
    println!("{:?}", receipt);
    receipt.result.expect("It should work");
}
