use sbor::*;
use scrypto::engine::types::*;
use scrypto::rust::cell::{Ref, RefCell, RefMut};
use scrypto::rust::collections::BTreeSet;
use scrypto::rust::rc::Rc;

use crate::model::{Bucket, BucketError};
use crate::model::{ResourceContainer, ResourceContainerError};

/// Represents an error when accessing a vault.
#[derive(Debug, Clone, PartialEq)]
pub enum VaultError {
    ResourceContainerError(ResourceContainerError),
    BucketError(BucketError),
}

/// A persistent resource container.
#[derive(Debug, TypeId, Encode, Decode)]
pub struct Vault {
    container: Rc<RefCell<ResourceContainer>>,
}

impl Vault {
    pub fn new(container: ResourceContainer) -> Self {
        Self {
            container: Rc::new(RefCell::new(container)),
        }
    }

    pub fn put(&mut self, other: Bucket) -> Result<(), VaultError> {
        self.borrow_container_mut()
            .put(other.into_container().map_err(VaultError::BucketError)?)
            .map_err(VaultError::ResourceContainerError)
    }

    pub fn take(&mut self, amount: Decimal) -> Result<Bucket, VaultError> {
        Ok(Bucket::new(
            self.borrow_container_mut()
                .take(amount)
                .map_err(VaultError::ResourceContainerError)?,
        ))
    }

    pub fn take_non_fungible(&mut self, id: &NonFungibleId) -> Result<Bucket, VaultError> {
        self.take_non_fungibles(&BTreeSet::from([id.clone()]))
    }

    pub fn take_non_fungibles(
        &mut self,
        ids: &BTreeSet<NonFungibleId>,
    ) -> Result<Bucket, VaultError> {
        Ok(Bucket::new(
            self.borrow_container_mut()
                .take_non_fungibles(ids)
                .map_err(VaultError::ResourceContainerError)?,
        ))
    }

    pub fn resource_def_id(&self) -> ResourceDefId {
        self.borrow_container().resource_def_id()
    }

    pub fn resource_type(&self) -> ResourceType {
        self.borrow_container().resource_type()
    }

    pub fn total_amount(&self) -> Decimal {
        self.borrow_container().total_amount()
    }

    pub fn total_ids(&self) -> Result<BTreeSet<NonFungibleId>, VaultError> {
        self.borrow_container()
            .total_ids()
            .map_err(VaultError::ResourceContainerError)
    }

    pub fn is_locked(&self) -> bool {
        self.borrow_container().is_locked()
    }

    pub fn is_empty(&self) -> bool {
        self.borrow_container().is_empty()
    }

    pub fn create_reference_for_proof(&self) -> Rc<RefCell<ResourceContainer>> {
        self.container.clone()
    }

    fn borrow_container(&self) -> Ref<ResourceContainer> {
        self.container.borrow()
    }

    fn borrow_container_mut(&mut self) -> RefMut<ResourceContainer> {
        self.container.borrow_mut()
    }
}
