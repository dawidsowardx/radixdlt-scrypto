use crate::model::{AuthRule};
use sbor::*;
use scrypto::engine::types::*;
use scrypto::rust::collections::*;
use scrypto::rust::string::String;
use scrypto::rust::vec::Vec;

/// A component is an instance of blueprint.
#[derive(Debug, Clone, TypeId, Encode, Decode)]
pub struct Component {
    package_id: PackageId,
    blueprint_name: String,
    state: Vec<u8>,
    sys_auth: HashMap<String, AuthRule>,
}

impl Component {
    pub fn new(
        package_id: PackageId,
        blueprint_name: String,
        state: Vec<u8>,
        sys_auth: HashMap<String, AuthRule>,
    ) -> Self {
        Self {
            package_id,
            blueprint_name,
            state,
            sys_auth,
        }
    }

    pub fn sys_auth(&self) -> &HashMap<String, AuthRule> {
        &self.sys_auth
    }

    pub fn package_id(&self) -> PackageId {
        self.package_id
    }

    pub fn blueprint_name(&self) -> &str {
        &self.blueprint_name
    }

    pub fn state(&self) -> &[u8] {
        &self.state
    }

    pub fn set_state(&mut self, new_state: Vec<u8>) {
        self.state = new_state;
    }
}
