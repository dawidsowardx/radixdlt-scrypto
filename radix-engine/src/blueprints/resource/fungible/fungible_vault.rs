use crate::blueprints::resource::*;
use crate::errors::ApplicationError;
use crate::errors::RuntimeError;
use crate::kernel::heap::{DroppedBucket, DroppedBucketResource};
use crate::kernel::kernel_api::{KernelNodeApi};
use crate::types::*;
use native_sdk::runtime::Runtime;
use radix_engine_interface::api::substate_lock_api::LockFlags;
use radix_engine_interface::api::ClientApi;
use radix_engine_interface::blueprints::resource::*;
use radix_engine_interface::types::*;

pub struct FungibleVaultBlueprint;

impl FungibleVaultBlueprint {
    fn check_amount(amount: &Decimal, divisibility: u8) -> bool {
        !amount.is_negative()
            && amount.0 % BnumI256::from(10i128.pow((18 - divisibility).into()))
                == BnumI256::from(0)
    }

    fn get_divisibility<Y>(api: &mut Y) -> Result<u8, RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        let handle = api.lock_parent_field(
            FungibleResourceManagerOffset::Divisibility.into(),
            LockFlags::read_only(),
        )?;
        let divisibility: u8 = api.field_lock_read_typed(handle)?;
        api.field_lock_release(handle)?;
        Ok(divisibility)
    }

    pub fn take<Y>(amount: &Decimal, api: &mut Y) -> Result<Bucket, RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        let divisibility = Self::get_divisibility(api)?;
        let resource_address =
            ResourceAddress::new_unchecked(api.get_info()?.blueprint_parent.unwrap().into());

        // Check amount
        if !Self::check_amount(amount, divisibility) {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::VaultError(VaultError::InvalidAmount),
            ));
        }

        // Take
        let taken = FungibleVault::take(*amount, api)?;

        // Create node
        let bucket_id = api.new_object(
            BUCKET_BLUEPRINT,
            vec![
                scrypto_encode(&BucketInfoSubstate {
                    resource_address,
                    resource_type: ResourceType::Fungible { divisibility },
                })
                .unwrap(),
                scrypto_encode(&taken).unwrap(),
                scrypto_encode(&LockedFungibleResource::default()).unwrap(),
                scrypto_encode(&LiquidNonFungibleResource::default()).unwrap(),
                scrypto_encode(&LockedNonFungibleResource::default()).unwrap(),
            ],
        )?;

        Ok(Bucket(Own(bucket_id)))
    }

    pub fn put<Y>(bucket: Bucket, api: &mut Y) -> Result<(), RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        // Drop other bucket
        let other_bucket: DroppedBucket = api.kernel_drop_node(bucket.0.as_node_id())?.into();

        // Check resource address
        {
            let resource_address =
                ResourceAddress::new_unchecked(api.get_info()?.blueprint_parent.unwrap().into());
            if resource_address != other_bucket.info.resource_address {
                return Err(RuntimeError::ApplicationError(
                    ApplicationError::VaultError(VaultError::MismatchingResource),
                ));
            }
        }

        // Put
        if let DroppedBucketResource::Fungible(r) = other_bucket.resource {
            FungibleVault::put(r, api)?;
        } else {
            panic!("expecting fungible bucket")
        }

        Ok(())
    }

    pub fn get_amount<Y>(api: &mut Y) -> Result<Decimal, RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        let amount = FungibleVault::liquid_amount(api)? + FungibleVault::locked_amount(api)?;

        Ok(amount)
    }

    pub fn lock_fee<Y>(
        receiver: &NodeId,
        amount: Decimal,
        contingent: bool,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        // Check resource address and amount
        let resource_address =
            ResourceAddress::new_unchecked(api.get_info()?.blueprint_parent.unwrap().into());
        if resource_address != RADIX_TOKEN {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::VaultError(VaultError::LockFeeNotRadixToken),
            ));
        }

        let divisibility = Self::get_divisibility(api)?;
        if !Self::check_amount(&amount, divisibility) {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::VaultError(VaultError::InvalidAmount),
            ));
        }

        // Lock the substate (with special flags)
        let vault_handle = api.lock_field(
            FungibleVaultOffset::LiquidFungible.into(),
            LockFlags::MUTABLE | LockFlags::UNMODIFIED_BASE | LockFlags::FORCE_WRITE,
        )?;

        // Take fee from the vault
        let mut vault: LiquidFungibleResource = api.field_lock_read_typed(vault_handle)?;
        let fee = vault.take_by_amount(amount).map_err(|_| {
            RuntimeError::ApplicationError(ApplicationError::VaultError(
                VaultError::LockFeeInsufficientBalance,
            ))
        })?;

        // Credit cost units
        let changes = api.credit_cost_units(receiver.clone().into(), fee, contingent)?;

        // Keep changes
        if !changes.is_empty() {
            vault.put(changes).expect("Failed to put fee changes");
        }

        // Flush updates
        api.field_lock_write_typed(vault_handle, &vault)?;
        api.field_lock_release(vault_handle)?;

        // Emitting an event once the fee has been locked
        Runtime::emit_event(api, LockFeeEvent { amount })?;

        Ok(IndexedScryptoValue::from_typed(&()))
    }

    pub fn recall<Y>(amount: Decimal, api: &mut Y) -> Result<Bucket, RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        let divisibility = Self::get_divisibility(api)?;
        if !Self::check_amount(&amount, divisibility) {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::VaultError(VaultError::InvalidAmount),
            ));
        }

        let resource_address =
            ResourceAddress::new_unchecked(api.get_info()?.blueprint_parent.unwrap().into());
        let taken = FungibleVault::take(amount, api)?;
        let bucket_id = api.new_object(
            BUCKET_BLUEPRINT,
            vec![
                scrypto_encode(&BucketInfoSubstate {
                    resource_address,
                    resource_type: ResourceType::Fungible { divisibility },
                })
                .unwrap(),
                scrypto_encode(&taken).unwrap(),
                scrypto_encode(&LockedFungibleResource::default()).unwrap(),
                scrypto_encode(&LiquidNonFungibleResource::default()).unwrap(),
                scrypto_encode(&LockedNonFungibleResource::default()).unwrap(),
            ],
        )?;

        Runtime::emit_event(api, RecallResourceEvent::Amount(amount))?;
        Ok(Bucket(Own(bucket_id)))
    }

    pub fn create_proof<Y>(receiver: &NodeId, api: &mut Y) -> Result<Proof, RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        let amount = FungibleVault::liquid_amount(api)? + FungibleVault::locked_amount(api)?;

        let divisibility = Self::get_divisibility(api)?;
        let resource_address =
            ResourceAddress::new_unchecked(api.get_info()?.blueprint_parent.unwrap().into());
        let proof_info = ProofInfoSubstate {
            resource_address,
            resource_type: ResourceType::Fungible { divisibility },
            restricted: false,
        };
        let proof = FungibleVault::lock_amount(receiver, amount, api)?;

        let proof_id = api.new_object(
            PROOF_BLUEPRINT,
            vec![
                scrypto_encode(&proof_info).unwrap(),
                scrypto_encode(&proof).unwrap(),
                scrypto_encode(&NonFungibleProof::default()).unwrap(),
            ],
        )?;

        Ok(Proof(Own(proof_id)))
    }

    pub fn create_proof_by_amount<Y>(
        receiver: &NodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<Proof, RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        let divisibility = Self::get_divisibility(api)?;
        if !Self::check_amount(&amount, divisibility) {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::VaultError(VaultError::InvalidAmount),
            ));
        }

        let resource_address =
            ResourceAddress::new_unchecked(api.get_info()?.blueprint_parent.unwrap().into());
        let proof_info = ProofInfoSubstate {
            resource_address,
            resource_type: ResourceType::Fungible { divisibility },
            restricted: false,
        };
        let proof = FungibleVault::lock_amount(receiver, amount, api)?;
        let proof_id = api.new_object(
            PROOF_BLUEPRINT,
            vec![
                scrypto_encode(&proof_info).unwrap(),
                scrypto_encode(&proof).unwrap(),
                scrypto_encode(&NonFungibleProof::default()).unwrap(),
            ],
        )?;

        Ok(Proof(Own(proof_id)))
    }

    //===================
    // Protected method
    //===================

    // FIXME: set up auth

    pub fn lock_amount<Y>(
        receiver: &NodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<(), RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        FungibleVault::lock_amount(receiver, amount, api)?;
        Ok(())
    }

    pub fn unlock_amount<Y>(amount: Decimal, api: &mut Y) -> Result<(), RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        FungibleVault::unlock_amount(amount, api)?;

        Ok(())
    }
}

pub struct FungibleVault;

impl FungibleVault {
    pub fn liquid_amount<Y>(api: &mut Y) -> Result<Decimal, RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        let handle = api.lock_field(
            FungibleVaultOffset::LiquidFungible.into(),
            LockFlags::read_only(),
        )?;
        let substate_ref: LiquidFungibleResource = api.field_lock_read_typed(handle)?;
        let amount = substate_ref.amount();
        api.field_lock_release(handle)?;
        Ok(amount)
    }

    pub fn locked_amount<Y>(api: &mut Y) -> Result<Decimal, RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        let handle = api.lock_field(
            FungibleVaultOffset::LockedFungible.into(),
            LockFlags::read_only(),
        )?;
        let substate_ref: LockedFungibleResource = api.field_lock_read_typed(handle)?;
        let amount = substate_ref.amount();
        api.field_lock_release(handle)?;
        Ok(amount)
    }

    pub fn take<Y>(amount: Decimal, api: &mut Y) -> Result<LiquidFungibleResource, RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        let handle = api.lock_field(
            FungibleVaultOffset::LiquidFungible.into(),
            LockFlags::MUTABLE,
        )?;
        let mut substate_ref: LiquidFungibleResource = api.field_lock_read_typed(handle)?;
        let taken = substate_ref.take_by_amount(amount).map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::VaultError(VaultError::ResourceError(
                e,
            )))
        })?;
        api.field_lock_write_typed(handle, &substate_ref)?;
        api.field_lock_release(handle)?;

        Runtime::emit_event(api, WithdrawResourceEvent::Amount(amount))?;

        Ok(taken)
    }

    pub fn put<Y>(resource: LiquidFungibleResource, api: &mut Y) -> Result<(), RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        if resource.is_empty() {
            return Ok(());
        }

        let event = DepositResourceEvent::Amount(resource.amount());

        let handle = api.lock_field(
            FungibleVaultOffset::LiquidFungible.into(),
            LockFlags::MUTABLE,
        )?;
        let mut substate_ref: LiquidFungibleResource = api.field_lock_read_typed(handle)?;
        substate_ref.put(resource).map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::VaultError(VaultError::ResourceError(
                e,
            )))
        })?;
        api.field_lock_write_typed(handle, &substate_ref)?;
        api.field_lock_release(handle)?;

        Runtime::emit_event(api, event)?;

        Ok(())
    }

    // protected method
    pub fn lock_amount<Y>(
        receiver: &NodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<FungibleProof, RuntimeError>
    where
        Y: KernelNodeApi + ClientApi<RuntimeError>,
    {
        let handle = api.lock_field(
            FungibleVaultOffset::LockedFungible.into(),
            LockFlags::MUTABLE,
        )?;
        let mut locked: LockedFungibleResource = api.field_lock_read_typed(handle)?;
        let max_locked = locked.amount();

        // Take from liquid if needed
        if amount > max_locked {
            let delta = amount - max_locked;
            FungibleVault::take(delta, api)?;
        }

        // Increase lock count
        locked.amounts.entry(amount).or_default().add_assign(1);
        api.field_lock_write_typed(handle, &locked)?;

        // Issue proof
        Ok(FungibleProof::new(
            amount,
            btreemap!(
                LocalRef::Vault(Reference(receiver.clone().into())) => amount
            ),
        )
        .map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::VaultError(VaultError::ProofError(e)))
        })?)
    }

    // protected method
    pub fn unlock_amount<Y>(amount: Decimal, api: &mut Y) -> Result<(), RuntimeError>
    where
        Y: ClientApi<RuntimeError>,
    {
        let handle = api.lock_field(
            FungibleVaultOffset::LockedFungible.into(),
            LockFlags::MUTABLE,
        )?;
        let mut locked: LockedFungibleResource = api.field_lock_read_typed(handle)?;

        let max_locked = locked.amount();
        let cnt = locked
            .amounts
            .remove(&amount)
            .expect("Attempted to unlock an amount that is not locked");
        if cnt > 1 {
            locked.amounts.insert(amount, cnt - 1);
        }

        api.field_lock_write_typed(handle, &locked)?;

        let delta = max_locked - locked.amount();
        FungibleVault::put(LiquidFungibleResource::new(delta), api)
    }
}
