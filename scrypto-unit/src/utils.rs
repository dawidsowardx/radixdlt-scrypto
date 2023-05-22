use radix_engine_interface::network::NetworkDefinition;
use scrypto::prelude::hash;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};
use transaction::errors::TransactionValidationError;
use transaction::manifest::{decompile, DecompileError};
use transaction::model::TransactionManifest;
use transaction::validation::NotarizedTransactionValidator;

pub fn dump_manifest_to_file_system<P>(
    manifest: &TransactionManifest,
    directory_path: P,
    network_definition: &NetworkDefinition,
) -> Result<(), DumpManifestError>
where
    P: AsRef<Path>,
{
    let path = directory_path.as_ref().to_owned();

    // Check that the path is a directory and not a file
    if path.is_file() {
        return Err(DumpManifestError::PathPointsToAFile(path));
    }

    // If the directory does not exist, then create it.
    create_dir_all(&path)?;

    // Decompile the transaction manifest to the manifest string and then write it to the
    // directory
    {
        let manifest_string = decompile(&manifest.instructions, network_definition)?;
        let manifest_path = path.join("transaction.rtm");
        std::fs::write(manifest_path, manifest_string)?;
    }

    // Write all of the blobs to the specified path
    for blob in &manifest.blobs {
        let blob_hash = hash(blob);
        let blob_path = path.join(format!("{blob_hash}.blob"));
        std::fs::write(blob_path, blob)?;
    }

    // Validate the manifest
    NotarizedTransactionValidator::validate_manifest(manifest)?;

    Ok(())
}

#[derive(Debug)]
pub enum DumpManifestError {
    PathPointsToAFile(PathBuf),
    IoError(std::io::Error),
    DecompileError(DecompileError),
    TransactionValidationError(TransactionValidationError),
}

impl From<std::io::Error> for DumpManifestError {
    fn from(value: std::io::Error) -> Self {
        Self::IoError(value)
    }
}

impl From<DecompileError> for DumpManifestError {
    fn from(value: DecompileError) -> Self {
        Self::DecompileError(value)
    }
}

impl From<TransactionValidationError> for DumpManifestError {
    fn from(value: TransactionValidationError) -> Self {
        Self::TransactionValidationError(value)
    }
}
