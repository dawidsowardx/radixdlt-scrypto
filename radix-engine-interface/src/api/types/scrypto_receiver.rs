use crate::api::types::*;
use crate::*;

#[derive(Debug, Copy, Clone, Eq, PartialEq, ScryptoCategorize, ScryptoEncode, ScryptoDecode)]
pub enum ScryptoReceiver {
    Global(ComponentAddress),
    Resource(ResourceAddress),
    Package(PackageAddress),
    Vault(VaultId),
    Component(ComponentId),
    Proof(ProofId),
    Bucket(BucketId),
    Worktop,
    Logger,
    TransactionRuntime,
    AuthZoneStack,
}
