use sbor::rust::boxed::Box;
use sbor::rust::cell::{RefCell, RefMut};
use sbor::rust::collections::*;
use sbor::rust::format;
use sbor::rust::marker::*;
use sbor::rust::ops::Deref;
use sbor::rust::string::String;
use sbor::rust::string::ToString;
use sbor::rust::vec;
use sbor::rust::vec::Vec;
use sbor::*;
use scrypto::buffer::scrypto_decode;
use scrypto::core::{SNodeRef, ScryptoActor};
use scrypto::engine::types::*;
use scrypto::resource::AuthZoneClearInput;
use scrypto::values::*;
use transaction::validation::*;

use crate::engine::*;
use crate::fee::*;
use crate::model::*;
use crate::wasm::*;

/// A call frame is the basic unit that forms a transaction call stack, which keeps track of the
/// owned objects by this function.
pub struct CallFrame<
    'p, // parent lifetime
    'g, // lifetime of values outliving all frames
    's, // Substate store lifetime
    W,  // WASM engine type
    I,  // WASM instance type
    C,  // Cost unit counter type
> where
    W: WasmEngine<I>,
    I: WasmInstance,
    C: CostUnitCounter,
{
    /// The transaction hash
    transaction_hash: Hash,
    /// The call depth
    depth: usize,
    /// Whether to show trace messages
    trace: bool,

    /// State track
    track: &'g mut Track<'s>,
    /// Wasm engine
    wasm_engine: &'g mut W,
    /// Wasm Instrumenter
    wasm_instrumenter: &'g mut WasmInstrumenter,

    /// Remaining cost unit counter
    cost_unit_counter: &'g mut C,
    /// Fee table
    fee_table: &'g FeeTable,

    id_allocator: &'g mut IdAllocator,

    /// All ref values accessible by this call frame. The value may be located in one of the following:
    /// 1. borrowed values
    /// 2. track
    value_refs: HashMap<RENodeId, REValueInfo>,

    /// Owned Values
    owned_values: HashMap<RENodeId, REValue>,
    auth_zone: Option<RefCell<AuthZone>>,

    /// Borrowed Values from call frames up the stack
    parent_values: Vec<&'p mut HashMap<RENodeId, REValue>>,
    caller_auth_zone: Option<&'p RefCell<AuthZone>>,

    phantom: PhantomData<I>,
}

#[macro_export]
macro_rules! trace {
    ( $self: expr, $level: expr, $msg: expr $( , $arg:expr )* ) => {
        #[cfg(not(feature = "alloc"))]
        if $self.trace {
            println!("{}[{:5}] {}", "  ".repeat($self.depth), $level, sbor::rust::format!($msg, $( $arg ),*));
        }
    };
}

fn verify_stored_value_update(
    old: &HashSet<RENodeId>,
    missing: &HashSet<RENodeId>,
) -> Result<(), RuntimeError> {
    // TODO: optimize intersection search
    for old_id in old.iter() {
        if !missing.contains(&old_id) {
            return Err(RuntimeError::StoredValueRemoved(old_id.clone()));
        }
    }

    for missing_id in missing.iter() {
        if !old.contains(missing_id) {
            return Err(RuntimeError::ValueNotFound(*missing_id));
        }
    }

    Ok(())
}

pub fn insert_non_root_nodes<'s>(track: &mut Track<'s>, values: HashMap<RENodeId, RENode>) {
    for (id, node) in values {
        match node {
            RENode::Vault(vault) => {
                let addr = SubstateId::Vault(id.into());
                track.create_uuid_value(addr, vault);
            }
            RENode::Component(component, component_state) => {
                let component_address = id.into();
                track.create_uuid_value(
                    SubstateId::ComponentInfo(component_address, false),
                    component,
                );
                track.create_uuid_value(
                    SubstateId::ComponentState(component_address),
                    component_state,
                );
            }
            RENode::KeyValueStore(store) => {
                let id = id.into();
                let substate_id = SubstateId::KeyValueStoreSpace(id);
                for (k, v) in store.store {
                    track.set_key_value(
                        substate_id.clone(),
                        k,
                        KeyValueStoreEntryWrapper(Some(v.raw)),
                    );
                }
            }
            _ => panic!("Invalid node being persisted: {:?}", node),
        }
    }
}

#[derive(Debug, Clone)]
pub struct REValueInfo {
    visible: bool,
    location: REValuePointer,
}

#[derive(Debug, Clone)]
pub enum REValuePointer {
    Stack {
        frame_id: Option<usize>,
        root: RENodeId,
        id: Option<RENodeId>,
    },
    Track(SubstateId),
}

impl REValuePointer {
    fn child(&self, child_id: RENodeId) -> REValuePointer {
        match self {
            REValuePointer::Stack { frame_id, root, .. } => REValuePointer::Stack {
                frame_id: frame_id.clone(),
                root: root.clone(),
                id: Option::Some(child_id),
            },
            REValuePointer::Track(..) => {
                let child_address = match child_id {
                    RENodeId::KeyValueStore(kv_store_id) => {
                        SubstateId::KeyValueStoreSpace(kv_store_id)
                    }
                    RENodeId::Vault(vault_id) => SubstateId::Vault(vault_id),
                    RENodeId::Component(component_id) => {
                        SubstateId::ComponentInfo(component_id, false)
                    }
                    _ => panic!("Unexpected"),
                };
                REValuePointer::Track(child_address)
            }
        }
    }

    fn borrow_native_ref<'p, 's>(
        &self, // TODO: Consider changing this to self
        owned_values: &mut HashMap<RENodeId, REValue>,
        borrowed_values: &mut Vec<&'p mut HashMap<RENodeId, REValue>>,
        track: &mut Track<'s>,
    ) -> RENativeValueRef {
        match self {
            REValuePointer::Stack { frame_id, root, id } => {
                let frame = if let Some(frame_id) = frame_id {
                    borrowed_values.get_mut(*frame_id).unwrap()
                } else {
                    owned_values
                };
                let re_value = frame.remove(root).expect("Should exist");
                RENativeValueRef::Stack(re_value, frame_id.clone(), root.clone(), id.clone())
            }
            REValuePointer::Track(substate_id) => {
                let value = track.take_value(substate_id.clone());
                RENativeValueRef::Track(substate_id.clone(), value)
            }
        }
    }

    fn to_ref<'f, 'p, 's>(
        &self,
        owned_values: &'f HashMap<RENodeId, REValue>,
        borrowed_values: &'f Vec<&'p mut HashMap<RENodeId, REValue>>,
        track: &'f Track<'s>,
    ) -> REValueRef<'f, 's> {
        match self {
            REValuePointer::Stack { frame_id, root, id } => {
                let frame = if let Some(frame_id) = frame_id {
                    borrowed_values.get(*frame_id).unwrap()
                } else {
                    owned_values
                };
                REValueRef::Stack(frame.get(root).unwrap(), id.clone())
            }
            REValuePointer::Track(substate_id) => REValueRef::Track(track, substate_id.clone()),
        }
    }

    fn to_ref_mut<'f, 'p, 's>(
        &self,
        owned_values: &'f mut HashMap<RENodeId, REValue>,
        borrowed_values: &'f mut Vec<&'p mut HashMap<RENodeId, REValue>>,
        track: &'f mut Track<'s>,
    ) -> REValueRefMut<'f, 's> {
        match self {
            REValuePointer::Stack { frame_id, root, id } => {
                let frame = if let Some(frame_id) = frame_id {
                    borrowed_values.get_mut(*frame_id).unwrap()
                } else {
                    owned_values
                };
                REValueRefMut::Stack(frame.get_mut(root).unwrap(), id.clone())
            }
            REValuePointer::Track(substate_id) => REValueRefMut::Track(track, substate_id.clone()),
        }
    }
}

pub enum RENativeValueRef {
    Stack(REValue, Option<usize>, RENodeId, Option<RENodeId>),
    Track(SubstateId, Substate),
}

impl RENativeValueRef {
    pub fn bucket(&mut self) -> &mut Bucket {
        match self {
            RENativeValueRef::Stack(root, _frame_id, _root_id, maybe_child) => {
                match root.get_node_mut(maybe_child.as_ref()) {
                    RENode::Bucket(bucket) => bucket,
                    _ => panic!("Expecting to be a bucket"),
                }
            }
            _ => panic!("Expecting to be a bucket"),
        }
    }

    pub fn proof(&mut self) -> &mut Proof {
        match self {
            RENativeValueRef::Stack(ref mut root, _frame_id, _root_id, maybe_child) => {
                match root.get_node_mut(maybe_child.as_ref()) {
                    RENode::Proof(proof) => proof,
                    _ => panic!("Expecting to be a proof"),
                }
            }
            _ => panic!("Expecting to be a proof"),
        }
    }

    pub fn worktop(&mut self) -> &mut Worktop {
        match self {
            RENativeValueRef::Stack(ref mut root, _frame_id, _root_id, maybe_child) => {
                match root.get_node_mut(maybe_child.as_ref()) {
                    RENode::Worktop(worktop) => worktop,
                    _ => panic!("Expecting to be a worktop"),
                }
            }
            _ => panic!("Expecting to be a worktop"),
        }
    }

    pub fn vault(&mut self) -> &mut Vault {
        match self {
            RENativeValueRef::Stack(root, _frame_id, _root_id, maybe_child) => {
                root.get_node_mut(maybe_child.as_ref()).vault_mut()
            }
            RENativeValueRef::Track(_address, value) => value.vault_mut(),
        }
    }

    pub fn system(&mut self) -> &mut System {
        match self {
            RENativeValueRef::Track(_address, value) => value.system_mut(),
            _ => panic!("Expecting to be system"),
        }
    }

    pub fn component(&mut self) -> &mut Component {
        match self {
            RENativeValueRef::Stack(root, _frame_id, _root_id, maybe_child) => {
                root.get_node_mut(maybe_child.as_ref()).component_mut()
            }
            _ => panic!("Expecting to be a component"),
        }
    }

    pub fn package(&mut self) -> &ValidatedPackage {
        match self {
            RENativeValueRef::Track(_address, value) => value.package(),
            _ => panic!("Expecting to be tracked"),
        }
    }

    pub fn resource_manager(&mut self) -> &mut ResourceManager {
        match self {
            RENativeValueRef::Stack(value, _frame_id, _root_id, maybe_child) => value
                .get_node_mut(maybe_child.as_ref())
                .resource_manager_mut(),
            RENativeValueRef::Track(_address, value) => value.resource_manager_mut(),
        }
    }

    pub fn return_to_location<'a, 'p, 's>(
        self,
        owned_values: &'a mut HashMap<RENodeId, REValue>,
        borrowed_values: &'a mut Vec<&'p mut HashMap<RENodeId, REValue>>,
        track: &mut Track<'s>,
    ) {
        match self {
            RENativeValueRef::Stack(owned, frame_id, value_id, ..) => {
                let frame = if let Some(frame_id) = frame_id {
                    borrowed_values.get_mut(frame_id).unwrap()
                } else {
                    owned_values
                };
                frame.insert(value_id, owned);
            }
            RENativeValueRef::Track(substate_id, value) => track.write_value(substate_id, value),
        }
    }
}

pub enum REValueRef<'f, 's> {
    Stack(&'f REValue, Option<RENodeId>),
    Track(&'f Track<'s>, SubstateId),
}

impl<'f, 's> REValueRef<'f, 's> {
    pub fn vault(&self) -> &Vault {
        match self {
            REValueRef::Stack(value, id) => id
                .as_ref()
                .map_or(value.root(), |v| value.non_root(v))
                .vault(),
            REValueRef::Track(track, substate_id) => track.read_value(substate_id.clone()).vault(),
        }
    }

    pub fn system(&self) -> &System {
        match self {
            REValueRef::Stack(value, id) => id
                .as_ref()
                .map_or(value.root(), |v| value.non_root(v))
                .system(),
            REValueRef::Track(track, substate_id) => track.read_value(substate_id.clone()).system(),
        }
    }

    pub fn resource_manager(&self) -> &ResourceManager {
        match self {
            REValueRef::Stack(value, id) => id
                .as_ref()
                .map_or(value.root(), |v| value.non_root(v))
                .resource_manager(),
            REValueRef::Track(track, substate_id) => {
                track.read_value(substate_id.clone()).resource_manager()
            }
        }
    }

    pub fn component_state(&self) -> &ComponentState {
        match self {
            REValueRef::Stack(value, id) => id
                .as_ref()
                .map_or(value.root(), |v| value.non_root(v))
                .component_state(),
            REValueRef::Track(track, substate_id) => {
                let component_state_address = match substate_id {
                    SubstateId::ComponentInfo(substate_id, ..) => {
                        SubstateId::ComponentState(*substate_id)
                    }
                    _ => panic!("Unexpected"),
                };
                track.read_value(component_state_address).component_state()
            }
        }
    }

    pub fn component(&self) -> &Component {
        match self {
            REValueRef::Stack(value, id) => id
                .as_ref()
                .map_or(value.root(), |v| value.non_root(v))
                .component(),
            REValueRef::Track(track, substate_id) => {
                track.read_value(substate_id.clone()).component()
            }
        }
    }

    pub fn package(&self) -> &ValidatedPackage {
        match self {
            REValueRef::Stack(value, id) => id
                .as_ref()
                .map_or(value.root(), |v| value.non_root(v))
                .package(),
            REValueRef::Track(track, substate_id) => {
                track.read_value(substate_id.clone()).package()
            }
        }
    }
}

pub enum REValueRefMut<'f, 's> {
    Stack(&'f mut REValue, Option<RENodeId>),
    Track(&'f mut Track<'s>, SubstateId),
}

impl<'f, 's> REValueRefMut<'f, 's> {
    pub fn kv_store_put(
        &mut self,
        key: Vec<u8>,
        value: ScryptoValue,
        to_store: HashMap<RENodeId, REValue>,
    ) {
        match self {
            REValueRefMut::Stack(re_value, id) => {
                re_value
                    .get_node_mut(id.as_ref())
                    .kv_store_mut()
                    .put(key, value);
                for (id, val) in to_store {
                    re_value.insert_non_root_nodes(val.to_nodes(id));
                }
            }
            REValueRefMut::Track(track, substate_id) => {
                track.set_key_value(
                    substate_id.clone(),
                    key,
                    Substate::KeyValueStoreEntry(KeyValueStoreEntryWrapper(Some(value.raw))),
                );
                for (id, val) in to_store {
                    insert_non_root_nodes(track, val.to_nodes(id));
                }
            }
        }
    }

    pub fn kv_store_get(&mut self, key: &[u8]) -> ScryptoValue {
        let wrapper = match self {
            REValueRefMut::Stack(re_value, id) => {
                let store = re_value.get_node_mut(id.as_ref()).kv_store_mut();
                store
                    .get(key)
                    .map(|v| KeyValueStoreEntryWrapper(Some(v.raw)))
                    .unwrap_or(KeyValueStoreEntryWrapper(None))
            }
            REValueRefMut::Track(track, substate_id) => {
                let substate_value = track.read_key_value(substate_id.clone(), key.to_vec());
                substate_value.into()
            }
        };

        // TODO: Cleanup after adding polymorphism support for SBOR
        // For now, we have to use `Vec<u8>` within `KeyValueStoreEntryWrapper`
        // and apply the following ugly conversion.
        let value = wrapper.0.map_or(
            Value::Option {
                value: Box::new(Option::None),
            },
            |v| Value::Option {
                value: Box::new(Some(decode_any(&v).unwrap())),
            },
        );

        ScryptoValue::from_value(value).unwrap()
    }

    pub fn non_fungible_get(&mut self, id: &NonFungibleId) -> ScryptoValue {
        let wrapper = match self {
            REValueRefMut::Stack(value, re_id) => {
                let non_fungible_set = re_id
                    .as_ref()
                    .map_or(value.root(), |v| value.non_root(v))
                    .non_fungibles();
                non_fungible_set
                    .get(id)
                    .cloned()
                    .map(|v| NonFungibleWrapper(Some(v)))
                    .unwrap_or(NonFungibleWrapper(None))
            }
            REValueRefMut::Track(track, substate_id) => {
                let resource_address: ResourceAddress = substate_id.clone().into();
                let substate_value = track
                    .read_key_value(SubstateId::NonFungibleSpace(resource_address), id.to_vec());
                substate_value.into()
            }
        };

        ScryptoValue::from_typed(&wrapper)
    }

    pub fn non_fungible_remove(&mut self, id: &NonFungibleId) {
        match self {
            REValueRefMut::Stack(..) => {
                panic!("Not supported");
            }
            REValueRefMut::Track(track, substate_id) => {
                let resource_address: ResourceAddress = substate_id.clone().into();
                track.set_key_value(
                    SubstateId::NonFungibleSpace(resource_address),
                    id.to_vec(),
                    Substate::NonFungible(NonFungibleWrapper(None)),
                );
            }
        }
    }

    pub fn non_fungible_put(&mut self, id: NonFungibleId, value: ScryptoValue) {
        match self {
            REValueRefMut::Stack(re_value, re_id) => {
                let wrapper: NonFungibleWrapper =
                    scrypto_decode(&value.raw).expect("Should not fail.");

                let non_fungible_set = re_value.get_node_mut(re_id.as_ref()).non_fungibles_mut();
                if let Some(non_fungible) = wrapper.0 {
                    non_fungible_set.insert(id, non_fungible);
                } else {
                    panic!("TODO: invalidate this code path and possibly consolidate `non_fungible_remove` and `non_fungible_put`")
                }
            }
            REValueRefMut::Track(track, substate_id) => {
                let wrapper: NonFungibleWrapper =
                    scrypto_decode(&value.raw).expect("Should not fail.");
                let resource_address: ResourceAddress = substate_id.clone().into();
                track.set_key_value(
                    SubstateId::NonFungibleSpace(resource_address),
                    id.to_vec(),
                    Substate::NonFungible(wrapper),
                );
            }
        }
    }

    pub fn component_state_set(
        &mut self,
        value: ScryptoValue,
        to_store: HashMap<RENodeId, REValue>,
    ) {
        match self {
            REValueRefMut::Stack(re_value, id) => {
                let component_state = re_value.get_node_mut(id.as_ref()).component_state_mut();
                component_state.set_state(value.raw);
                for (id, val) in to_store {
                    re_value.insert_non_root_nodes(val.to_nodes(id));
                }
            }
            REValueRefMut::Track(track, substate_id) => {
                let component_address: ComponentAddress = substate_id.clone().into();
                track.write_value(
                    SubstateId::ComponentState(component_address),
                    ComponentState::new(value.raw),
                );
                for (id, val) in to_store {
                    insert_non_root_nodes(track, val.to_nodes(id));
                }
            }
        }
    }

    pub fn component(&mut self) -> &Component {
        match self {
            REValueRefMut::Stack(re_value, id) => re_value.get_node_mut(id.as_ref()).component(),
            REValueRefMut::Track(track, substate_id) => {
                let component_val = track.read_value(substate_id.clone());
                component_val.component()
            }
        }
    }

    pub fn component_state(&mut self) -> &ComponentState {
        match self {
            REValueRefMut::Stack(re_value, id) => {
                re_value.get_node_mut(id.as_ref()).component_state()
            }
            REValueRefMut::Track(track, substate_id) => {
                let component_address: ComponentAddress = substate_id.clone().into();
                let component_state_address = SubstateId::ComponentState(component_address);
                let component_val = track.read_value(component_state_address.clone());
                component_val.component_state()
            }
        }
    }
}

pub enum StaticSNodeState {
    Package,
    Resource,
    TransactionProcessor,
}

pub enum SNodeExecution<'a> {
    Static(StaticSNodeState),
    Consumed(RENodeId),
    AuthZone(RefMut<'a, AuthZone>),
    ValueRef(RENodeId),
    Scrypto(ScryptoActorInfo, PackageAddress),
}

impl<'p, 'g, 's, W, I, C> CallFrame<'p, 'g, 's, W, I, C>
where
    W: WasmEngine<I>,
    I: WasmInstance,
    C: CostUnitCounter,
{
    pub fn new_root(
        verbose: bool,
        transaction_hash: Hash,
        signer_public_keys: Vec<EcdsaPublicKey>,
        is_system: bool,
        id_allocator: &'g mut IdAllocator,
        track: &'g mut Track<'s>,
        wasm_engine: &'g mut W,
        wasm_instrumenter: &'g mut WasmInstrumenter,
        cost_unit_counter: &'g mut C,
        fee_table: &'g FeeTable,
    ) -> Self {
        // TODO: Cleanup initialization of authzone
        let signer_non_fungible_ids: BTreeSet<NonFungibleId> = signer_public_keys
            .clone()
            .into_iter()
            .map(|public_key| NonFungibleId::from_bytes(public_key.to_vec()))
            .collect();

        let mut initial_auth_zone_proofs = Vec::new();
        if !signer_non_fungible_ids.is_empty() {
            // Proofs can't be zero amount
            let mut ecdsa_bucket = Bucket::new(ResourceContainer::new_non_fungible(
                ECDSA_TOKEN,
                signer_non_fungible_ids,
            ));
            let ecdsa_proof = ecdsa_bucket.create_proof(ECDSA_TOKEN_BUCKET_ID).unwrap();
            initial_auth_zone_proofs.push(ecdsa_proof);
        }

        if is_system {
            let id = [NonFungibleId::from_u32(0)].into_iter().collect();
            let mut system_bucket =
                Bucket::new(ResourceContainer::new_non_fungible(SYSTEM_TOKEN, id));
            let system_proof = system_bucket
                .create_proof(id_allocator.new_bucket_id().unwrap())
                .unwrap();
            initial_auth_zone_proofs.push(system_proof);
        }

        Self::new(
            transaction_hash,
            0,
            verbose,
            id_allocator,
            track,
            wasm_engine,
            wasm_instrumenter,
            cost_unit_counter,
            fee_table,
            Some(RefCell::new(AuthZone::new_with_proofs(
                initial_auth_zone_proofs,
            ))),
            HashMap::new(),
            HashMap::new(),
            Vec::new(),
            None,
        )
    }

    pub fn new(
        transaction_hash: Hash,
        depth: usize,
        trace: bool,
        id_allocator: &'g mut IdAllocator,
        track: &'g mut Track<'s>,
        wasm_engine: &'g mut W,
        wasm_instrumenter: &'g mut WasmInstrumenter,
        cost_unit_counter: &'g mut C,
        fee_table: &'g FeeTable,
        auth_zone: Option<RefCell<AuthZone>>,
        owned_values: HashMap<RENodeId, REValue>,
        value_refs: HashMap<RENodeId, REValueInfo>,
        parent_values: Vec<&'p mut HashMap<RENodeId, REValue>>,
        caller_auth_zone: Option<&'p RefCell<AuthZone>>,
    ) -> Self {
        Self {
            transaction_hash,
            depth,
            trace,
            id_allocator,
            track,
            wasm_engine,
            wasm_instrumenter,
            cost_unit_counter,
            fee_table,
            owned_values,
            value_refs,
            parent_values,
            auth_zone,
            caller_auth_zone,
            phantom: PhantomData,
        }
    }

    fn drop_owned_values(&mut self) -> Result<(), RuntimeError> {
        let values = self
            .owned_values
            .drain()
            .map(|(_id, value)| value)
            .collect();
        RENode::drop_values(values).map_err(|e| RuntimeError::DropFailure(e))
    }

    fn process_call_data(validated: &ScryptoValue) -> Result<(), RuntimeError> {
        if !validated.kv_store_ids.is_empty() {
            return Err(RuntimeError::KeyValueStoreNotAllowed);
        }
        if !validated.vault_ids.is_empty() {
            return Err(RuntimeError::VaultNotAllowed);
        }
        Ok(())
    }

    fn process_return_data(
        &mut self,
        from: SNodeRef,
        validated: &ScryptoValue,
    ) -> Result<(), RuntimeError> {
        if !validated.kv_store_ids.is_empty() {
            return Err(RuntimeError::KeyValueStoreNotAllowed);
        }

        // Allow vaults to be returned from ResourceStatic
        // TODO: Should we allow vaults to be returned by any component?
        if !matches!(from, SNodeRef::ResourceRef(_)) {
            if !validated.vault_ids.is_empty() {
                return Err(RuntimeError::VaultNotAllowed);
            }
        }

        Ok(())
    }

    pub fn run(
        &mut self,
        snode_ref: SNodeRef, // TODO: Remove, abstractions between invoke_snode() and run() are a bit messy right now
        execution: SNodeExecution<'p>,
        fn_ident: &str,
        input: ScryptoValue,
    ) -> Result<(ScryptoValue, HashMap<RENodeId, REValue>), RuntimeError> {
        trace!(
            self,
            Level::Debug,
            "Run started! Depth: {}, Remaining cost units: {}",
            self.depth,
            self.cost_unit_counter.balance()
        );

        self.cost_unit_counter
            .consume(
                self.fee_table.function_cost(&snode_ref, fn_ident, &input),
                "run_function",
            )
            .map_err(RuntimeError::CostingError)?;

        let output = {
            let rtn = match execution {
                SNodeExecution::Static(state) => match state {
                    StaticSNodeState::TransactionProcessor => TransactionProcessor::static_main(
                        fn_ident, input, self,
                    )
                    .map_err(|e| match e {
                        TransactionProcessorError::InvalidRequestData(_) => panic!("Illegal state"),
                        TransactionProcessorError::InvalidMethod => panic!("Illegal state"),
                        TransactionProcessorError::RuntimeError(e) => e,
                    }),
                    StaticSNodeState::Package => {
                        ValidatedPackage::static_main(fn_ident, input, self)
                            .map_err(RuntimeError::PackageError)
                    }
                    StaticSNodeState::Resource => {
                        ResourceManager::static_main(fn_ident, input, self)
                            .map_err(RuntimeError::ResourceManagerError)
                    }
                },
                SNodeExecution::Consumed(value_id) => match value_id {
                    RENodeId::Bucket(..) => Bucket::consuming_main(value_id, fn_ident, input, self)
                        .map_err(RuntimeError::BucketError),
                    RENodeId::Proof(..) => Proof::main_consume(value_id, fn_ident, input, self)
                        .map_err(RuntimeError::ProofError),
                    RENodeId::Component(..) => {
                        Component::main_consume(value_id, fn_ident, input, self)
                            .map_err(RuntimeError::ComponentError)
                    }
                    _ => panic!("Unexpected"),
                },
                SNodeExecution::AuthZone(mut auth_zone) => auth_zone
                    .main(fn_ident, input, self)
                    .map_err(RuntimeError::AuthZoneError),
                SNodeExecution::ValueRef(value_id) => match value_id {
                    RENodeId::Bucket(bucket_id) => Bucket::main(bucket_id, fn_ident, input, self)
                        .map_err(RuntimeError::BucketError),
                    RENodeId::Proof(..) => Proof::main(value_id, fn_ident, input, self)
                        .map_err(RuntimeError::ProofError),
                    RENodeId::Worktop => Worktop::main(value_id, fn_ident, input, self)
                        .map_err(RuntimeError::WorktopError),
                    RENodeId::Vault(vault_id) => Vault::main(vault_id, fn_ident, input, self)
                        .map_err(RuntimeError::VaultError),
                    RENodeId::Component(..) => Component::main(value_id, fn_ident, input, self)
                        .map_err(RuntimeError::ComponentError),
                    RENodeId::Resource(resource_address) => {
                        ResourceManager::main(resource_address, fn_ident, input, self)
                            .map_err(RuntimeError::ResourceManagerError)
                    }
                    RENodeId::System => {
                        System::main(fn_ident, input, self).map_err(RuntimeError::SystemError)
                    }
                    _ => panic!("Unexpected"),
                },
                SNodeExecution::Scrypto(ref actor, package_address) => {
                    let output = {
                        let package = self.track.read_value(package_address).package();
                        let wasm_metering_params = self.fee_table.wasm_metering_params();
                        let instrumented_code = self
                            .wasm_instrumenter
                            .instrument(package.code(), &wasm_metering_params);
                        let mut instance = self.wasm_engine.instantiate(instrumented_code);
                        let blueprint_abi = package
                            .blueprint_abi(actor.blueprint_name())
                            .expect("Blueprint should exist");
                        let export_name = &blueprint_abi
                            .get_fn_abi(fn_ident)
                            .unwrap()
                            .export_name
                            .to_string();
                        let mut runtime: Box<dyn WasmRuntime> =
                            Box::new(RadixEngineWasmRuntime::new(actor.clone(), self));
                        instance
                            .invoke_export(&export_name, &input, &mut runtime)
                            .map_err(|e| match e {
                                // Flatten error code for more readable transaction receipt
                                InvokeError::RuntimeError(e) => e,
                                e @ _ => RuntimeError::InvokeError(e.into()),
                            })?
                    };

                    let package = self.track.read_value(package_address).package();
                    let blueprint_abi = package
                        .blueprint_abi(actor.blueprint_name())
                        .expect("Blueprint should exist");
                    let fn_abi = blueprint_abi.get_fn_abi(fn_ident).unwrap();
                    if !fn_abi.output.matches(&output.dom) {
                        Err(RuntimeError::InvalidFnOutput {
                            fn_ident: fn_ident.to_string(),
                            output: output.dom,
                        })
                    } else {
                        Ok(output)
                    }
                }
            }?;

            rtn
        };

        // Prevent vaults/kvstores from being returned
        self.process_return_data(snode_ref, &output)?;

        // Take values to return
        let values_to_take = output.value_ids();
        let (taken_values, mut missing) = self.take_available_values(values_to_take, false)?;
        let first_missing_value = missing.drain().nth(0);
        if let Some(missing_value) = first_missing_value {
            return Err(RuntimeError::ValueNotFound(missing_value));
        }

        // drop proofs and check resource leak
        if self.auth_zone.is_some() {
            self.invoke_snode(
                SNodeRef::AuthZoneRef,
                "clear".to_string(),
                ScryptoValue::from_typed(&AuthZoneClearInput {}),
            )?;
        }
        self.drop_owned_values()?;

        trace!(
            self,
            Level::Debug,
            "Run finished! Remaining cost units: {}",
            self.cost_unit_counter().balance()
        );

        Ok((output, taken_values))
    }

    fn take_available_values(
        &mut self,
        value_ids: HashSet<RENodeId>,
        persist_only: bool,
    ) -> Result<(HashMap<RENodeId, REValue>, HashSet<RENodeId>), RuntimeError> {
        let (taken, missing) = {
            let mut taken_values = HashMap::new();
            let mut missing_values = HashSet::new();

            for id in value_ids {
                let maybe = self.owned_values.remove(&id);
                if let Some(value) = maybe {
                    value.root().verify_can_move()?;
                    if persist_only {
                        value.root().verify_can_persist()?;
                    }
                    taken_values.insert(id, value);
                } else {
                    missing_values.insert(id);
                }
            }

            (taken_values, missing_values)
        };

        // Moved values must have their references removed
        for (id, value) in &taken {
            self.value_refs.remove(id);
            for (id, ..) in &value.non_root_nodes {
                self.value_refs.remove(id);
            }
        }

        Ok((taken, missing))
    }

    fn read_value_internal(
        &mut self,
        substate_id: &SubstateId,
    ) -> Result<(REValuePointer, ScryptoValue), RuntimeError> {
        let value_id = substate_id.get_node_id();

        // Get location
        // Note this must be run AFTER values are taken, otherwise there would be inconsistent readable_values state
        let (value_info, address_borrowed) = self
            .value_refs
            .get(&value_id)
            .cloned()
            .map(|v| (v, None))
            .or_else(|| {
                // Allow global read access to any component info
                if let SubstateId::ComponentInfo(component_address, ..) = substate_id {
                    if self.owned_values.contains_key(&value_id) {
                        return Some((
                            REValueInfo {
                                location: REValuePointer::Stack {
                                    frame_id: Option::None,
                                    root: value_id.clone(),
                                    id: Option::None,
                                },
                                visible: true,
                            },
                            None,
                        ));
                    } else if self
                        .track
                        .acquire_lock(
                            SubstateId::ComponentInfo(*component_address, true),
                            false,
                            false,
                        )
                        .is_ok()
                    {
                        return Some((
                            REValueInfo {
                                location: REValuePointer::Track(SubstateId::ComponentInfo(
                                    *component_address,
                                    true,
                                )),
                                visible: true,
                            },
                            Some(component_address),
                        ));
                    }
                }

                None
            })
            .ok_or_else(|| RuntimeError::InvalidDataAccess(value_id))?;
        if !value_info.visible {
            return Err(RuntimeError::InvalidDataAccess(value_id));
        }
        let location = &value_info.location;

        // Read current value
        let current_value = {
            let value_ref = location.to_ref_mut(
                &mut self.owned_values,
                &mut self.parent_values,
                &mut self.track,
            );
            substate_id.read_scrypto_value(value_ref)?
        };

        // TODO: Remove, currently a hack to allow for global component info retrieval
        if let Some(component_address) = address_borrowed {
            self.track
                .release_lock(SubstateId::ComponentInfo(*component_address, true), false);
        }

        Ok((location.clone(), current_value))
    }

    /// Creates a new package ID.
    pub fn new_package_address(&mut self) -> PackageAddress {
        // Security Alert: ensure ID allocating will practically never fail
        let package_address = self
            .id_allocator
            .new_package_address(self.transaction_hash)
            .unwrap();
        package_address
    }

    /// Creates a new component address.
    pub fn new_component_address(&mut self, component: &Component) -> ComponentAddress {
        let component_address = self
            .id_allocator
            .new_component_address(
                self.transaction_hash,
                &component.package_address(),
                component.blueprint_name(),
            )
            .unwrap();
        component_address
    }

    /// Creates a new resource address.
    pub fn new_resource_address(&mut self) -> ResourceAddress {
        let resource_address = self
            .id_allocator
            .new_resource_address(self.transaction_hash)
            .unwrap();
        resource_address
    }

    /// Creates a new UUID.
    pub fn new_uuid(&mut self) -> u128 {
        self.id_allocator.new_uuid(self.transaction_hash).unwrap()
    }

    /// Creates a new bucket ID.
    pub fn new_bucket_id(&mut self) -> BucketId {
        self.id_allocator.new_bucket_id().unwrap()
    }

    /// Creates a new vault ID.
    pub fn new_vault_id(&mut self) -> VaultId {
        self.id_allocator
            .new_vault_id(self.transaction_hash)
            .unwrap()
    }

    /// Creates a new reference id.
    pub fn new_proof_id(&mut self) -> ProofId {
        self.id_allocator.new_proof_id().unwrap()
    }

    /// Creates a new map id.
    pub fn new_kv_store_id(&mut self) -> KeyValueStoreId {
        self.id_allocator
            .new_kv_store_id(self.transaction_hash)
            .unwrap()
    }
}

impl<'p, 'g, 's, W, I, C> SystemApi<'p, 's, W, I, C> for CallFrame<'p, 'g, 's, W, I, C>
where
    W: WasmEngine<I>,
    I: WasmInstance,
    C: CostUnitCounter,
{
    fn invoke_snode(
        &mut self,
        snode_ref: SNodeRef,
        fn_ident: String,
        input: ScryptoValue,
    ) -> Result<ScryptoValue, RuntimeError> {
        trace!(
            self,
            Level::Debug,
            "Invoking: {:?} {:?}",
            snode_ref,
            &fn_ident
        );

        if self.depth == MAX_CALL_DEPTH {
            return Err(RuntimeError::MaxCallDepthLimitReached);
        }

        // TODO: find a better way to handle this
        let is_pay_fee = matches!(snode_ref, SNodeRef::VaultRef(..)) && &fn_ident == "pay_fee";

        self.cost_unit_counter
            .consume(
                self.fee_table
                    .system_api_cost(SystemApiCostingEntry::InvokeFunction {
                        receiver: &snode_ref,
                        input: &input,
                    }),
                "invoke_function",
            )
            .map_err(RuntimeError::CostingError)?;

        // Prevent vaults/kvstores from being moved
        Self::process_call_data(&input)?;

        // Figure out what buckets and proofs to move from this process
        let values_to_take = input.value_ids();
        let (taken_values, mut missing) = self.take_available_values(values_to_take, false)?;
        let first_missing_value = missing.drain().nth(0);
        if let Some(missing_value) = first_missing_value {
            return Err(RuntimeError::ValueNotFound(missing_value));
        }

        let mut next_owned_values = HashMap::new();

        // Internal state update to taken values
        for (id, mut value) in taken_values {
            trace!(self, Level::Debug, "Sending value: {:?}", value);
            match &mut value.root_mut() {
                RENode::Proof(proof) => proof.change_to_restricted(),
                _ => {}
            }
            next_owned_values.insert(id, value);
        }

        let mut locked_values = HashSet::new();
        let mut value_refs = HashMap::new();

        // Authorization and state load
        let (loaded_snode, method_auths) = match &snode_ref {
            SNodeRef::TransactionProcessor => {
                // FIXME: only TransactionExecutor can invoke this function
                Ok((
                    SNodeExecution::Static(StaticSNodeState::TransactionProcessor),
                    vec![],
                ))
            }
            SNodeRef::PackageStatic => {
                Ok((SNodeExecution::Static(StaticSNodeState::Package), vec![]))
            }
            SNodeRef::ResourceStatic => {
                Ok((SNodeExecution::Static(StaticSNodeState::Resource), vec![]))
            }
            SNodeRef::Consumed(value_id) => {
                let value = self
                    .owned_values
                    .remove(value_id)
                    .ok_or(RuntimeError::ValueNotFound(*value_id))?;

                let method_auths = match &value.root() {
                    RENode::Bucket(bucket) => {
                        let resource_address = bucket.resource_address();
                        self.track
                            .acquire_lock(resource_address, true, false)
                            .expect("Should not fail.");
                        locked_values.insert(resource_address.clone().into());
                        let resource_manager =
                            self.track.read_value(resource_address).resource_manager();
                        let method_auth = resource_manager.get_consuming_bucket_auth(&fn_ident);
                        value_refs.insert(
                            RENodeId::Resource(resource_address),
                            REValueInfo {
                                location: REValuePointer::Track(SubstateId::ResourceManager(
                                    resource_address,
                                )),
                                visible: true,
                            },
                        );
                        vec![method_auth.clone()]
                    }
                    RENode::Proof(_) => vec![],
                    RENode::Component(component, ..) => {
                        let package_address = component.package_address();
                        self.track
                            .acquire_lock(package_address, false, false)
                            .expect("Should not fail.");
                        locked_values.insert(package_address.clone().into());
                        value_refs.insert(
                            RENodeId::Package(package_address),
                            REValueInfo {
                                location: REValuePointer::Track(SubstateId::Package(
                                    package_address,
                                )),
                                visible: true,
                            },
                        );
                        vec![]
                    }
                    _ => return Err(RuntimeError::MethodDoesNotExist(fn_ident.clone())),
                };

                next_owned_values.insert(*value_id, value);

                Ok((SNodeExecution::Consumed(*value_id), method_auths))
            }
            SNodeRef::SystemRef => {
                self.track
                    .acquire_lock(SubstateId::System, true, false)
                    .expect("System access should never fail");
                locked_values.insert(SubstateId::System);
                value_refs.insert(
                    RENodeId::System,
                    REValueInfo {
                        location: REValuePointer::Track(SubstateId::System),
                        visible: true,
                    },
                );
                let fn_str: &str = &fn_ident;
                let access_rules = match fn_str {
                    "set_epoch" => {
                        vec![MethodAuthorization::Protected(HardAuthRule::ProofRule(
                            HardProofRule::Require(HardResourceOrNonFungible::Resource(
                                SYSTEM_TOKEN,
                            )),
                        ))]
                    }
                    _ => vec![],
                };
                Ok((SNodeExecution::ValueRef(RENodeId::System), access_rules))
            }
            SNodeRef::AuthZoneRef => {
                if let Some(auth_zone) = &self.auth_zone {
                    for resource_address in &input.resource_addresses {
                        self.track
                            .acquire_lock(resource_address.clone(), false, false)
                            .map_err(|e| match e {
                                TrackError::NotFound => {
                                    RuntimeError::ResourceManagerNotFound(resource_address.clone())
                                }
                                TrackError::Reentrancy => {
                                    panic!("Package reentrancy error should never occur.")
                                }
                                TrackError::StateTrackError(..) => panic!("Unexpected"),
                            })?;
                        locked_values.insert(resource_address.clone().into());
                        value_refs.insert(
                            RENodeId::Resource(resource_address.clone()),
                            REValueInfo {
                                location: REValuePointer::Track(SubstateId::ResourceManager(
                                    resource_address.clone(),
                                )),
                                visible: true,
                            },
                        );
                    }
                    let borrowed = auth_zone.borrow_mut();
                    Ok((SNodeExecution::AuthZone(borrowed), vec![]))
                } else {
                    Err(RuntimeError::AuthZoneDoesNotExist)
                }
            }
            SNodeRef::ResourceRef(resource_address) => {
                let value_id = RENodeId::Resource(*resource_address);
                let substate_id: SubstateId = SubstateId::ResourceManager(*resource_address);
                self.track
                    .acquire_lock(substate_id.clone(), true, false)
                    .map_err(|e| match e {
                        TrackError::NotFound => {
                            RuntimeError::ResourceManagerNotFound(resource_address.clone())
                        }
                        TrackError::Reentrancy => {
                            panic!("Resource call has caused reentrancy")
                        }
                        TrackError::StateTrackError(..) => panic!("Unexpected"),
                    })?;
                locked_values.insert(substate_id.clone());
                let resource_manager = self.track.read_value(substate_id).resource_manager();
                let method_auth = resource_manager.get_auth(&fn_ident, &input).clone();
                value_refs.insert(
                    value_id.clone(),
                    REValueInfo {
                        location: REValuePointer::Track(SubstateId::ResourceManager(
                            *resource_address,
                        )),
                        visible: true,
                    },
                );

                Ok((SNodeExecution::ValueRef(value_id), vec![method_auth]))
            }
            SNodeRef::BucketRef(bucket_id) => {
                let value_id = RENodeId::Bucket(*bucket_id);
                if !self.owned_values.contains_key(&value_id) {
                    return Err(RuntimeError::BucketNotFound(bucket_id.clone()));
                }
                value_refs.insert(
                    value_id.clone(),
                    REValueInfo {
                        location: REValuePointer::Stack {
                            frame_id: Some(self.depth),
                            root: value_id.clone(),
                            id: None,
                        },
                        visible: true,
                    },
                );

                Ok((SNodeExecution::ValueRef(value_id), vec![]))
            }
            SNodeRef::ProofRef(proof_id) => {
                let value_id = RENodeId::Proof(*proof_id);
                if !self.owned_values.contains_key(&value_id) {
                    return Err(RuntimeError::ProofNotFound(proof_id.clone()));
                }
                value_refs.insert(
                    value_id.clone(),
                    REValueInfo {
                        location: REValuePointer::Stack {
                            frame_id: Some(self.depth),
                            root: value_id.clone(),
                            id: None,
                        },
                        visible: true,
                    },
                );
                Ok((SNodeExecution::ValueRef(value_id), vec![]))
            }
            SNodeRef::WorktopRef => {
                let value_id = RENodeId::Worktop;
                if !self.owned_values.contains_key(&value_id) {
                    return Err(RuntimeError::ValueNotFound(value_id));
                }
                value_refs.insert(
                    value_id.clone(),
                    REValueInfo {
                        location: REValuePointer::Stack {
                            frame_id: Some(self.depth),
                            root: value_id.clone(),
                            id: None,
                        },
                        visible: true,
                    },
                );

                for resource_address in &input.resource_addresses {
                    self.track
                        .acquire_lock(resource_address.clone(), false, false)
                        .map_err(|e| match e {
                            TrackError::NotFound => {
                                RuntimeError::ResourceManagerNotFound(resource_address.clone())
                            }
                            TrackError::Reentrancy => {
                                panic!("Package reentrancy error should never occur.")
                            }
                            TrackError::StateTrackError(..) => panic!("Unexpected"),
                        })?;

                    locked_values.insert(resource_address.clone().into());
                    value_refs.insert(
                        RENodeId::Resource(resource_address.clone()),
                        REValueInfo {
                            location: REValuePointer::Track(SubstateId::ResourceManager(
                                resource_address.clone(),
                            )),
                            visible: true,
                        },
                    );
                }

                Ok((SNodeExecution::ValueRef(value_id), vec![]))
            }
            SNodeRef::Scrypto(actor) => match actor {
                ScryptoActor::Blueprint(package_address, blueprint_name) => {
                    self.track
                        .acquire_lock(package_address.clone(), false, false)
                        .map_err(|e| match e {
                            TrackError::NotFound => RuntimeError::PackageNotFound(*package_address),
                            TrackError::Reentrancy => {
                                panic!("Package reentrancy error should never occur.")
                            }
                            TrackError::StateTrackError(..) => panic!("Unexpected"),
                        })?;
                    locked_values.insert(package_address.clone().into());
                    let package = self.track.read_value(package_address.clone()).package();
                    let abi = package.blueprint_abi(blueprint_name).ok_or(
                        RuntimeError::BlueprintNotFound(
                            package_address.clone(),
                            blueprint_name.clone(),
                        ),
                    )?;
                    let fn_abi = abi
                        .get_fn_abi(&fn_ident)
                        .ok_or(RuntimeError::MethodDoesNotExist(fn_ident.clone()))?;
                    if !fn_abi.input.matches(&input.dom) {
                        return Err(RuntimeError::InvalidFnInput {
                            fn_ident,
                            input: input.dom,
                        });
                    }
                    Ok((
                        SNodeExecution::Scrypto(
                            ScryptoActorInfo::blueprint(
                                package_address.clone(),
                                blueprint_name.clone(),
                            ),
                            package_address.clone(),
                        ),
                        vec![],
                    ))
                }
                ScryptoActor::Component(component_address) => {
                    let component_address = *component_address;

                    // Find value
                    let value_id = RENodeId::Component(component_address);
                    let cur_location = if self.owned_values.contains_key(&value_id) {
                        REValuePointer::Stack {
                            frame_id: None,
                            root: value_id.clone(),
                            id: None,
                        }
                    } else if let Some(REValueInfo { location, .. }) =
                        self.value_refs.get(&value_id)
                    {
                        location.clone()
                    } else {
                        REValuePointer::Track(SubstateId::ComponentInfo(component_address, true))
                    };

                    // Lock values and setup next frame
                    let (next_location, is_global) = match cur_location.clone() {
                        REValuePointer::Track(substate_id) => {
                            self.track
                                .acquire_lock(substate_id.clone(), true, false)
                                .map_err(|e| match e {
                                    TrackError::NotFound => {
                                        RuntimeError::ComponentNotFound(component_address)
                                    }
                                    TrackError::Reentrancy => {
                                        RuntimeError::ComponentReentrancy(component_address)
                                    }
                                    TrackError::StateTrackError(..) => {
                                        panic!("Unexpected")
                                    }
                                })?;
                            locked_values.insert(substate_id.clone());

                            let component_address: ComponentAddress = substate_id.clone().into();
                            let component_state_address =
                                SubstateId::ComponentState(component_address);
                            self.track
                                .acquire_lock(component_state_address.clone(), true, false)
                                .map_err(|e| match e {
                                    TrackError::NotFound => {
                                        RuntimeError::ComponentNotFound(component_address)
                                    }
                                    TrackError::Reentrancy => {
                                        RuntimeError::ComponentReentrancy(component_address)
                                    }
                                    TrackError::StateTrackError(..) => {
                                        panic!("Unexpected")
                                    }
                                })?;
                            locked_values.insert(component_state_address);

                            let is_global =
                                if let SubstateId::ComponentInfo(_component_address, is_global) =
                                    substate_id
                                {
                                    is_global
                                } else {
                                    panic!("Unexpected substate_id");
                                };

                            (REValuePointer::Track(substate_id), is_global)
                        }
                        REValuePointer::Stack { frame_id, root, id } => (
                            REValuePointer::Stack {
                                frame_id: frame_id.or(Some(self.depth)),
                                root,
                                id,
                            },
                            false,
                        ),
                    };

                    let actor_info = {
                        let value_ref = cur_location.to_ref(
                            &self.owned_values,
                            &self.parent_values,
                            &mut self.track,
                        );
                        let component = value_ref.component();
                        ScryptoActorInfo::component(
                            component.package_address(),
                            component.blueprint_name().to_string(),
                            component_address,
                            is_global,
                        )
                    };

                    // Retrieve Method Authorization
                    let (method_auths, package_address) = {
                        let package_address = actor_info.package_address().clone();
                        let blueprint_name = actor_info.blueprint_name().to_string();
                        self.track
                            .acquire_lock(package_address, false, false)
                            .expect("Should never fail");
                        locked_values.insert(package_address.clone().into());
                        let package = self.track.read_value(package_address).package();
                        let abi = package
                            .blueprint_abi(&blueprint_name)
                            .expect("Blueprint not found for existing component");
                        let fn_abi = abi
                            .get_fn_abi(&fn_ident)
                            .ok_or(RuntimeError::MethodDoesNotExist(fn_ident.clone()))?;
                        if !fn_abi.input.matches(&input.dom) {
                            return Err(RuntimeError::InvalidFnInput {
                                fn_ident,
                                input: input.dom,
                            });
                        }

                        let method_auths = {
                            let value_ref = cur_location.to_ref(
                                &self.owned_values,
                                &self.parent_values,
                                &self.track,
                            );

                            let component = value_ref.component();
                            let component_state = value_ref.component_state();
                            component.method_authorization(
                                component_state,
                                &abi.structure,
                                &fn_ident,
                            )
                        };

                        (method_auths, package_address)
                    };

                    value_refs.insert(
                        value_id,
                        REValueInfo {
                            location: next_location,
                            visible: true,
                        },
                    );

                    Ok((
                        SNodeExecution::Scrypto(actor_info, package_address),
                        method_auths,
                    ))
                }
            },
            SNodeRef::Component(component_address) => {
                let component_address = *component_address;

                // Find value
                let value_id = RENodeId::Component(component_address);
                let cur_location = if self.owned_values.contains_key(&value_id) {
                    REValuePointer::Stack {
                        frame_id: None,
                        root: value_id.clone(),
                        id: None,
                    }
                } else {
                    return Err(RuntimeError::NotSupported);
                };

                // Setup next frame
                match cur_location {
                    REValuePointer::Stack {
                        frame_id: _,
                        root,
                        id,
                    } => {
                        let owned_ref = self.owned_values.get_mut(&root).unwrap();

                        // Lock package
                        let package_address = owned_ref.root().component().package_address();
                        self.track
                            .acquire_lock(package_address, false, false)
                            .map_err(|e| match e {
                                TrackError::NotFound => panic!("Should exist"),
                                TrackError::Reentrancy => RuntimeError::PackageReentrancy,
                                TrackError::StateTrackError(..) => panic!("Unexpected"),
                            })?;
                        locked_values.insert(package_address.into());
                        value_refs.insert(
                            RENodeId::Package(package_address),
                            REValueInfo {
                                location: REValuePointer::Track(SubstateId::Package(
                                    package_address,
                                )),
                                visible: true,
                            },
                        );

                        value_refs.insert(
                            value_id,
                            REValueInfo {
                                location: REValuePointer::Stack {
                                    frame_id: Some(self.depth),
                                    root: root.clone(),
                                    id: id.clone(),
                                },
                                visible: true,
                            },
                        );
                    }
                    _ => panic!("Unexpected"),
                }

                Ok((SNodeExecution::ValueRef(value_id), vec![]))
            }

            SNodeRef::VaultRef(vault_id) => {
                // Find value
                let value_id = RENodeId::Vault(*vault_id);
                let cur_location = if self.owned_values.contains_key(&value_id) {
                    REValuePointer::Stack {
                        frame_id: None,
                        root: value_id.clone(),
                        id: Option::None,
                    }
                } else {
                    let maybe_value_ref = self.value_refs.get(&value_id);
                    maybe_value_ref
                        .map(|info| &info.location)
                        .cloned()
                        .ok_or(RuntimeError::ValueNotFound(RENodeId::Vault(*vault_id)))?
                };
                if is_pay_fee && !matches!(cur_location, REValuePointer::Track { .. }) {
                    return Err(RuntimeError::PayFeeError(PayFeeError::ValueNotInTrack));
                }

                // Lock values and setup next frame
                let next_location = {
                    // Lock Vault
                    let next_location = match cur_location.clone() {
                        REValuePointer::Track(substate_id) => {
                            self.track
                                .acquire_lock(substate_id.clone(), true, is_pay_fee)
                                .map_err(|e| match e {
                                    TrackError::NotFound | TrackError::Reentrancy => {
                                        panic!("Illegal state")
                                    }
                                    TrackError::StateTrackError(e) => {
                                        RuntimeError::PayFeeError(match e {
                                            StateTrackError::ValueAlreadyTouched => {
                                                PayFeeError::ValueAlreadyTouched
                                            }
                                        })
                                    }
                                })?;
                            locked_values.insert(substate_id.clone().into());
                            REValuePointer::Track(substate_id)
                        }
                        REValuePointer::Stack { frame_id, root, id } => REValuePointer::Stack {
                            frame_id: frame_id.or(Some(self.depth)),
                            root,
                            id,
                        },
                    };

                    // Lock Resource
                    let resource_address = {
                        let value_ref = cur_location.to_ref(
                            &mut self.owned_values,
                            &mut self.parent_values,
                            &mut self.track,
                        );
                        value_ref.vault().resource_address()
                    };
                    self.track
                        .acquire_lock(resource_address, true, false)
                        .expect("Should never fail.");
                    locked_values.insert(resource_address.into());

                    next_location
                };

                // Retrieve Method Authorization
                let method_auth = {
                    let resource_address = {
                        let value_ref = cur_location.to_ref(
                            &mut self.owned_values,
                            &mut self.parent_values,
                            &mut self.track,
                        );
                        value_ref.vault().resource_address()
                    };
                    let resource_manager =
                        self.track.read_value(resource_address).resource_manager();
                    resource_manager.get_vault_auth(&fn_ident).clone()
                };

                value_refs.insert(
                    value_id.clone(),
                    REValueInfo {
                        location: next_location,
                        visible: true,
                    },
                );

                Ok((SNodeExecution::ValueRef(value_id), vec![method_auth]))
            }
        }?;

        // Authorization check
        if !method_auths.is_empty() {
            let mut auth_zones = Vec::new();
            if let Some(self_auth_zone) = &self.auth_zone {
                auth_zones.push(self_auth_zone.borrow());
            }

            match &loaded_snode {
                // Resource auth check includes caller
                SNodeExecution::Scrypto(..)
                | SNodeExecution::ValueRef(RENodeId::Resource(..), ..)
                | SNodeExecution::ValueRef(RENodeId::Vault(..), ..)
                | SNodeExecution::Consumed(RENodeId::Bucket(..)) => {
                    if let Some(auth_zone) = self.caller_auth_zone {
                        auth_zones.push(auth_zone.borrow());
                    }
                }
                // Extern call auth check
                _ => {}
            };

            let mut borrowed = Vec::new();
            for auth_zone in &auth_zones {
                borrowed.push(auth_zone.deref());
            }
            for method_auth in method_auths {
                method_auth
                    .check(&borrowed)
                    .map_err(|error| RuntimeError::AuthorizationError {
                        function: fn_ident.clone(),
                        authorization: method_auth,
                        error,
                    })?;
            }
        }

        // Setup next parent frame
        let mut next_borrowed_values: Vec<&mut HashMap<RENodeId, REValue>> = Vec::new();
        for parent_values in &mut self.parent_values {
            next_borrowed_values.push(parent_values);
        }
        next_borrowed_values.push(&mut self.owned_values);

        // start a new frame
        let mut frame = CallFrame::new(
            self.transaction_hash,
            self.depth + 1,
            self.trace,
            self.id_allocator,
            self.track,
            self.wasm_engine,
            self.wasm_instrumenter,
            self.cost_unit_counter,
            self.fee_table,
            match loaded_snode {
                SNodeExecution::Scrypto(..)
                | SNodeExecution::Static(StaticSNodeState::TransactionProcessor) => {
                    Some(RefCell::new(AuthZone::new()))
                }
                _ => None,
            },
            next_owned_values,
            value_refs,
            next_borrowed_values,
            self.auth_zone.as_ref(),
        );

        // invoke the main function
        let (result, received_values) = frame.run(snode_ref, loaded_snode, &fn_ident, input)?;
        drop(frame);

        // Release locked addresses
        for l in locked_values {
            // TODO: refactor after introducing `Lock` representation.
            self.track
                .release_lock(l.clone(), is_pay_fee && matches!(l, SubstateId::Vault(..)));
        }

        // move buckets and proofs to this process.
        for (id, value) in received_values {
            trace!(self, Level::Debug, "Received value: {:?}", value);
            self.owned_values.insert(id, value);
        }

        trace!(self, Level::Debug, "Invoking finished!");
        Ok(result)
    }

    fn borrow_value(
        &mut self,
        value_id: &RENodeId,
    ) -> Result<REValueRef<'_, 's>, CostUnitCounterError> {
        trace!(self, Level::Debug, "Borrowing value: {:?}", value_id);
        self.cost_unit_counter.consume(
            self.fee_table.system_api_cost({
                match value_id {
                    RENodeId::Bucket(_) => SystemApiCostingEntry::BorrowLocal,
                    RENodeId::Proof(_) => SystemApiCostingEntry::BorrowLocal,
                    RENodeId::Worktop => SystemApiCostingEntry::BorrowLocal,
                    RENodeId::Vault(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::Component(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::KeyValueStore(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::Resource(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::Package(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::System => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                }
            }),
            "borrow",
        )?;

        let info = self
            .value_refs
            .get(value_id)
            .expect(&format!("{:?} is unknown.", value_id));
        if !info.visible {
            panic!("Trying to read value which is not visible.")
        }

        Ok(info
            .location
            .to_ref(&self.owned_values, &self.parent_values, &self.track))
    }

    fn borrow_value_mut(
        &mut self,
        value_id: &RENodeId,
    ) -> Result<RENativeValueRef, CostUnitCounterError> {
        trace!(self, Level::Debug, "Borrowing value (mut): {:?}", value_id);

        self.cost_unit_counter.consume(
            self.fee_table.system_api_cost({
                match value_id {
                    RENodeId::Bucket(_) => SystemApiCostingEntry::BorrowLocal,
                    RENodeId::Proof(_) => SystemApiCostingEntry::BorrowLocal,
                    RENodeId::Worktop => SystemApiCostingEntry::BorrowLocal,
                    RENodeId::Vault(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::Component(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::KeyValueStore(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::Resource(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::Package(_) => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                    RENodeId::System => SystemApiCostingEntry::BorrowGlobal {
                        // TODO: figure out loaded state and size
                        loaded: false,
                        size: 0,
                    },
                }
            }),
            "borrow",
        )?;

        let info = self
            .value_refs
            .get(value_id)
            .expect(&format!("Value should exist {:?}", value_id));
        if !info.visible {
            panic!("Trying to read value which is not visible.")
        }

        Ok(info.location.borrow_native_ref(
            &mut self.owned_values,
            &mut self.parent_values,
            &mut self.track,
        ))
    }

    fn return_value_mut(&mut self, val_ref: RENativeValueRef) -> Result<(), CostUnitCounterError> {
        trace!(self, Level::Debug, "Returning value");

        self.cost_unit_counter.consume(
            self.fee_table.system_api_cost({
                match &val_ref {
                    RENativeValueRef::Stack(..) => SystemApiCostingEntry::ReturnLocal,
                    RENativeValueRef::Track(substate_id, _) => match substate_id {
                        SubstateId::Vault(_) => SystemApiCostingEntry::ReturnGlobal { size: 0 },
                        SubstateId::KeyValueStoreSpace(_) => {
                            SystemApiCostingEntry::ReturnGlobal { size: 0 }
                        }
                        SubstateId::KeyValueStoreEntry(_, _) => {
                            SystemApiCostingEntry::ReturnGlobal { size: 0 }
                        }
                        SubstateId::ResourceManager(_) => {
                            SystemApiCostingEntry::ReturnGlobal { size: 0 }
                        }
                        SubstateId::Package(_) => SystemApiCostingEntry::ReturnGlobal { size: 0 },
                        SubstateId::NonFungibleSpace(_) => {
                            SystemApiCostingEntry::ReturnGlobal { size: 0 }
                        }
                        SubstateId::NonFungible(_, _) => {
                            SystemApiCostingEntry::ReturnGlobal { size: 0 }
                        }
                        SubstateId::ComponentInfo(..) => {
                            SystemApiCostingEntry::ReturnGlobal { size: 0 }
                        }
                        SubstateId::ComponentState(_) => {
                            SystemApiCostingEntry::ReturnGlobal { size: 0 }
                        }
                        SubstateId::System => SystemApiCostingEntry::ReturnGlobal { size: 0 },
                    },
                }
            }),
            "return",
        )?;

        val_ref.return_to_location(
            &mut self.owned_values,
            &mut self.parent_values,
            &mut self.track,
        );
        Ok(())
    }

    fn drop_node(&mut self, value_id: &RENodeId) -> Result<REValue, CostUnitCounterError> {
        trace!(self, Level::Debug, "Dropping value: {:?}", value_id);

        // TODO: costing

        Ok(self.owned_values.remove(&value_id).unwrap())
    }

    fn create_node<V: Into<REValueByComplexity>>(
        &mut self,
        v: V,
    ) -> Result<RENodeId, RuntimeError> {
        trace!(self, Level::Debug, "Creating value");

        self.cost_unit_counter
            .consume(
                self.fee_table
                    .system_api_cost(SystemApiCostingEntry::Create {
                        size: 0, // TODO: get size of the value
                    }),
                "create",
            )
            .map_err(RuntimeError::CostingError)?;

        let value_by_complexity = v.into();
        let id = match value_by_complexity {
            REValueByComplexity::Primitive(REPrimitiveValue::Bucket(..)) => {
                let bucket_id = self.new_bucket_id();
                RENodeId::Bucket(bucket_id)
            }
            REValueByComplexity::Primitive(REPrimitiveValue::Proof(..)) => {
                let proof_id = self.new_proof_id();
                RENodeId::Proof(proof_id)
            }
            REValueByComplexity::Primitive(REPrimitiveValue::Worktop(..)) => RENodeId::Worktop,
            REValueByComplexity::Primitive(REPrimitiveValue::Vault(..)) => {
                let vault_id = self.new_vault_id();
                RENodeId::Vault(vault_id)
            }
            REValueByComplexity::Primitive(REPrimitiveValue::KeyValue(..)) => {
                let kv_store_id = self.new_kv_store_id();
                RENodeId::KeyValueStore(kv_store_id)
            }
            REValueByComplexity::Primitive(REPrimitiveValue::Package(..)) => {
                let package_address = self.new_package_address();
                RENodeId::Package(package_address)
            }
            REValueByComplexity::Primitive(REPrimitiveValue::Resource(..)) => {
                let resource_address = self.new_resource_address();
                RENodeId::Resource(resource_address)
            }
            REValueByComplexity::Complex(REComplexValue::Component(ref component, ..)) => {
                let component_address = self.new_component_address(component);
                RENodeId::Component(component_address)
            }
        };

        let re_value = match value_by_complexity {
            REValueByComplexity::Primitive(primitive) => primitive.into(),
            REValueByComplexity::Complex(complex) => {
                let children = complex.get_children()?;
                let (child_values, mut missing) = self.take_available_values(children, true)?;
                let first_missing_value = missing.drain().nth(0);
                if let Some(missing_value) = first_missing_value {
                    return Err(RuntimeError::ValueNotFound(missing_value));
                }
                complex.into_re_value(child_values)
            }
        };
        self.owned_values.insert(id, re_value);

        match id {
            RENodeId::KeyValueStore(..) | RENodeId::Resource(..) => {
                self.value_refs.insert(
                    id.clone(),
                    REValueInfo {
                        location: REValuePointer::Stack {
                            frame_id: None,
                            root: id.clone(),
                            id: None,
                        },
                        visible: true,
                    },
                );
            }
            _ => {}
        }

        Ok(id)
    }

    fn globalize_node(&mut self, value_id: &RENodeId) -> Result<(), CostUnitCounterError> {
        trace!(self, Level::Debug, "Globalizing value: {:?}", value_id);

        self.cost_unit_counter.consume(
            self.fee_table
                .system_api_cost(SystemApiCostingEntry::Globalize {
                    size: 0, // TODO: get size of the value
                }),
            "globalize",
        )?;

        let mut values = HashSet::new();
        values.insert(value_id.clone());
        let (taken_values, missing) = self.take_available_values(values, false).unwrap();
        assert!(missing.is_empty());
        assert!(taken_values.len() == 1);
        let value = taken_values.into_values().nth(0).unwrap();

        let (substates, maybe_non_fungibles) = match value.root {
            RENode::Component(component, component_state) => {
                let mut substates = HashMap::new();
                let component_address = value_id.clone().into();
                substates.insert(
                    SubstateId::ComponentInfo(component_address, true),
                    Substate::Component(component),
                );
                substates.insert(
                    SubstateId::ComponentState(component_address),
                    Substate::ComponentState(component_state),
                );
                (substates, None)
            }
            RENode::Package(package) => {
                let mut substates = HashMap::new();
                let package_address = value_id.clone().into();
                substates.insert(
                    SubstateId::Package(package_address),
                    Substate::Package(package),
                );
                (substates, None)
            }
            RENode::Resource(resource_manager, non_fungibles) => {
                let mut substates = HashMap::new();
                let resource_address: ResourceAddress = value_id.clone().into();
                substates.insert(
                    SubstateId::ResourceManager(resource_address),
                    Substate::Resource(resource_manager),
                );
                (substates, non_fungibles)
            }
            _ => panic!("Not expected"),
        };

        for (substate_id, substate) in substates {
            self.track.create_uuid_value(substate_id.clone(), substate);
        }

        let mut to_store_values = HashMap::new();
        for (id, value) in value.non_root_nodes.into_iter() {
            to_store_values.insert(id, value);
        }
        insert_non_root_nodes(self.track, to_store_values);

        if let Some(non_fungibles) = maybe_non_fungibles {
            let resource_address: ResourceAddress = value_id.clone().into();
            let parent_address = SubstateId::NonFungibleSpace(resource_address.clone());
            for (id, non_fungible) in non_fungibles {
                self.track.set_key_value(
                    parent_address.clone(),
                    id.to_vec(),
                    Substate::NonFungible(NonFungibleWrapper(Some(non_fungible))),
                );
            }
        }

        Ok(())
    }

    fn take_substate(&mut self, substate_id: SubstateId) -> Result<ScryptoValue, RuntimeError> {
        trace!(self, Level::Debug, "Removing value data: {:?}", substate_id);

        let (location, current_value) = self.read_value_internal(&substate_id)?;
        let cur_children = current_value.value_ids();
        if !cur_children.is_empty() {
            return Err(RuntimeError::ValueNotAllowed);
        }

        // Write values
        let value_ref = location.to_ref_mut(
            &mut self.owned_values,
            &mut self.parent_values,
            &mut self.track,
        );
        substate_id.replace_value_with_default(value_ref);

        Ok(current_value)
    }

    fn read_substate(&mut self, substate_id: SubstateId) -> Result<ScryptoValue, RuntimeError> {
        trace!(self, Level::Debug, "Reading value data: {:?}", substate_id);

        self.cost_unit_counter
            .consume(
                self.fee_table.system_api_cost(SystemApiCostingEntry::Read {
                    size: 0, // TODO: get size of the value
                }),
                "read",
            )
            .map_err(RuntimeError::CostingError)?;

        let (parent_location, current_value) = self.read_value_internal(&substate_id)?;
        let cur_children = current_value.value_ids();
        for child_id in cur_children {
            let child_location = parent_location.child(child_id);

            // Extend current readable space when kv stores are found
            let visible = matches!(child_id, RENodeId::KeyValueStore(..));
            let child_info = REValueInfo {
                location: child_location,
                visible,
            };
            self.value_refs.insert(child_id, child_info);
        }
        Ok(current_value)
    }

    fn write_substate(
        &mut self,
        substate_id: SubstateId,
        value: ScryptoValue,
    ) -> Result<(), RuntimeError> {
        trace!(self, Level::Debug, "Writing value data: {:?}", substate_id);

        self.cost_unit_counter
            .consume(
                self.fee_table
                    .system_api_cost(SystemApiCostingEntry::Write {
                        size: 0, // TODO: get size of the value
                    }),
                "write",
            )
            .map_err(RuntimeError::CostingError)?;

        substate_id.verify_can_write()?;

        // If write, take values from current frame
        let (taken_values, missing) = {
            let value_ids = value.value_ids();
            if !value_ids.is_empty() {
                if !substate_id.can_own_nodes() {
                    return Err(RuntimeError::ValueNotAllowed);
                }

                self.take_available_values(value_ids, true)?
            } else {
                (HashMap::new(), HashSet::new())
            }
        };

        let (location, current_value) = self.read_value_internal(&substate_id)?;
        let cur_children = current_value.value_ids();

        // Fulfill method
        verify_stored_value_update(&cur_children, &missing)?;

        // TODO: verify against some schema

        // Write values
        let value_ref = location.to_ref_mut(
            &mut self.owned_values,
            &mut self.parent_values,
            &mut self.track,
        );
        substate_id.write_value(value_ref, value, taken_values);

        Ok(())
    }

    fn transaction_hash(&mut self) -> Result<Hash, CostUnitCounterError> {
        self.cost_unit_counter.consume(
            self.fee_table
                .system_api_cost(SystemApiCostingEntry::ReadTransactionHash),
            "read_transaction_hash",
        )?;
        Ok(self.transaction_hash)
    }

    fn generate_uuid(&mut self) -> Result<u128, CostUnitCounterError> {
        self.cost_unit_counter.consume(
            self.fee_table
                .system_api_cost(SystemApiCostingEntry::GenerateUuid),
            "generate_uuid",
        )?;
        Ok(self.new_uuid())
    }

    fn emit_log(&mut self, level: Level, message: String) -> Result<(), CostUnitCounterError> {
        self.cost_unit_counter.consume(
            self.fee_table
                .system_api_cost(SystemApiCostingEntry::EmitLog {
                    size: message.len() as u32,
                }),
            "emit_log",
        )?;
        self.track.add_log(level, message);
        Ok(())
    }

    fn check_access_rule(
        &mut self,
        access_rule: scrypto::resource::AccessRule,
        proof_ids: Vec<ProofId>,
    ) -> Result<bool, RuntimeError> {
        let proofs = proof_ids
            .iter()
            .map(|proof_id| {
                self.owned_values
                    .get(&RENodeId::Proof(*proof_id))
                    .map(|p| match p.root() {
                        RENode::Proof(proof) => proof.clone(),
                        _ => panic!("Expected proof"),
                    })
                    .ok_or(RuntimeError::ProofNotFound(proof_id.clone()))
            })
            .collect::<Result<Vec<Proof>, RuntimeError>>()?;
        let mut simulated_auth_zone = AuthZone::new_with_proofs(proofs);

        let method_authorization = convert(&Type::Unit, &Value::Unit, &access_rule);
        let is_authorized = method_authorization.check(&[&simulated_auth_zone]).is_ok();
        simulated_auth_zone
            .main(
                "clear",
                ScryptoValue::from_typed(&AuthZoneClearInput {}),
                self,
            )
            .map_err(RuntimeError::AuthZoneError)?;

        Ok(is_authorized)
    }

    fn cost_unit_counter(&mut self) -> &mut C {
        self.cost_unit_counter
    }

    fn fee_table(&self) -> &FeeTable {
        self.fee_table
    }
}
