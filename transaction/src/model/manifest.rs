use crate::model::Instruction;
use scrypto::scrypto;

#[derive(Debug, Clone, PartialEq, Eq)]
#[scrypto(TypeId, Encode, Decode)]
pub struct TransactionManifest {
    pub instructions: Vec<Instruction>,
    pub blobs: Vec<Vec<u8>>,
}
