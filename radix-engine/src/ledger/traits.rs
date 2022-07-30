use crate::engine::Substate;
use sbor::rust::collections::*;
use sbor::rust::vec::Vec;
use sbor::*;
use scrypto::crypto::*;
use scrypto::engine::types::*;

use crate::engine::SubstateId;

pub trait QueryableSubstateStore {
    fn get_kv_store_entries(
        &self,
        component_address: ComponentAddress,
        kv_store_id: &KeyValueStoreId,
    ) -> HashMap<Vec<u8>, Substate>;
}

#[derive(Debug, Clone, Hash, TypeId, Encode, Decode, PartialEq, Eq)]
pub struct OutputId {
    pub address: SubstateId,
    pub substate_hash: Hash,
    pub version: u32,
}

#[derive(Debug, Clone, Encode, Decode, TypeId, PartialEq, Eq)]
pub struct OutputValue {
    pub substate: Substate,
    pub version: u32,
}

pub trait ReadableSubstateStore {
    fn get_substate(&self, address: &SubstateId) -> Option<OutputValue>;
}

pub trait WriteableSubstateStore {
    fn put_substate(&mut self, address: SubstateId, substate: OutputValue);
}

pub trait SubstateStore: ReadableSubstateStore + WriteableSubstateStore {}

impl<T: ReadableSubstateStore + WriteableSubstateStore> SubstateStore for T {}
