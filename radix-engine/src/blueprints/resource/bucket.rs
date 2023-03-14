use crate::blueprints::resource::*;
use crate::errors::RuntimeError;
use crate::errors::{ApplicationError, InterpreterError};
use crate::kernel::heap::{DroppedBucket, DroppedBucketResource};
use crate::kernel::kernel_api::{KernelNodeApi, KernelSubstateApi};
use crate::types::*;
use native_sdk::resource::SysBucket;
use radix_engine_interface::api::substate_api::LockFlags;
use radix_engine_interface::api::ClientApi;
use radix_engine_interface::api::{types::*, ClientSubstateApi};
use radix_engine_interface::blueprints::resource::*;

#[derive(Debug, Clone, PartialEq, Eq, ScryptoSbor)]
pub enum BucketError {
    ResourceError(ResourceError),
    ProofError(ProofError),
    NonFungibleOperationNotSupported,
    MismatchingResource,
    NotEmpty,
    InvalidAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, ScryptoSbor)]
pub struct BucketInfoSubstate {
    pub resource_address: ResourceAddress,
    pub resource_type: ResourceType,
}

impl BucketInfoSubstate {
    pub fn of<Y>(node_id: RENodeId, api: &mut Y) -> Result<Self, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::Info),
            LockFlags::read_only(),
        )?;
        let substate_ref: &BucketInfoSubstate = api.kernel_get_substate_ref(handle)?;
        let info = substate_ref.clone();
        api.sys_drop_lock(handle)?;
        Ok(info)
    }
}

pub struct FungibleBucket;

impl FungibleBucket {
    pub fn liquid_amount<Y>(node_id: RENodeId, api: &mut Y) -> Result<Decimal, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidFungible),
            LockFlags::read_only(),
        )?;
        let substate_ref: &LiquidFungibleResource = api.kernel_get_substate_ref(handle)?;
        let amount = substate_ref.amount();
        api.sys_drop_lock(handle)?;
        Ok(amount)
    }

    pub fn locked_amount<Y>(node_id: RENodeId, api: &mut Y) -> Result<Decimal, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedFungible),
            LockFlags::read_only(),
        )?;
        let substate_ref: &LockedFungibleResource = api.kernel_get_substate_ref(handle)?;
        let amount = substate_ref.amount();
        api.sys_drop_lock(handle)?;
        Ok(amount)
    }

    pub fn is_locked<Y>(node_id: RENodeId, api: &mut Y) -> Result<bool, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        Ok(!Self::locked_amount(node_id, api)?.is_zero())
    }

    pub fn take<Y>(
        node_id: RENodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<LiquidFungibleResource, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidFungible),
            LockFlags::MUTABLE,
        )?;
        let substate_ref: &mut LiquidFungibleResource = api.kernel_get_substate_ref_mut(handle)?;
        let taken = substate_ref.take_by_amount(amount).map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::BucketError(
                BucketError::ResourceError(e),
            ))
        })?;
        api.sys_drop_lock(handle)?;
        Ok(taken)
    }

    pub fn put<Y>(
        node_id: RENodeId,
        resource: LiquidFungibleResource,
        api: &mut Y,
    ) -> Result<(), RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        if resource.is_empty() {
            return Ok(());
        }

        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidFungible),
            LockFlags::MUTABLE,
        )?;
        let substate_ref: &mut LiquidFungibleResource = api.kernel_get_substate_ref_mut(handle)?;
        substate_ref.put(resource).map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::BucketError(
                BucketError::ResourceError(e),
            ))
        })?;
        api.sys_drop_lock(handle)?;
        Ok(())
    }

    // protected method
    pub fn lock_amount<Y>(
        node_id: RENodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<FungibleProof, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedFungible),
            LockFlags::MUTABLE,
        )?;
        let mut locked: &mut LockedFungibleResource = api.kernel_get_substate_ref_mut(handle)?;
        let max_locked = locked.amount();

        // Take from liquid if needed
        if amount > max_locked {
            let delta = amount - max_locked;
            FungibleBucket::take(node_id, delta, api)?;
        }

        // Increase lock count
        locked = api.kernel_get_substate_ref_mut(handle)?; // grab ref again
        locked.amounts.entry(amount).or_default().add_assign(1);

        // Issue proof
        Ok(FungibleProof::new(
            amount,
            btreemap!(
                LocalRef::Bucket(node_id.into()) => amount
            ),
        )
        .map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::BucketError(BucketError::ProofError(
                e,
            )))
        })?)
    }

    // protected method
    pub fn unlock_amount<Y>(
        node_id: RENodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<(), RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedFungible),
            LockFlags::MUTABLE,
        )?;
        let locked: &mut LockedFungibleResource = api.kernel_get_substate_ref_mut(handle)?;

        let max_locked = locked.amount();
        let cnt = locked
            .amounts
            .remove(&amount)
            .expect("Attempted to unlock an amount that is not locked");
        if cnt > 1 {
            locked.amounts.insert(amount, cnt - 1);
        }

        let delta = max_locked - locked.amount();
        FungibleBucket::put(node_id, LiquidFungibleResource::new(delta), api)
    }
}

pub struct NonFungibleBucket;

impl NonFungibleBucket {
    pub fn liquid_amount<Y>(node_id: RENodeId, api: &mut Y) -> Result<Decimal, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidNonFungible),
            LockFlags::read_only(),
        )?;
        let substate_ref: &LiquidNonFungibleResource = api.kernel_get_substate_ref(handle)?;
        let amount = substate_ref.amount();
        api.sys_drop_lock(handle)?;
        Ok(amount)
    }

    pub fn locked_amount<Y>(node_id: RENodeId, api: &mut Y) -> Result<Decimal, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedNonFungible),
            LockFlags::read_only(),
        )?;
        let substate_ref: &LockedNonFungibleResource = api.kernel_get_substate_ref(handle)?;
        let amount = substate_ref.amount();
        api.sys_drop_lock(handle)?;
        Ok(amount)
    }

    pub fn is_locked<Y>(node_id: RENodeId, api: &mut Y) -> Result<bool, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        Ok(!Self::locked_amount(node_id, api)?.is_zero())
    }

    pub fn liquid_non_fungible_local_ids<Y>(
        node_id: RENodeId,
        api: &mut Y,
    ) -> Result<BTreeSet<NonFungibleLocalId>, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidNonFungible),
            LockFlags::read_only(),
        )?;
        let substate_ref: &LiquidNonFungibleResource = api.kernel_get_substate_ref(handle)?;
        let ids = substate_ref.ids().clone();
        api.sys_drop_lock(handle)?;
        Ok(ids)
    }

    pub fn locked_non_fungible_local_ids<Y>(
        node_id: RENodeId,
        api: &mut Y,
    ) -> Result<BTreeSet<NonFungibleLocalId>, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedNonFungible),
            LockFlags::read_only(),
        )?;
        let substate_ref: &LockedNonFungibleResource = api.kernel_get_substate_ref(handle)?;
        let ids = substate_ref.ids();
        api.sys_drop_lock(handle)?;
        Ok(ids)
    }

    pub fn take<Y>(
        node_id: RENodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<LiquidNonFungibleResource, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidNonFungible),
            LockFlags::MUTABLE,
        )?;
        let substate_ref: &mut LiquidNonFungibleResource =
            api.kernel_get_substate_ref_mut(handle)?;
        let taken = substate_ref.take_by_amount(amount).map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::BucketError(
                BucketError::ResourceError(e),
            ))
        })?;
        api.sys_drop_lock(handle)?;
        Ok(taken)
    }

    pub fn take_non_fungibles<Y>(
        node_id: RENodeId,
        ids: &BTreeSet<NonFungibleLocalId>,
        api: &mut Y,
    ) -> Result<LiquidNonFungibleResource, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidNonFungible),
            LockFlags::MUTABLE,
        )?;
        let substate_ref: &mut LiquidNonFungibleResource =
            api.kernel_get_substate_ref_mut(handle)?;
        let taken = substate_ref
            .take_by_ids(ids)
            .map_err(BucketError::ResourceError)
            .map_err(|e| RuntimeError::ApplicationError(ApplicationError::BucketError(e)))?;
        api.sys_drop_lock(handle)?;
        Ok(taken)
    }

    pub fn put<Y>(
        node_id: RENodeId,
        resource: LiquidNonFungibleResource,
        api: &mut Y,
    ) -> Result<(), RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        if resource.is_empty() {
            return Ok(());
        }

        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LiquidNonFungible),
            LockFlags::MUTABLE,
        )?;
        let substate_ref: &mut LiquidNonFungibleResource =
            api.kernel_get_substate_ref_mut(handle)?;
        substate_ref.put(resource).map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::BucketError(
                BucketError::ResourceError(e),
            ))
        })?;
        api.sys_drop_lock(handle)?;
        Ok(())
    }

    // protected method
    pub fn lock_amount<Y>(
        node_id: RENodeId,
        amount: Decimal,
        api: &mut Y,
    ) -> Result<NonFungibleProof, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedNonFungible),
            LockFlags::MUTABLE,
        )?;
        let mut locked: &mut LockedNonFungibleResource = api.kernel_get_substate_ref_mut(handle)?;
        let max_locked: Decimal = locked.ids.len().into();

        // Take from liquid if needed
        if amount > max_locked {
            let delta = amount - max_locked;
            let resource = NonFungibleBucket::take(node_id, delta, api)?;

            locked = api.kernel_get_substate_ref_mut(handle)?; // grab ref again
            for nf in resource.into_ids() {
                locked.ids.insert(nf, 0);
            }
        }

        // Increase lock count
        let n: usize = amount
            .to_string()
            .parse()
            .expect("Failed to convert amount to usize");
        let ids_for_proof: BTreeSet<NonFungibleLocalId> =
            locked.ids.keys().cloned().into_iter().take(n).collect();
        for id in &ids_for_proof {
            locked.ids.entry(id.clone()).or_default().add_assign(1);
        }

        // Issue proof
        Ok(NonFungibleProof::new(
            ids_for_proof.clone(),
            btreemap!(
                LocalRef::Bucket(node_id.into()) => ids_for_proof
            ),
        )
        .map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::BucketError(BucketError::ProofError(
                e,
            )))
        })?)
    }

    // protected method
    pub fn lock_non_fungibles<Y>(
        node_id: RENodeId,
        ids: BTreeSet<NonFungibleLocalId>,
        api: &mut Y,
    ) -> Result<NonFungibleProof, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedNonFungible),
            LockFlags::MUTABLE,
        )?;
        let mut locked: &mut LockedNonFungibleResource = api.kernel_get_substate_ref_mut(handle)?;

        // Take from liquid if needed
        let delta: BTreeSet<NonFungibleLocalId> = ids
            .iter()
            .cloned()
            .filter(|id| !locked.ids.contains_key(id))
            .collect();
        NonFungibleBucket::take_non_fungibles(node_id, &delta, api)?;

        // Increase lock count
        locked = api.kernel_get_substate_ref_mut(handle)?; // grab ref again
        for id in &ids {
            locked.ids.entry(id.clone()).or_default().add_assign(1);
        }

        // Issue proof
        Ok(NonFungibleProof::new(
            ids.clone(),
            btreemap!(
                LocalRef::Bucket(node_id.into()) => ids
            ),
        )
        .map_err(|e| {
            RuntimeError::ApplicationError(ApplicationError::BucketError(BucketError::ProofError(
                e,
            )))
        })?)
    }

    // protected method
    pub fn unlock_non_fungibles<Y>(
        node_id: RENodeId,
        ids: BTreeSet<NonFungibleLocalId>,
        api: &mut Y,
    ) -> Result<(), RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientSubstateApi<RuntimeError>,
    {
        let handle = api.sys_lock_substate(
            node_id,
            SubstateOffset::Bucket(BucketOffset::LockedNonFungible),
            LockFlags::MUTABLE,
        )?;
        let locked: &mut LockedNonFungibleResource = api.kernel_get_substate_ref_mut(handle)?;

        let mut liquid_non_fungibles = BTreeSet::<NonFungibleLocalId>::new();
        for id in ids {
            let cnt = locked
                .ids
                .remove(&id)
                .expect("Attempted to unlock non-fungible that was not locked");
            if cnt > 1 {
                locked.ids.insert(id, cnt - 1);
            } else {
                liquid_non_fungibles.insert(id);
            }
        }

        NonFungibleBucket::put(
            node_id,
            LiquidNonFungibleResource::new(liquid_non_fungibles),
            api,
        )
    }
}

pub struct BucketBlueprint;

impl BucketBlueprint {
    pub fn drop_empty<Y>(
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketDropEmptyInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        let amount = input.bucket.sys_amount(api)?;
        if amount.is_zero() {
            api.kernel_drop_node(RENodeId::Object(input.bucket.0))?;
            Ok(IndexedScryptoValue::from_typed(&()))
        } else {
            Err(RuntimeError::ApplicationError(
                ApplicationError::BucketError(BucketError::NotEmpty),
            ))
        }
    }

    pub fn take<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketTakeInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        // Check amount
        let info = BucketInfoSubstate::of(receiver, api)?;
        if !info.resource_type.check_amount(input.amount) {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::BucketError(BucketError::InvalidAmount),
            ));
        }

        let node_id = if info.resource_type.is_fungible() {
            // Take
            let taken = FungibleBucket::take(receiver, input.amount, api)?;

            // Create node
            let bucket_id = api.new_object(
                BUCKET_BLUEPRINT,
                vec![
                    scrypto_encode(&info).unwrap(),
                    scrypto_encode(&taken).unwrap(),
                    scrypto_encode(&LockedFungibleResource::default()).unwrap(),
                    scrypto_encode(&LiquidNonFungibleResource::default()).unwrap(),
                    scrypto_encode(&LockedNonFungibleResource::default()).unwrap(),
                ],
            )?;

            RENodeId::Object(bucket_id)
        } else {
            // Take
            let taken = NonFungibleBucket::take(receiver, input.amount, api)?;

            // Create node
            let bucket_id = api.new_object(
                BUCKET_BLUEPRINT,
                vec![
                    scrypto_encode(&info).unwrap(),
                    scrypto_encode(&LiquidFungibleResource::default()).unwrap(),
                    scrypto_encode(&LockedFungibleResource::default()).unwrap(),
                    scrypto_encode(&taken).unwrap(),
                    scrypto_encode(&LockedNonFungibleResource::default()).unwrap(),
                ],
            )?;

            RENodeId::Object(bucket_id)
        };
        let bucket_id = node_id.into();

        Ok(IndexedScryptoValue::from_typed(&Bucket(bucket_id)))
    }

    pub fn take_non_fungibles<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketTakeNonFungiblesInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        let info = BucketInfoSubstate::of(receiver, api)?;

        if info.resource_type.is_fungible() {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::BucketError(BucketError::NonFungibleOperationNotSupported),
            ));
        } else {
            // Take
            let taken = NonFungibleBucket::take_non_fungibles(receiver, &input.ids, api)?;

            // Create node
            let bucket_id = api.new_object(
                BUCKET_BLUEPRINT,
                vec![
                    scrypto_encode(&info).unwrap(),
                    scrypto_encode(&LiquidFungibleResource::default()).unwrap(),
                    scrypto_encode(&LockedFungibleResource::default()).unwrap(),
                    scrypto_encode(&taken).unwrap(),
                    scrypto_encode(&LockedNonFungibleResource::default()).unwrap(),
                ],
            )?;

            Ok(IndexedScryptoValue::from_typed(&Bucket(bucket_id)))
        }
    }

    pub fn put<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketPutInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        // Drop other bucket
        let other_bucket: DroppedBucket = api
            .kernel_drop_node(RENodeId::Object(input.bucket.0))?
            .into();

        // Check resource address
        let info = BucketInfoSubstate::of(receiver, api)?;
        if info.resource_address != other_bucket.info.resource_address {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::BucketError(BucketError::MismatchingResource),
            ));
        }

        // Put
        match other_bucket.resource {
            DroppedBucketResource::Fungible(r) => {
                FungibleBucket::put(receiver, r, api)?;
            }
            DroppedBucketResource::NonFungible(r) => {
                NonFungibleBucket::put(receiver, r, api)?;
            }
        }
        Ok(IndexedScryptoValue::from_typed(&()))
    }

    pub fn get_non_fungible_local_ids<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let _input: BucketGetNonFungibleLocalIdsInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        let info = BucketInfoSubstate::of(receiver, api)?;
        if info.resource_type.is_fungible() {
            return Err(RuntimeError::ApplicationError(
                ApplicationError::BucketError(BucketError::NonFungibleOperationNotSupported),
            ));
        } else {
            let mut ids = NonFungibleBucket::liquid_non_fungible_local_ids(receiver, api)?;
            ids.extend(NonFungibleBucket::locked_non_fungible_local_ids(
                receiver, api,
            )?);
            Ok(IndexedScryptoValue::from_typed(&ids))
        }
    }

    pub fn get_amount<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let _input: BucketGetAmountInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        let info = BucketInfoSubstate::of(receiver, api)?;
        let amount = if info.resource_type.is_fungible() {
            FungibleBucket::liquid_amount(receiver, api)?
                + FungibleBucket::locked_amount(receiver, api)?
        } else {
            NonFungibleBucket::liquid_amount(receiver, api)?
                + NonFungibleBucket::locked_amount(receiver, api)?
        };

        Ok(IndexedScryptoValue::from_typed(&amount))
    }

    pub fn get_resource_address<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let _input: BucketGetResourceAddressInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        let info = BucketInfoSubstate::of(receiver, api)?;

        Ok(IndexedScryptoValue::from_typed(&info.resource_address))
    }

    pub fn create_proof<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let _input: BucketCreateProofInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        let info = BucketInfoSubstate::of(receiver, api)?;
        let node_id = if info.resource_type.is_fungible() {
            let amount = FungibleBucket::locked_amount(receiver, api)?
                + FungibleBucket::liquid_amount(receiver, api)?;

            let proof_info = ProofInfoSubstate {
                resource_address: info.resource_address,
                resource_type: info.resource_type,
                restricted: false,
            };
            let proof = FungibleBucket::lock_amount(receiver, amount, api)?;

            let proof_id = api.new_object(
                PROOF_BLUEPRINT,
                vec![
                    scrypto_encode(&proof_info).unwrap(),
                    scrypto_encode(&proof).unwrap(),
                    scrypto_encode(&NonFungibleProof::default()).unwrap(),
                ],
            )?;
            RENodeId::Object(proof_id)
        } else {
            let amount = NonFungibleBucket::locked_amount(receiver, api)?
                + NonFungibleBucket::liquid_amount(receiver, api)?;

            let proof_info = ProofInfoSubstate {
                resource_address: info.resource_address,
                resource_type: info.resource_type,
                restricted: false,
            };
            let proof = NonFungibleBucket::lock_amount(receiver, amount, api)?;
            let proof_id = api.new_object(
                PROOF_BLUEPRINT,
                vec![
                    scrypto_encode(&proof_info).unwrap(),
                    scrypto_encode(&FungibleProof::default()).unwrap(),
                    scrypto_encode(&proof).unwrap(),
                ],
            )?;
            RENodeId::Object(proof_id)
        };

        let proof_id = node_id.into();
        Ok(IndexedScryptoValue::from_typed(&Proof(proof_id)))
    }

    //===================
    // Protected method
    //===================

    // FIXME: set up auth

    pub fn lock_amount<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketLockAmountInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        FungibleBucket::lock_amount(receiver, input.amount, api)?;

        Ok(IndexedScryptoValue::from_typed(&()))
    }

    pub fn lock_non_fungibles<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketLockNonFungiblesInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        NonFungibleBucket::lock_non_fungibles(receiver, input.local_ids, api)?;

        Ok(IndexedScryptoValue::from_typed(&()))
    }

    pub fn unlock_amount<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketUnlockAmountInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        FungibleBucket::unlock_amount(receiver, input.amount, api)?;

        Ok(IndexedScryptoValue::from_typed(&()))
    }

    pub fn unlock_non_fungibles<Y>(
        receiver: RENodeId,
        input: IndexedScryptoValue,
        api: &mut Y,
    ) -> Result<IndexedScryptoValue, RuntimeError>
    where
        Y: KernelNodeApi + KernelSubstateApi + ClientApi<RuntimeError>,
    {
        let input: BucketUnlockNonFungiblesInput = input.as_typed().map_err(|e| {
            RuntimeError::InterpreterError(InterpreterError::ScryptoInputDecodeError(e))
        })?;

        NonFungibleBucket::unlock_non_fungibles(receiver, input.local_ids, api)?;

        Ok(IndexedScryptoValue::from_typed(&()))
    }
}
