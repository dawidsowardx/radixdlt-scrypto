use radix_engine::blueprints::resource::NonFungibleResourceManagerError;
use radix_engine::errors::{ApplicationError, RuntimeError, SystemModuleError};
use radix_engine::system::system_modules::auth::AuthError;
use radix_engine::transaction::BalanceChange;
use radix_engine::types::*;
use radix_engine_interface::api::node_modules::metadata::MetadataValue;
use radix_engine_interface::blueprints::account::{AccountSecurifyInput, ACCOUNT_SECURIFY_IDENT};
use radix_engine_interface::blueprints::resource::FromPublicKey;
use scrypto_unit::*;
use transaction::prelude::*;

#[test]
fn can_securify_virtual_account() {
    securify_account(true, true, true);
}

#[test]
fn cannot_securify_virtual_account_without_key() {
    securify_account(true, false, false);
}

#[test]
fn cannot_securify_allocated_account() {
    securify_account(false, true, false);
}

fn securify_account(is_virtual: bool, use_key: bool, expect_success: bool) {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (key, _, account) = test_runner.new_account(is_virtual);

    let (_, _, storing_account) = test_runner.new_account(true);

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee_from_faucet()
        .call_method(account, ACCOUNT_SECURIFY_IDENT, AccountSecurifyInput {})
        .try_deposit_batch_or_refund(storing_account)
        .build();
    let initial_proofs = if use_key {
        vec![NonFungibleGlobalId::from_public_key(&key)]
    } else {
        vec![]
    };
    let receipt = test_runner.execute_manifest(manifest, initial_proofs);

    // Assert
    if expect_success {
        receipt.expect_commit_success();
    } else {
        receipt.expect_specific_failure(|e| {
            matches!(
                e,
                RuntimeError::SystemModuleError(SystemModuleError::AuthError(
                    AuthError::Unauthorized { .. }
                ))
            )
        });
    }
}

#[test]
fn can_withdraw_from_my_allocated_account() {
    can_withdraw_from_my_account_internal(|test_runner| {
        let (public_key, _, account) = test_runner.new_account(false);
        (public_key, account)
    });
}

#[test]
fn can_withdraw_from_my_virtual_account() {
    can_withdraw_from_my_account_internal(|test_runner| {
        let (public_key, _, account) = test_runner.new_account(true);
        (public_key, account)
    });
}

fn can_withdraw_from_my_account_internal<F>(new_account: F)
where
    F: FnOnce(&mut TestRunner) -> (Secp256k1PublicKey, ComponentAddress),
{
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, account) = new_account(&mut test_runner);
    let (_, _, other_account) = test_runner.new_account(true);

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee_and_withdraw(account, 500, XRD, 1)
        .try_deposit_batch_or_refund(other_account)
        .build();
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );

    // Assert
    let other_account_balance: Decimal = test_runner.account_balance(other_account, XRD).unwrap();
    let transfer_amount = other_account_balance - 10000 /* initial balance */;

    assert_eq!(
        receipt
            .expect_commit_success()
            .state_update_summary
            .balance_changes
            .get(&GlobalAddress::from(other_account))
            .unwrap()
            .get(&XRD)
            .unwrap(),
        &BalanceChange::Fungible(transfer_amount)
    );
}

fn can_withdraw_non_fungible_from_my_account_internal(use_virtual: bool) {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account) = test_runner.new_account(use_virtual);
    let (_, _, other_account) = test_runner.new_account(use_virtual);
    let resource_address = test_runner.create_non_fungible_resource(account);

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee_and_withdraw(account, 500, resource_address, 1)
        .try_deposit_batch_or_refund(other_account)
        .build();
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );

    // Assert
    receipt.expect_commit_success();
}

#[test]
fn can_withdraw_non_fungible_from_my_allocated_account() {
    can_withdraw_non_fungible_from_my_account_internal(false)
}

#[test]
fn can_withdraw_non_fungible_from_my_virtual_account() {
    can_withdraw_non_fungible_from_my_account_internal(true)
}

fn cannot_withdraw_from_other_account_internal(is_virtual: bool) {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account) = test_runner.new_account(is_virtual);
    let (_, _, other_account) = test_runner.new_account(is_virtual);
    let manifest = ManifestBuilder::new()
        .lock_fee(account, 500u32)
        .withdraw_from_account(other_account, XRD, 1)
        .try_deposit_batch_or_refund(account)
        .build();

    // Act
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );

    // Assert
    receipt.expect_specific_failure(is_auth_error);
}

#[test]
fn virtual_account_is_created_with_public_key_hash_metadata() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();

    // Act
    let (public_key, _, account) = test_runner.new_account(true);

    // Assert
    let entry = test_runner.get_metadata(account.into(), "owner_keys");

    let public_key_hash = public_key.get_hash().into_enum();
    assert_eq!(
        entry,
        Some(MetadataValue::PublicKeyHashArray(vec![public_key_hash])),
    );
}

#[test]
fn cannot_withdraw_from_other_allocated_account() {
    cannot_withdraw_from_other_account_internal(false);
}

#[test]
fn cannot_withdraw_from_other_virtual_account() {
    cannot_withdraw_from_other_account_internal(true);
}

fn account_to_bucket_to_account_internal(use_virtual: bool) {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account) = test_runner.new_account(use_virtual);
    let manifest = ManifestBuilder::new()
        .lock_fee_and_withdraw(account, 500u32, XRD, 1)
        .take_all_from_worktop(XRD, "xrd")
        .try_deposit_or_abort(account, "xrd")
        .build();

    // Act
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );

    // Assert
    let result = receipt.expect_commit_success();

    assert_eq!(
        receipt
            .expect_commit_success()
            .state_update_summary
            .balance_changes
            .get(&GlobalAddress::from(account))
            .unwrap()
            .get(&XRD)
            .unwrap(),
        &BalanceChange::Fungible(-result.fee_summary.total_cost())
    );
}

#[test]
fn account_to_bucket_to_allocated_account() {
    account_to_bucket_to_account_internal(false);
}

#[test]
fn account_to_bucket_to_virtual_account() {
    account_to_bucket_to_account_internal(true);
}

#[test]
fn create_account_and_bucket_fail() {
    let mut test_runner = TestRunner::builder().build();
    let manifest = ManifestBuilder::new().new_account().build();
    let receipt = test_runner.execute_manifest_ignoring_fee(manifest, vec![]);
    receipt.expect_specific_failure(|e| {
        matches!(
            e,
            RuntimeError::ApplicationError(ApplicationError::NonFungibleResourceManagerError(
                NonFungibleResourceManagerError::DropNonEmptyBucket
            ))
        )
    });
}

#[test]
fn virtual_account_has_expected_owner_key() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (_, _, account) = test_runner.new_account(true);

    // Act
    let metadata = test_runner
        .get_metadata(account.into(), "owner_badge")
        .unwrap();

    // Assert
    assert_eq!(
        metadata,
        MetadataValue::NonFungibleLocalId(
            NonFungibleLocalId::bytes(account.as_node_id().0).unwrap()
        )
    )
}

#[test]
fn securified_account_is_owned_by_correct_owner_badge() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (pk, _, account) = test_runner.new_account(true);

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee_from_faucet()
        .call_method(account, ACCOUNT_SECURIFY_IDENT, AccountSecurifyInput {})
        .try_deposit_batch_or_refund(account)
        .build();
    let receipt =
        test_runner.execute_manifest(manifest, vec![NonFungibleGlobalId::from_public_key(&pk)]);

    // Assert
    let balance_changes = receipt.expect_commit_success().balance_changes();
    let balance_change = balance_changes
        .get(&GlobalAddress::from(account))
        .unwrap()
        .get(&ACCOUNT_OWNER_BADGE)
        .unwrap()
        .clone();
    assert_eq!(
        balance_change,
        BalanceChange::NonFungible {
            added: btreeset![NonFungibleLocalId::bytes(account.as_node_id().0).unwrap()],
            removed: btreeset![]
        }
    )
}

#[test]
fn account_created_with_create_advanced_has_an_empty_owner_badge() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let account = test_runner.new_account_advanced(OwnerRole::None);

    // Act
    let metadata = test_runner.get_metadata(account.into(), "owner_badge");

    // Assert
    assert!(is_metadata_empty(&metadata))
}

fn is_metadata_empty(metadata_value: &Option<MetadataValue>) -> bool {
    if let None = metadata_value {
        true
    } else {
        false
    }
}
