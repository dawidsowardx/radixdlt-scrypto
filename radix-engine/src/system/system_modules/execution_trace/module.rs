use crate::blueprints::resource::VaultUtil;
use crate::errors::*;
use crate::kernel::actor::{Actor, MethodActor};
use crate::kernel::call_frame::Message;
use crate::kernel::kernel_api::KernelApi;
use crate::kernel::kernel_callback_api::KernelCallbackObject;
use crate::system::module::SystemModule;
use crate::system::system_callback::SystemConfig;
use crate::system::system_callback_api::SystemCallbackObject;
use crate::track::interface::{NodeSubstates, StoreAccessInfo};
use crate::transaction::{FeeLocks, TransactionExecutionTrace};
use crate::types::*;
use radix_engine_interface::blueprints::resource::*;
use radix_engine_interface::math::Decimal;
use sbor::rust::collections::*;
use sbor::rust::fmt::Debug;

//===================================================================================
// Note: ExecutionTrace must not produce any error or transactional side effect!
//===================================================================================

#[derive(Debug, Clone)]
pub struct ExecutionTraceModule {
    /// Maximum depth up to which kernel calls are being traced.
    max_kernel_call_depth_traced: usize,

    /// Current transaction index
    current_instruction_index: usize,

    /// Current kernel calls depth. Note that this doesn't necessarily correspond to the
    /// call frame depth, as there can be nested kernel calls within a single call frame
    /// (e.g. open_substate call inside drop_node).
    current_kernel_call_depth: usize,

    /// A stack of traced kernel call inputs, their origin, and the instruction index.
    traced_kernel_call_inputs_stack: Vec<(ResourceSummary, TraceOrigin, usize)>,

    /// A mapping of complete KernelCallTrace stacks (\w both inputs and outputs), indexed by depth.
    kernel_call_traces_stacks: IndexMap<usize, Vec<ExecutionTrace>>,

    /// Vault operations: (Caller, Vault ID, operation, instruction index)
    vault_ops: Vec<(TraceActor, NodeId, VaultOp, usize)>,
}

impl ExecutionTraceModule {
    pub fn update_instruction_index(&mut self, new_index: usize) {
        self.current_instruction_index = new_index;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, ScryptoSbor)]
pub struct ResourceChange {
    pub node_id: NodeId,
    pub vault_id: NodeId,
    pub resource_address: ResourceAddress,
    pub amount: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, ScryptoSbor)]
pub enum WorktopChange {
    Take(ResourceSpecifier),
    Put(ResourceSpecifier),
}

#[derive(Debug, Clone, PartialEq, Eq, ScryptoSbor)]
pub enum ResourceSpecifier {
    Amount(ResourceAddress, Decimal),
    Ids(ResourceAddress, BTreeSet<NonFungibleLocalId>),
}

impl From<&BucketSnapshot> for ResourceSpecifier {
    fn from(value: &BucketSnapshot) -> Self {
        match value {
            BucketSnapshot::Fungible {
                resource_address,
                liquid,
                ..
            } => Self::Amount(*resource_address, *liquid),
            BucketSnapshot::NonFungible {
                resource_address,
                liquid,
                ..
            } => Self::Ids(*resource_address, liquid.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum VaultOp {
    Create(Decimal),               // TODO: add trace of vault creation
    Put(ResourceAddress, Decimal), // TODO: add non-fungible support
    Take(ResourceAddress, Decimal),
    LockFee(Decimal, bool),
}

#[derive(Clone, Debug, PartialEq, Eq, ScryptoSbor)]
pub enum BucketSnapshot {
    Fungible {
        resource_address: ResourceAddress,
        liquid: Decimal,
    },
    NonFungible {
        resource_address: ResourceAddress,
        liquid: BTreeSet<NonFungibleLocalId>,
    },
}

impl BucketSnapshot {
    pub fn resource_address(&self) -> ResourceAddress {
        match self {
            BucketSnapshot::Fungible {
                resource_address, ..
            } => resource_address.clone(),
            BucketSnapshot::NonFungible {
                resource_address, ..
            } => resource_address.clone(),
        }
    }
    pub fn amount(&self) -> Decimal {
        match self {
            BucketSnapshot::Fungible { liquid, .. } => liquid.clone(),
            BucketSnapshot::NonFungible { liquid, .. } => liquid.len().into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, ScryptoSbor)]
pub enum ProofSnapshot {
    Fungible {
        resource_address: ResourceAddress,
        total_locked: Decimal,
    },
    NonFungible {
        resource_address: ResourceAddress,
        total_locked: BTreeSet<NonFungibleLocalId>,
    },
}

impl ProofSnapshot {
    pub fn resource_address(&self) -> ResourceAddress {
        match self {
            ProofSnapshot::Fungible {
                resource_address, ..
            } => resource_address.clone(),
            ProofSnapshot::NonFungible {
                resource_address, ..
            } => resource_address.clone(),
        }
    }
    pub fn amount(&self) -> Decimal {
        match self {
            ProofSnapshot::Fungible { total_locked, .. } => total_locked.clone(),
            ProofSnapshot::NonFungible { total_locked, .. } => total_locked.len().into(),
        }
    }
}

#[derive(Debug, Clone, ScryptoSbor)]
pub struct ResourceSummary {
    pub buckets: IndexMap<NodeId, BucketSnapshot>,
    pub proofs: IndexMap<NodeId, ProofSnapshot>,
}

// TODO: Clean up
#[derive(Debug, Clone, ScryptoSbor)]
pub enum TraceActor {
    Method(NodeId),
    NonMethod,
}

impl TraceActor {
    pub fn from_actor(actor: &Actor) -> TraceActor {
        match actor {
            Actor::Method(MethodActor { node_id, .. }) => TraceActor::Method(node_id.clone()),
            _ => TraceActor::NonMethod,
        }
    }
}

#[derive(Debug, Clone, ScryptoSbor)]
pub struct ExecutionTrace {
    pub origin: TraceOrigin,
    pub kernel_call_depth: usize,
    pub current_frame_actor: TraceActor,
    pub current_frame_depth: usize,
    pub instruction_index: usize,
    pub input: ResourceSummary,
    pub output: ResourceSummary,
    pub children: Vec<ExecutionTrace>,
}

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub struct ApplicationFnIdentifier {
    pub package_address: PackageAddress,
    pub blueprint_name: String,
    pub ident: String,
}

#[derive(Debug, Clone, Eq, PartialEq, ScryptoSbor)]
pub enum TraceOrigin {
    ScryptoFunction(ApplicationFnIdentifier),
    ScryptoMethod(ApplicationFnIdentifier),
    CreateNode,
    DropNode,
}

impl ExecutionTrace {
    pub fn worktop_changes(
        &self,
        worktop_changes_aggregator: &mut IndexMap<usize, Vec<WorktopChange>>,
    ) {
        if let TraceOrigin::ScryptoMethod(fn_identifier) = &self.origin {
            if fn_identifier.blueprint_name == WORKTOP_BLUEPRINT
                && fn_identifier.package_address == RESOURCE_PACKAGE
            {
                if fn_identifier.ident == WORKTOP_PUT_IDENT {
                    for (_, bucket_snapshot) in self.input.buckets.iter() {
                        worktop_changes_aggregator
                            .entry(self.instruction_index)
                            .or_default()
                            .push(WorktopChange::Put(bucket_snapshot.into()))
                    }
                } else if fn_identifier.ident == WORKTOP_TAKE_IDENT
                    || fn_identifier.ident == WORKTOP_TAKE_ALL_IDENT
                    || fn_identifier.ident == WORKTOP_TAKE_NON_FUNGIBLES_IDENT
                    || fn_identifier.ident == WORKTOP_DRAIN_IDENT
                {
                    for (_, bucket_snapshot) in self.output.buckets.iter() {
                        worktop_changes_aggregator
                            .entry(self.instruction_index)
                            .or_default()
                            .push(WorktopChange::Take(bucket_snapshot.into()))
                    }
                }
            }
        }

        // Aggregate the worktop changes for all children traces
        for child in self.children.iter() {
            child.worktop_changes(worktop_changes_aggregator)
        }
    }
}

impl ResourceSummary {
    pub fn default() -> Self {
        Self {
            buckets: index_map_new(),
            proofs: index_map_new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty() && self.proofs.is_empty()
    }

    pub fn from_message<Y: KernelApi<M>, M: KernelCallbackObject>(
        api: &mut Y,
        message: &Message,
    ) -> Self {
        let mut buckets = index_map_new();
        let mut proofs = index_map_new();
        for node_id in &message.move_nodes {
            if let Some(x) = api.kernel_read_bucket(node_id) {
                buckets.insert(*node_id, x);
            }
            if let Some(x) = api.kernel_read_proof(node_id) {
                proofs.insert(*node_id, x);
            }
        }
        Self { buckets, proofs }
    }

    pub fn from_node_id<Y: KernelApi<M>, M: KernelCallbackObject>(
        api: &mut Y,
        node_id: &NodeId,
    ) -> Self {
        let mut buckets = index_map_new();
        let mut proofs = index_map_new();
        if let Some(x) = api.kernel_read_bucket(node_id) {
            buckets.insert(*node_id, x);
        }
        if let Some(x) = api.kernel_read_proof(node_id) {
            proofs.insert(*node_id, x);
        }
        Self { buckets, proofs }
    }
}

impl<V: SystemCallbackObject> SystemModule<SystemConfig<V>> for ExecutionTraceModule {
    fn before_create_node<Y: KernelApi<SystemConfig<V>>>(
        api: &mut Y,
        _node_id: &NodeId,
        _node_substates: &NodeSubstates,
    ) -> Result<(), RuntimeError> {
        api.kernel_get_system_state()
            .system
            .modules
            .execution_trace
            .handle_before_create_node();
        Ok(())
    }

    fn after_create_node<Y: KernelApi<SystemConfig<V>>>(
        api: &mut Y,
        node_id: &NodeId,
        _total_substate_size: usize,
        _store_access: &StoreAccessInfo,
    ) -> Result<(), RuntimeError> {
        let current_depth = api.kernel_get_current_depth();
        let resource_summary = ResourceSummary::from_node_id(api, node_id);
        let system_state = api.kernel_get_system_state();
        system_state
            .system
            .modules
            .execution_trace
            .handle_after_create_node(system_state.current, current_depth, resource_summary);
        Ok(())
    }

    fn before_drop_node<Y: KernelApi<SystemConfig<V>>>(
        api: &mut Y,
        node_id: &NodeId,
    ) -> Result<(), RuntimeError> {
        let resource_summary = ResourceSummary::from_node_id(api, node_id);
        api.kernel_get_system_state()
            .system
            .modules
            .execution_trace
            .handle_before_drop_node(resource_summary);
        Ok(())
    }

    fn after_drop_node<Y: KernelApi<SystemConfig<V>>>(
        api: &mut Y,
        _total_substate_size: usize,
    ) -> Result<(), RuntimeError> {
        let current_depth = api.kernel_get_current_depth();
        let system_state = api.kernel_get_system_state();
        system_state
            .system
            .modules
            .execution_trace
            .handle_after_drop_node(system_state.current, current_depth);
        Ok(())
    }

    fn before_push_frame<Y: KernelApi<SystemConfig<V>>>(
        api: &mut Y,
        callee: &Actor,
        update: &mut Message,
        args: &IndexedScryptoValue,
    ) -> Result<(), RuntimeError> {
        let resource_summary = ResourceSummary::from_message(api, update);
        let system_state = api.kernel_get_system_state();
        system_state
            .system
            .modules
            .execution_trace
            .handle_before_push_frame(system_state.current, callee, resource_summary, args);
        Ok(())
    }

    fn on_execution_finish<Y: KernelApi<SystemConfig<V>>>(
        api: &mut Y,
        update: &Message,
    ) -> Result<(), RuntimeError> {
        let current_depth = api.kernel_get_current_depth();
        let resource_summary = ResourceSummary::from_message(api, update);

        let system_state = api.kernel_get_system_state();

        let caller = TraceActor::from_actor(system_state.caller);

        system_state
            .system
            .modules
            .execution_trace
            .handle_on_execution_finish(
                system_state.current,
                current_depth,
                &caller,
                resource_summary,
            );

        Ok(())
    }
}

impl ExecutionTraceModule {
    pub fn new(max_kernel_call_depth_traced: usize) -> ExecutionTraceModule {
        Self {
            max_kernel_call_depth_traced,
            current_instruction_index: 0,
            current_kernel_call_depth: 0,
            traced_kernel_call_inputs_stack: vec![],
            kernel_call_traces_stacks: index_map_new(),
            vault_ops: Vec::new(),
        }
    }

    fn handle_before_create_node(&mut self) {
        if self.current_kernel_call_depth <= self.max_kernel_call_depth_traced {
            let instruction_index = self.instruction_index();

            let traced_input = (
                ResourceSummary::default(),
                TraceOrigin::CreateNode,
                instruction_index,
            );
            self.traced_kernel_call_inputs_stack.push(traced_input);
        }

        self.current_kernel_call_depth += 1;
    }

    fn handle_after_create_node(
        &mut self,
        current_actor: &Actor,
        current_depth: usize,
        resource_summary: ResourceSummary,
    ) {
        // Important to always update the counter (even if we're over the depth limit).
        self.current_kernel_call_depth -= 1;

        if self.current_kernel_call_depth > self.max_kernel_call_depth_traced {
            // Nothing to trace at this depth, exit.
            return;
        }

        let current_actor = TraceActor::from_actor(current_actor);
        self.finalize_kernel_call_trace(resource_summary, current_actor, current_depth)
    }

    fn handle_before_drop_node(&mut self, resource_summary: ResourceSummary) {
        if self.current_kernel_call_depth <= self.max_kernel_call_depth_traced {
            let instruction_index = self.instruction_index();

            let traced_input = (resource_summary, TraceOrigin::DropNode, instruction_index);
            self.traced_kernel_call_inputs_stack.push(traced_input);
        }

        self.current_kernel_call_depth += 1;
    }

    fn handle_after_drop_node(&mut self, current_actor: &Actor, current_depth: usize) {
        // Important to always update the counter (even if we're over the depth limit).
        self.current_kernel_call_depth -= 1;

        if self.current_kernel_call_depth > self.max_kernel_call_depth_traced {
            // Nothing to trace at this depth, exit.
            return;
        }

        let traced_output = ResourceSummary::default();

        let current_actor = TraceActor::from_actor(current_actor);
        self.finalize_kernel_call_trace(traced_output, current_actor, current_depth)
    }

    fn handle_before_push_frame(
        &mut self,
        current_actor: &Actor,
        callee: &Actor,
        resource_summary: ResourceSummary,
        args: &IndexedScryptoValue,
    ) {
        if self.current_kernel_call_depth <= self.max_kernel_call_depth_traced {
            let origin = match &callee {
                Actor::Method(MethodActor {
                    module_object_info: object_info,
                    ident,
                    ..
                }) => TraceOrigin::ScryptoMethod(ApplicationFnIdentifier {
                    package_address: object_info.blueprint_id.package_address.clone(),
                    blueprint_name: object_info.blueprint_id.blueprint_name.clone(),
                    ident: ident.clone(),
                }),
                Actor::Function {
                    blueprint_id: blueprint,
                    ident,
                } => TraceOrigin::ScryptoFunction(ApplicationFnIdentifier {
                    package_address: blueprint.package_address.clone(),
                    blueprint_name: blueprint.blueprint_name.clone(),
                    ident: ident.clone(),
                }),
                Actor::VirtualLazyLoad { .. } | Actor::Root => {
                    return;
                }
            };
            let instruction_index = self.instruction_index();

            self.traced_kernel_call_inputs_stack.push((
                resource_summary.clone(),
                origin,
                instruction_index,
            ));
        }

        self.current_kernel_call_depth += 1;

        match &callee {
            Actor::Method(MethodActor {
                node_id,
                module_object_info: object_info,
                ident,
                ..
            }) if VaultUtil::is_vault_blueprint(&object_info.blueprint_id)
                && ident.eq(VAULT_PUT_IDENT) =>
            {
                self.handle_vault_put_input(&resource_summary, current_actor, node_id)
            }
            Actor::Method(MethodActor {
                node_id,
                module_object_info: object_info,
                ident,
                ..
            }) if VaultUtil::is_vault_blueprint(&object_info.blueprint_id)
                && ident.eq(FUNGIBLE_VAULT_LOCK_FEE_IDENT) =>
            {
                self.handle_vault_lock_fee_input(current_actor, node_id, args)
            }
            _ => {}
        }
    }

    fn handle_on_execution_finish(
        &mut self,
        current_actor: &Actor,
        current_depth: usize,
        caller: &TraceActor,
        resource_summary: ResourceSummary,
    ) {
        match current_actor {
            Actor::Method(MethodActor {
                node_id,
                module_object_info: object_info,
                ident,
                ..
            }) if VaultUtil::is_vault_blueprint(&object_info.blueprint_id)
                && ident.eq(VAULT_TAKE_IDENT) =>
            {
                self.handle_vault_take_output(&resource_summary, &caller, node_id)
            }
            Actor::VirtualLazyLoad { .. } => return,
            _ => {}
        }

        // Important to always update the counter (even if we're over the depth limit).
        self.current_kernel_call_depth -= 1;

        if self.current_kernel_call_depth > self.max_kernel_call_depth_traced {
            // Nothing to trace at this depth, exit.
            return;
        }

        let current_actor = TraceActor::from_actor(current_actor);
        self.finalize_kernel_call_trace(resource_summary, current_actor, current_depth)
    }

    fn finalize_kernel_call_trace(
        &mut self,
        traced_output: ResourceSummary,
        current_actor: TraceActor,
        current_depth: usize,
    ) {
        let child_traces = self
            .kernel_call_traces_stacks
            .remove(&(self.current_kernel_call_depth + 1))
            .unwrap_or(vec![]);

        let (traced_input, origin, instruction_index) = self
            .traced_kernel_call_inputs_stack
            .pop()
            .expect("kernel call input stack underflow");

        // Only include the trace if:
        // * there's a non-empty traced input or output
        // * OR there are any child traces: they need a parent regardless of whether it traces any inputs/outputs.
        //   At some depth (up to the tracing limit) there must have been at least one traced input/output
        //   so we need to include the full path up to the root.
        if !traced_input.is_empty() || !traced_output.is_empty() || !child_traces.is_empty() {
            let trace = ExecutionTrace {
                origin,
                kernel_call_depth: self.current_kernel_call_depth,
                current_frame_actor: current_actor,
                current_frame_depth: current_depth,
                instruction_index,
                input: traced_input,
                output: traced_output,
                children: child_traces,
            };

            let siblings = self
                .kernel_call_traces_stacks
                .entry(self.current_kernel_call_depth)
                .or_insert(vec![]);
            siblings.push(trace);
        }
    }

    pub fn finalize(
        mut self,
        fee_payments: &IndexMap<NodeId, Decimal>,
        is_success: bool,
    ) -> TransactionExecutionTrace {
        let mut execution_traces = Vec::new();
        for (_, traces) in self.kernel_call_traces_stacks.drain(..) {
            execution_traces.extend(traces);
        }

        let fee_locks = calculate_fee_locks(&self.vault_ops);
        let resource_changes = calculate_resource_changes(self.vault_ops, fee_payments, is_success);

        TransactionExecutionTrace {
            execution_traces,
            resource_changes,
            fee_locks,
        }
    }

    fn instruction_index(&self) -> usize {
        self.current_instruction_index
    }

    fn handle_vault_put_input<'s>(
        &mut self,
        resource_summary: &ResourceSummary,
        caller: &Actor,
        vault_id: &NodeId,
    ) {
        let actor = TraceActor::from_actor(caller);
        for (_, resource) in &resource_summary.buckets {
            self.vault_ops.push((
                actor.clone(),
                vault_id.clone(),
                VaultOp::Put(resource.resource_address(), resource.amount()),
                self.instruction_index(),
            ));
        }
    }

    fn handle_vault_lock_fee_input<'s>(
        &mut self,
        caller: &Actor,
        vault_id: &NodeId,
        args: &IndexedScryptoValue,
    ) {
        let actor = TraceActor::from_actor(caller);
        let FungibleVaultLockFeeInput { amount, contingent } = args.as_typed().unwrap();
        self.vault_ops.push((
            actor,
            vault_id.clone(),
            VaultOp::LockFee(amount, contingent),
            self.instruction_index(),
        ));
    }

    fn handle_vault_take_output<'s>(
        &mut self,
        resource_summary: &ResourceSummary,
        actor: &TraceActor,
        vault_id: &NodeId,
    ) {
        for (_, resource) in &resource_summary.buckets {
            self.vault_ops.push((
                actor.clone(),
                vault_id.clone(),
                VaultOp::Take(resource.resource_address(), resource.amount()),
                self.instruction_index(),
            ));
        }
    }
}

pub fn calculate_resource_changes(
    mut vault_ops: Vec<(TraceActor, NodeId, VaultOp, usize)>,
    fee_payments: &IndexMap<NodeId, Decimal>,
    is_commit_success: bool,
) -> IndexMap<usize, Vec<ResourceChange>> {
    // Retain lock fee only if the transaction fails.
    if !is_commit_success {
        vault_ops.retain(|x| matches!(x.2, VaultOp::LockFee(..)));
    }

    // Calculate per instruction index, actor, vault resource changes.
    let mut vault_changes =
        index_map_new::<usize, IndexMap<NodeId, IndexMap<NodeId, (ResourceAddress, Decimal)>>>();
    for (actor, vault_id, vault_op, instruction_index) in vault_ops {
        if let TraceActor::Method(node_id) = actor {
            match vault_op {
                VaultOp::Create(_) => todo!("Not supported yet!"),
                VaultOp::Put(resource_address, amount) => {
                    vault_changes
                        .entry(instruction_index)
                        .or_default()
                        .entry(node_id)
                        .or_default()
                        .entry(vault_id)
                        .or_insert((resource_address, Decimal::zero()))
                        .1 += amount;
                }
                VaultOp::Take(resource_address, amount) => {
                    vault_changes
                        .entry(instruction_index)
                        .or_default()
                        .entry(node_id)
                        .or_default()
                        .entry(vault_id)
                        .or_insert((resource_address, Decimal::zero()))
                        .1 -= amount;
                }
                VaultOp::LockFee(..) => {
                    vault_changes
                        .entry(instruction_index)
                        .or_default()
                        .entry(node_id)
                        .or_default()
                        .entry(vault_id)
                        .or_insert((XRD, Decimal::zero()))
                        .1 -= fee_payments.get(&vault_id).cloned().unwrap_or_default();
                }
            }
        }
    }

    // Convert into a vec for ease of consumption.
    let mut resource_changes = index_map_new::<usize, Vec<ResourceChange>>();
    for (instruction_index, instruction_resource_changes) in vault_changes {
        for (node_id, map) in instruction_resource_changes {
            for (vault_id, (resource_address, delta)) in map {
                // Add a resource change log if non-zero
                if !delta.is_zero() {
                    resource_changes
                        .entry(instruction_index)
                        .or_default()
                        .push(ResourceChange {
                            resource_address,
                            node_id,
                            vault_id,
                            amount: delta,
                        });
                }
            }
        }
    }

    resource_changes
}

pub fn calculate_fee_locks(vault_ops: &Vec<(TraceActor, NodeId, VaultOp, usize)>) -> FeeLocks {
    let mut fee_locks = FeeLocks {
        lock: Decimal::ZERO,
        contingent_lock: Decimal::ZERO,
    };
    for (_, _, vault_op, _) in vault_ops {
        if let VaultOp::LockFee(amount, is_contingent) = vault_op {
            if !is_contingent {
                fee_locks.lock += *amount
            } else {
                fee_locks.contingent_lock += *amount;
            }
        };
    }
    fee_locks
}
