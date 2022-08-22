use clap::Parser;
use radix_engine::constants::*;
use radix_engine::engine::Track;
use radix_engine::engine::{ExecutionTrace, Kernel, SystemApi};
use radix_engine::fee::{FeeTable, SystemLoanFeeReserve};
use radix_engine::types::*;
use radix_engine_stores::rocks_db::RadixEngineDB;

use crate::resim::*;

/// Set the current epoch
#[derive(Parser, Debug)]
pub struct SetCurrentEpoch {
    /// The new epoch number
    epoch: u64,
}

impl SetCurrentEpoch {
    pub fn run<O: std::io::Write>(&self, _out: &mut O) -> Result<(), Error> {
        // TODO: can we construct a proper transaction to do the following?

        let tx_hash = hash(get_nonce()?.to_string());
        let mut substate_store = RadixEngineDB::with_bootstrap(get_data_dir()?);
        let mut wasm_engine = DefaultWasmEngine::new();
        let mut wasm_instrumenter = WasmInstrumenter::new();
        let mut track = Track::new(&substate_store, SystemLoanFeeReserve::default());
        let mut fee_reserve = SystemLoanFeeReserve::default();
        let fee_table = FeeTable::new();
        let mut execution_trace = ExecutionTrace::new();

        let mut kernel = Kernel::new(
            tx_hash,
            vec![],
            true,
            DEFAULT_MAX_CALL_DEPTH,
            false,
            &mut track,
            &mut wasm_engine,
            &mut wasm_instrumenter,
            &mut fee_reserve,
            &fee_table,
            &mut execution_trace,
        );

        // Invoke the system
        kernel
            .invoke_method(
                Receiver::Ref(RENodeId::System),
                FnIdentifier::Native(NativeFnIdentifier::System(SystemFnIdentifier::SetEpoch)),
                ScryptoValue::from_typed(&SystemSetEpochInput { epoch: self.epoch }),
            )
            .map(|_| ())
            .map_err(Error::TransactionExecutionError)?;

        // Commit
        track.commit();
        let receipt = track.to_receipt();
        receipt.state_updates.commit(&mut substate_store);

        Ok(())
    }
}
