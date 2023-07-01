use crate::blueprints::resource::*;
use crate::*;
#[cfg(feature = "radix_engine_fuzzing")]
use arbitrary::{Arbitrary, Result, Unstructured};
use radix_engine_common::data::manifest::model::ManifestAddressReservation;
use radix_engine_common::data::manifest::ManifestValue;
use radix_engine_common::data::scrypto::{ScryptoCustomTypeKind, ScryptoSchema, ScryptoValue};
use radix_engine_common::prelude::replace_self_package_address;
use radix_engine_common::prelude::*;
use radix_engine_interface::api::node_modules::auth::ToRoleEntry;
use radix_engine_interface::api::node_modules::metadata::MetadataInit;
use radix_engine_interface::api::node_modules::ModuleConfig;
use radix_engine_interface::types::NonFungibleData;
use sbor::rust::collections::{BTreeMap, BTreeSet};
use sbor::rust::string::String;
use sbor::rust::string::ToString;
use sbor::rust::vec::Vec;
use sbor::{generate_full_schema, LocalTypeIndex, TypeAggregator};

pub const NON_FUNGIBLE_RESOURCE_MANAGER_BLUEPRINT: &str = "NonFungibleResourceManager";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Default, Debug, Clone, Eq, PartialEq, ScryptoSbor, ManifestSbor)]
pub struct NonFungibleResourceFeatures {
    pub mintable: Option<MintableRoles<RoleDefinition>>,
    pub burnable: Option<BurnableRoles<RoleDefinition>>,
    pub freezable: Option<FreezableRoles<RoleDefinition>>,
    pub recallable: Option<RecallableRoles<RoleDefinition>>,
    pub restrict_withdraw: Option<WithdrawableRoles<RoleDefinition>>,
    pub restrict_deposit: Option<DepositableRoles<RoleDefinition>>,
    pub updatable_non_fungible_data: Option<UpdatableNonFungibleDataRoles<RoleDefinition>>,
}

impl NonFungibleResourceFeatures {
    pub fn single_locked_rule(access_rule: AccessRule) -> Self {
        Self {
            mintable: mintable! {
                minter => access_rule.clone(), locked;
                minter_updater => rule!(deny_all), locked;
            },
            burnable: burnable! {
                burner => access_rule.clone(), locked;
                burner_updater => rule!(deny_all), locked;
            },
            freezable: freezable! {
                freezer => access_rule.clone(), locked;
                freezer_updater => rule!(deny_all), locked;
            },
            recallable: recallable! {
                recaller => access_rule.clone(), locked;
                recaller_updater => rule!(deny_all), locked;
            },
            updatable_non_fungible_data: updatable_non_fungible_data! {
                non_fungible_data_updater => access_rule.clone(), locked;
                non_fungible_data_updater_updater => rule!(deny_all), locked;
            },
            restrict_withdraw: restrict_withdraw! {
                withdrawer => access_rule.clone(), locked;
                withdrawer_updater => rule!(deny_all), locked;
            },
            restrict_deposit: restrict_deposit! {
                depositor => access_rule.clone(), locked;
                depositor_updater => rule!(deny_all), locked;
            },
        }
    }

    pub fn to_features_and_roles(self) -> (Vec<&'static str>, RolesInit) {
        let mut features = Vec::new();
        let mut roles = RolesInit::new();

        if let Some(mintable) = self.mintable {
            features.push(MINT_FEATURE);
            roles.data.extend(mintable.to_role_init().data);
        }

        if let Some(burnable) = self.burnable {
            features.push(BURN_FEATURE);
            roles.data.extend(burnable.to_role_init().data);
        }

        if let Some(freezable) = self.freezable {
            features.push(VAULT_FREEZE_FEATURE);
            roles.data.extend(freezable.to_role_init().data);
        }

        if let Some(recallable) = self.recallable {
            features.push(VAULT_RECALL_FEATURE);
            roles.data.extend(recallable.to_role_init().data);
        }

        if let Some(restrict_withdraw) = self.restrict_withdraw {
            roles.data.extend(restrict_withdraw.to_role_init().data);
        }

        if let Some(restrict_deposit) = self.restrict_deposit {
            roles.data.extend(restrict_deposit.to_role_init().data);
        }

        if let Some(updatable_non_fungible_data) = self.updatable_non_fungible_data {
            roles
                .data
                .extend(updatable_non_fungible_data.to_role_init().data);
        }

        (features, roles)
    }
}

pub const NON_FUNGIBLE_RESOURCE_MANAGER_CREATE_IDENT: &str = "create";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerCreateInput {
    pub owner_role: OwnerRole,
    pub id_type: NonFungibleIdType,
    pub track_total_supply: bool,
    pub non_fungible_schema: NonFungibleDataSchema,
    pub resource_features: NonFungibleResourceFeatures,
    pub metadata: ModuleConfig<MetadataInit>,
    pub address_reservation: Option<GlobalAddressReservation>,
}

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ManifestSbor)]
pub struct NonFungibleResourceManagerCreateManifestInput {
    pub owner_role: OwnerRole,
    pub id_type: NonFungibleIdType,
    pub track_total_supply: bool,
    pub non_fungible_schema: NonFungibleDataSchema,
    pub resource_features: NonFungibleResourceFeatures,
    pub metadata: ModuleConfig<MetadataInit>,
    pub address_reservation: Option<ManifestAddressReservation>,
}

pub type NonFungibleResourceManagerCreateOutput = ResourceAddress;

pub const NON_FUNGIBLE_RESOURCE_MANAGER_CREATE_WITH_INITIAL_SUPPLY_IDENT: &str =
    "create_with_initial_supply";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerCreateWithInitialSupplyInput {
    pub owner_role: OwnerRole,
    pub id_type: NonFungibleIdType,
    pub track_total_supply: bool,
    pub non_fungible_schema: NonFungibleDataSchema,
    pub entries: BTreeMap<NonFungibleLocalId, (ScryptoValue,)>,
    pub resource_features: NonFungibleResourceFeatures,
    pub metadata: ModuleConfig<MetadataInit>,
    pub address_reservation: Option<GlobalAddressReservation>,
}

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ManifestSbor)]
pub struct NonFungibleResourceManagerCreateWithInitialSupplyManifestInput {
    pub owner_role: OwnerRole,
    pub id_type: NonFungibleIdType,
    pub track_total_supply: bool,
    pub non_fungible_schema: NonFungibleDataSchema,
    pub entries: BTreeMap<NonFungibleLocalId, (ManifestValue,)>,
    pub resource_features: NonFungibleResourceFeatures,
    pub metadata: ModuleConfig<MetadataInit>,
    pub address_reservation: Option<ManifestAddressReservation>,
}

pub type NonFungibleResourceManagerCreateWithInitialSupplyOutput = (ResourceAddress, Bucket);

pub const NON_FUNGIBLE_RESOURCE_MANAGER_CREATE_RUID_WITH_INITIAL_SUPPLY_IDENT: &str =
    "create_ruid_non_fungible_with_initial_supply";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerCreateRuidWithInitialSupplyInput {
    pub owner_role: OwnerRole,
    pub track_total_supply: bool,
    pub non_fungible_schema: NonFungibleDataSchema,
    pub entries: Vec<(ScryptoValue,)>,
    pub resource_features: NonFungibleResourceFeatures,
    pub metadata: ModuleConfig<MetadataInit>,
    pub address_reservation: Option<GlobalAddressReservation>,
}

pub type NonFungibleResourceManagerCreateRuidWithInitialSupplyOutput = (ResourceAddress, Bucket);

pub const NON_FUNGIBLE_RESOURCE_MANAGER_UPDATE_DATA_IDENT: &str = "update_non_fungible_data";

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerUpdateDataInput {
    pub id: NonFungibleLocalId,
    pub field_name: String,
    pub data: ScryptoValue,
}

pub type NonFungibleResourceManagerUpdateDataOutput = ();

pub const NON_FUNGIBLE_RESOURCE_MANAGER_EXISTS_IDENT: &str = "non_fungible_exists";

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerExistsInput {
    pub id: NonFungibleLocalId,
}

pub type NonFungibleResourceManagerExistsOutput = bool;

pub const NON_FUNGIBLE_RESOURCE_MANAGER_GET_NON_FUNGIBLE_IDENT: &str = "get_non_fungible";

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerGetNonFungibleInput {
    pub id: NonFungibleLocalId,
}

pub type NonFungibleResourceManagerGetNonFungibleOutput = ScryptoValue;

pub const NON_FUNGIBLE_RESOURCE_MANAGER_MINT_IDENT: &str = "mint";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ManifestSbor)]
pub struct NonFungibleResourceManagerMintManifestInput {
    pub entries: BTreeMap<NonFungibleLocalId, (ManifestValue,)>,
}

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerMintInput {
    pub entries: BTreeMap<NonFungibleLocalId, (ScryptoValue,)>,
}

pub type NonFungibleResourceManagerMintOutput = Bucket;

pub const NON_FUNGIBLE_RESOURCE_MANAGER_MINT_RUID_IDENT: &str = "mint_ruid";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ManifestSbor)]
pub struct NonFungibleResourceManagerMintRuidManifestInput {
    pub entries: Vec<(ManifestValue,)>,
}

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerMintRuidInput {
    pub entries: Vec<(ScryptoValue,)>,
}

pub type NonFungibleResourceManagerMintRuidOutput = Bucket;

pub const NON_FUNGIBLE_RESOURCE_MANAGER_MINT_SINGLE_RUID_IDENT: &str = "mint_single_ruid";

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct NonFungibleResourceManagerMintSingleRuidInput {
    pub entry: ScryptoValue,
}
pub type NonFungibleResourceManagerMintSingleRuidOutput = (Bucket, NonFungibleLocalId);

#[derive(Debug, Clone, PartialEq, Eq, ScryptoSbor, ManifestSbor)]
pub struct NonFungibleDataSchema {
    pub schema: ScryptoSchema,
    pub non_fungible: LocalTypeIndex,
    pub mutable_fields: BTreeSet<String>,
}

impl NonFungibleData for () {
    const MUTABLE_FIELDS: &'static [&'static str] = &[];
}

impl NonFungibleDataSchema {
    pub fn new_schema<N: NonFungibleData>() -> Self {
        let mut aggregator = TypeAggregator::<ScryptoCustomTypeKind>::new();
        let non_fungible_type = aggregator.add_child_type_and_descendents::<N>();
        let schema = generate_full_schema(aggregator);
        Self {
            schema,
            non_fungible: non_fungible_type,
            mutable_fields: N::MUTABLE_FIELDS.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn replace_self_package_address(&mut self, package_address: PackageAddress) {
        replace_self_package_address(&mut self.schema, package_address);
    }
}

#[cfg(feature = "radix_engine_fuzzing")]
impl<'a> Arbitrary<'a> for NonFungibleDataSchema {
    // At the moment I see no smart method to derive Arbitrary for type Schema, which is part of
    // ScryptoSchema, therefore implementing arbitrary by hand.
    // TODO: Introduce a method that genearates NonFungibleDataSchema in a truly random manner
    fn arbitrary(_u: &mut Unstructured<'a>) -> Result<Self> {
        Ok(Self::new_schema::<()>())
    }
}
