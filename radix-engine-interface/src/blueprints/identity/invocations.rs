use crate::blueprints::resource::*;
use crate::*;
#[cfg(feature = "radix_engine_fuzzing")]
use arbitrary::Arbitrary;
use radix_engine_common::types::ComponentAddress;
use sbor::rust::fmt::Debug;

pub const IDENTITY_BLUEPRINT: &str = "Identity";

pub const IDENTITY_CREATE_VIRTUAL_SECP256K1_ID: u8 = 0u8;
pub const IDENTITY_CREATE_VIRTUAL_ED25519_ID: u8 = 1u8;

pub const IDENTITY_CREATE_ADVANCED_IDENT: &str = "create_advanced";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor, ManifestSbor)]
pub struct IdentityCreateAdvancedInput {
    pub owner_role: OwnerRole,
}

pub type IdentityCreateAdvancedOutput = ComponentAddress;

pub const IDENTITY_CREATE_IDENT: &str = "create";

#[cfg_attr(feature = "radix_engine_fuzzing", derive(Arbitrary))]
#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor, ManifestSbor)]
pub struct IdentityCreateInput {}

pub type IdentityCreateOutput = (ComponentAddress, Bucket);

pub const IDENTITY_SECURIFY_IDENT: &str = "securify";

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor, ManifestSbor)]
pub struct IdentitySecurifyToSingleBadgeInput {}

pub type IdentitySecurifyToSingleBadgeOutput = Bucket;
