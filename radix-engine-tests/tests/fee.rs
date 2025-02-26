use radix_engine::blueprints::resource::WorktopError;
use radix_engine::errors::{ApplicationError, CallFrameError, KernelError};
use radix_engine::errors::{RejectionError, RuntimeError};
use radix_engine::kernel::call_frame::OpenSubstateError;
use radix_engine::kernel::heap::HeapOpenSubstateError;
use radix_engine::track::interface::AcquireLockError;
use radix_engine::transaction::{FeeLocks, TransactionReceipt};
use radix_engine::types::*;
use radix_engine_interface::blueprints::resource::FromPublicKey;
use scrypto_unit::*;
use transaction::prelude::PreviewFlags;
use transaction::prelude::*;
use utils::ContextualDisplay;

fn run_manifest<F>(f: F) -> TransactionReceipt
where
    F: FnOnce(ComponentAddress) -> TransactionManifestV1,
{
    let (mut test_runner, component_address) = setup_test_runner();

    // Run the provided manifest
    let manifest = f(component_address);
    test_runner.execute_manifest(manifest, vec![])
}

fn setup_test_runner() -> (TestRunner, ComponentAddress) {
    // Basic setup
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account) = test_runner.new_allocated_account();

    // Publish package and instantiate component
    let package_address = test_runner.compile_and_publish("./tests/blueprints/fee");
    let receipt1 = test_runner.execute_manifest(
        ManifestBuilder::new()
            .lock_standard_test_fee(account)
            .withdraw_from_account(account, XRD, 1000)
            .take_all_from_worktop(XRD, "bucket")
            .with_name_lookup(|builder, lookup| {
                builder.call_function(
                    package_address,
                    "Fee",
                    "new",
                    manifest_args!(lookup.bucket("bucket")),
                )
            })
            .build(),
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );
    let commit_result = receipt1.expect_commit(true);
    let component_address = commit_result.new_component_addresses()[0];

    (test_runner, component_address)
}

#[test]
fn should_be_aborted_when_loan_repaid() {
    let (mut test_runner, component_address) = setup_test_runner();

    let manifest = ManifestBuilder::new()
        // First, lock the fee so that the loan will be repaid
        .lock_fee_from_faucet()
        // Now spin-loop to wait for the fee loan to burn through
        .call_method(component_address, "spin_loop", manifest_args!())
        .build();

    let start = std::time::Instant::now();
    let receipt = test_runner.execute_manifest(manifest, vec![]);
    let duration = start.elapsed();
    println!("Time elapsed is: {:?}", duration);
    println!(
        "{}",
        receipt.display(&AddressBech32Encoder::for_simulator())
    );
    receipt.expect_commit_failure();
}

#[test]
fn should_succeed_when_fee_is_paid() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .lock_fee(component_address, 500u32)
            .build()
    });

    receipt.expect_commit_success();
}

#[test]
fn should_be_rejected_when_no_fee_is_paid() {
    let receipt = run_manifest(|_| ManifestBuilder::new().build());

    receipt.expect_rejection();
}

#[test]
fn should_be_rejected_when_insufficient_balance() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .call_method(
                component_address,
                "lock_fee_with_empty_vault",
                manifest_args!(Decimal::from(10)),
            )
            .build()
    });

    receipt.expect_rejection();
}

#[test]
fn should_be_rejected_when_non_xrd() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .call_method(
                component_address,
                "lock_fee_with_doge",
                manifest_args!(Decimal::from(10)),
            )
            .build()
    });

    receipt.expect_rejection();
}

#[test]
fn should_be_rejected_when_system_loan_is_not_fully_repaid() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .call_method(
                component_address,
                "lock_fee",
                manifest_args!(Decimal::from_str("0.001").unwrap()), // = 1000 cost units
            )
            .build()
    });

    receipt.expect_rejection();
}

#[test]
fn should_be_rejected_when_lock_fee_with_temp_vault() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .call_method(
                component_address,
                "lock_fee_with_temp_vault",
                manifest_args!(Decimal::from(10)),
            )
            .build()
    });

    receipt.expect_specific_rejection(|e| match e {
        RejectionError::ErrorBeforeFeeLoanRepaid(RuntimeError::KernelError(
            KernelError::CallFrameError(CallFrameError::OpenSubstateError(
                OpenSubstateError::HeapError(HeapOpenSubstateError::LockUnmodifiedBaseOnHeapNode),
            )),
        )) => true,
        _ => false,
    });
}

#[test]
fn should_be_success_when_query_vault_and_lock_fee() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .call_method(
                component_address,
                "query_vault_and_lock_fee",
                manifest_args!(Decimal::from(500)),
            )
            .build()
    });

    receipt.expect_commit_success();
}

#[test]
fn should_be_rejected_when_mutate_vault_and_lock_fee() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .call_method(
                component_address,
                "update_vault_and_lock_fee",
                manifest_args!(Decimal::from(10)),
            )
            .build()
    });

    receipt.expect_specific_rejection(|e| match e {
        RejectionError::ErrorBeforeFeeLoanRepaid(RuntimeError::KernelError(
            KernelError::CallFrameError(CallFrameError::OpenSubstateError(
                OpenSubstateError::TrackError(err),
            )),
        )) => {
            if let AcquireLockError::LockUnmodifiedBaseOnOnUpdatedSubstate(..) = **err {
                return true;
            } else {
                return false;
            }
        }
        _ => false,
    });
}

#[test]
fn should_succeed_when_lock_fee_and_query_vault() {
    let receipt = run_manifest(|component_address| {
        ManifestBuilder::new()
            .call_method(
                component_address,
                "lock_fee_and_query_vault",
                manifest_args!(Decimal::from(500)),
            )
            .build()
    });

    receipt.expect_commit_success();
}

#[test]
fn test_fee_accounting_success() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account1) = test_runner.new_allocated_account();
    let (_, _, account2) = test_runner.new_allocated_account();
    let account1_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account1, 500)
        .withdraw_from_account(account1, XRD, 66)
        .try_deposit_batch_or_abort(account2)
        .build();
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );

    // Assert
    let commit_result = receipt.expect_commit(true);
    let account1_new_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_new_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();
    let summary = &commit_result.fee_summary;
    assert_eq!(
        account1_new_balance,
        account1_balance
            - 66
            - (summary.cost_unit_price + summary.cost_unit_price * summary.tip_percentage / 100)
                * summary.execution_cost_sum
            - summary.total_state_expansion_cost_xrd
    );
    assert_eq!(account2_new_balance, account2_balance + 66);
}

#[test]
fn test_fee_accounting_failure() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account1) = test_runner.new_allocated_account();
    let (_, _, account2) = test_runner.new_allocated_account();
    let account1_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account1, 500)
        .withdraw_from_account(account1, XRD, 66)
        .try_deposit_batch_or_abort(account2)
        .assert_worktop_contains(XRD, 1)
        .build();
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );

    // Assert
    receipt.expect_specific_failure(|e| {
        matches!(
            e,
            RuntimeError::ApplicationError(ApplicationError::WorktopError(
                WorktopError::AssertionFailed
            ))
        )
    });
    let commit_result = receipt.expect_commit(false);
    let account1_new_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_new_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();
    let summary = &commit_result.fee_summary;
    assert_eq!(
        account1_new_balance,
        account1_balance
            - (summary.cost_unit_price + summary.cost_unit_price * summary.tip_percentage / 100)
                * summary.execution_cost_sum
    );
    assert_eq!(account2_new_balance, account2_balance);
}

#[test]
fn test_fee_accounting_rejection() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account1) = test_runner.new_allocated_account();
    let account1_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account1, Decimal::from_str("0.000000000000000001").unwrap())
        .build();
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![NonFungibleGlobalId::from_public_key(&public_key)],
    );

    // Assert
    receipt.expect_rejection();
    let account1_new_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    assert_eq!(account1_new_balance, account1_balance);
}

#[test]
fn test_contingent_fee_accounting_success() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key1, _, account1) = test_runner.new_allocated_account();
    let (public_key2, _, account2) = test_runner.new_allocated_account();
    let account1_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account1, 500)
        .lock_contingent_fee(account2, dec!("0.001"))
        .build();
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![
            NonFungibleGlobalId::from_public_key(&public_key1),
            NonFungibleGlobalId::from_public_key(&public_key2),
        ],
    );

    // Assert
    let commit_result = receipt.expect_commit(true);
    let account1_new_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_new_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();
    let summary = &commit_result.fee_summary;
    let effective_price =
        summary.cost_unit_price + summary.cost_unit_price * summary.tip_percentage / 100;
    let contingent_fee = dec!("0.001");
    assert_eq!(
        account1_new_balance,
        account1_balance
            - effective_price * summary.execution_cost_sum
            - summary.total_state_expansion_cost_xrd
            + contingent_fee
    );
    assert_eq!(account2_new_balance, account2_balance - contingent_fee);
}

#[test]
fn test_contingent_fee_accounting_failure() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key1, _, account1) = test_runner.new_allocated_account();
    let (public_key2, _, account2) = test_runner.new_allocated_account();
    let account1_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account1, 500)
        .lock_contingent_fee(account2, dec!("0.001"))
        .assert_worktop_contains(XRD, 1)
        .build();
    let receipt = test_runner.execute_manifest(
        manifest,
        vec![
            NonFungibleGlobalId::from_public_key(&public_key1),
            NonFungibleGlobalId::from_public_key(&public_key2),
        ],
    );

    // Assert
    receipt.expect_specific_failure(|e| {
        matches!(
            e,
            RuntimeError::ApplicationError(ApplicationError::WorktopError(
                WorktopError::AssertionFailed
            ))
        )
    });
    let commit_result = receipt.expect_commit(false);
    let account1_new_balance = test_runner
        .get_component_resources(account1)
        .get(&XRD)
        .cloned()
        .unwrap();
    let account2_new_balance = test_runner
        .get_component_resources(account2)
        .get(&XRD)
        .cloned()
        .unwrap();
    let summary = &commit_result.fee_summary;
    let effective_price =
        summary.cost_unit_price + summary.cost_unit_price * summary.tip_percentage / 100;
    assert_eq!(
        account1_new_balance,
        account1_balance - effective_price * summary.execution_cost_sum
    );
    assert_eq!(account2_new_balance, account2_balance);
}

#[test]
fn locked_fees_are_correct_in_execution_trace() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key, _, account) = test_runner.new_account(false);

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account, dec!("104.676"))
        .build();
    let receipt = test_runner.preview_manifest(
        manifest,
        vec![public_key.into()],
        0,
        PreviewFlags::default(),
    );

    // Assert
    let commit = receipt.expect_commit_success();
    assert_eq!(
        commit.execution_trace.fee_locks,
        FeeLocks {
            lock: dec!("104.676"),
            contingent_lock: Decimal::ZERO
        }
    )
}

#[test]
fn multiple_locked_fees_are_correct_in_execution_trace() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key1, _, account1) = test_runner.new_account(false);
    let (public_key2, _, account2) = test_runner.new_account(false);

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account1, dec!("104.676"))
        .lock_fee(account2, dec!("102.180"))
        .build();
    let receipt = test_runner.preview_manifest(
        manifest,
        vec![public_key1.into(), public_key2.into()],
        0,
        PreviewFlags::default(),
    );

    // Assert
    let commit = receipt.expect_commit_success();
    assert_eq!(
        commit.execution_trace.fee_locks,
        FeeLocks {
            lock: dec!("206.856"),
            contingent_lock: Decimal::ZERO
        }
    )
}

#[test]
fn regular_and_contingent_fee_locks_are_correct_in_execution_trace() {
    // Arrange
    let mut test_runner = TestRunner::builder().build();
    let (public_key1, _, account1) = test_runner.new_account(false);
    let (public_key2, _, account2) = test_runner.new_account(false);

    // Act
    let manifest = ManifestBuilder::new()
        .lock_fee(account1, dec!("104.676"))
        .lock_contingent_fee(account2, dec!("102.180"))
        .build();
    let receipt = test_runner.preview_manifest(
        manifest,
        vec![public_key1.into(), public_key2.into()],
        0,
        PreviewFlags::default(),
    );

    // Assert
    let commit = receipt.expect_commit_success();
    assert_eq!(
        commit.execution_trace.fee_locks,
        FeeLocks {
            lock: dec!("104.676"),
            contingent_lock: dec!("102.180")
        }
    )
}
