use colored::*;
use radix_engine::ledger::*;
use radix_engine::model::*;
use scrypto::engine::types::*;
use scrypto::rust::collections::HashSet;

use crate::utils::*;

/// Represents an error when displaying an entity.
#[derive(Debug, Clone)]
pub enum DisplayError {
    PackageNotFound,
    ComponentNotFound,
    ResourceDefNotFound,
}

/// Dump a package into console.
pub fn dump_package<T: SubstateStore>(
    package_id: PackageId,
    ledger: &T,
) -> Result<(), DisplayError> {
    let package = ledger.get_package(package_id);
    match package {
        Some(b) => {
            println!("{}: {}", "Package".green().bold(), package_id.to_string());
            println!("{}: {} bytes", "Code size".green().bold(), b.code().len());
            Ok(())
        }
        None => Err(DisplayError::PackageNotFound),
    }
}

/// Dump a component into console.
pub fn dump_component<T: SubstateStore + QueryableSubstateStore>(
    component_id: ComponentId,
    ledger: &T,
) -> Result<(), DisplayError> {
    let component = ledger.get_component(component_id);
    match component {
        Some(c) => {
            println!(
                "{}: {}",
                "Component".green().bold(),
                component_id.to_string()
            );

            println!(
                "{}: {{ package_id: {}, blueprint_name: \"{}\" }}",
                "Blueprint".green().bold(),
                c.package_id(),
                c.blueprint_name()
            );
            let state = c.state();
            let state_validated = ValidatedData::from_slice(state).unwrap();
            println!("{}: {}", "State".green().bold(), state_validated);

            // The current implementation recursively displays all referenced maps and vaults which
            // the component may not have access to.
            // Dump lazy map using DFS
            // Consider using a proper Queue structure
            let mut queue: Vec<LazyMapId> = state_validated.lazy_map_ids.clone();
            let mut i = 0;
            let mut maps_visited: HashSet<LazyMapId> = HashSet::new();
            let mut vaults_found: HashSet<VaultId> =
                state_validated.vault_ids.iter().cloned().collect();
            while i < queue.len() {
                let lazy_map_id = queue[i];
                i += 1;
                if maps_visited.insert(lazy_map_id) {
                    let (maps, vaults) = dump_lazy_map(component_id, &lazy_map_id, ledger)?;
                    queue.extend(maps);
                    for v in vaults {
                        vaults_found.insert(v);
                    }
                }
            }

            // Dump resources
            dump_resources(component_id, &vaults_found, ledger)
        }
        None => Err(DisplayError::ComponentNotFound),
    }
}

fn dump_lazy_map<T: SubstateStore + QueryableSubstateStore>(
    component_id: ComponentId,
    lazy_map_id: &LazyMapId,
    substate_store: &T,
) -> Result<(Vec<LazyMapId>, Vec<VaultId>), DisplayError> {
    let mut referenced_maps = Vec::new();
    let mut referenced_vaults = Vec::new();
    let map = substate_store.get_lazy_map_entries(component_id, lazy_map_id);
    println!(
        "{}: {:?}{:?}",
        "Lazy Map".green().bold(),
        component_id,
        lazy_map_id
    );
    for (last, (k, v)) in map.iter().identify_last() {
        let v_validated = ValidatedData::from_slice(v).unwrap();
        println!("{} {:?} => {}", list_item_prefix(last), k, v_validated);
        referenced_maps.extend(v_validated.lazy_map_ids);
        referenced_vaults.extend(v_validated.vault_ids);
    }
    Ok((referenced_maps, referenced_vaults))
}

fn dump_resources<T: SubstateStore>(
    component_id: ComponentId,
    vaults: &HashSet<VaultId>,
    ledger: &T,
) -> Result<(), DisplayError> {
    println!("{}:", "Resources".green().bold());
    for (last, vault_id) in vaults.iter().identify_last() {
        let vault = ledger.get_vault(component_id, vault_id);
        let amount = vault.amount();
        let resource_def_id = vault.resource_def_id();
        let resource_def = ledger.get_resource_def(resource_def_id).unwrap();
        println!(
            "{} {{ amount: {}, resource definition: {}{}{} }}",
            list_item_prefix(last),
            amount,
            resource_def_id,
            resource_def
                .metadata()
                .get("name")
                .map(|name| format!(", name: \"{}\"", name))
                .unwrap_or(String::new()),
            resource_def
                .metadata()
                .get("symbol")
                .map(|symbol| format!(", symbol: \"{}\"", symbol))
                .unwrap_or(String::new()),
        );
        if let Resource::NonFungible { keys } = vault.resource() {
            for (inner_last, key) in keys.iter().identify_last() {
                let non_fungible = ledger.get_non_fungible(resource_def_id, key).unwrap();
                let immutable_data =
                    ValidatedData::from_slice(&non_fungible.immutable_data()).unwrap();
                let mutable_data = ValidatedData::from_slice(&non_fungible.mutable_data()).unwrap();
                println!(
                    "{}  {} NON_FUNGIBLE {{ id: {}, immutable_data: {}, mutable_data: {} }}",
                    if last { " " } else { "│" },
                    list_item_prefix(inner_last),
                    key,
                    immutable_data,
                    mutable_data
                );
            }
        }
    }
    Ok(())
}

/// Dump a resource definition into console.
pub fn dump_resource_def<T: SubstateStore>(
    resource_def_id: ResourceDefId,
    ledger: &T,
) -> Result<(), DisplayError> {
    let resource_def = ledger.get_resource_def(resource_def_id);
    match resource_def {
        Some(r) => {
            println!(
                "{}: {:?}",
                "Resource Type".green().bold(),
                r.resource_type()
            );
            println!("{}: {}", "Metadata".green().bold(), r.metadata().len());
            for (last, e) in r.metadata().iter().identify_last() {
                println!("{} {}: {}", list_item_prefix(last), e.0.green().bold(), e.1);
            }
            println!("{}: {}", "Flags".green().bold(), r.flags());
            println!("{}: {}", "Mutable Flags".green().bold(), r.mutable_flags());
            println!("{}: {:?}", "Authorities".green().bold(), r.authorities());
            println!("{}: {}", "Total Supply".green().bold(), r.total_supply());
            Ok(())
        }
        None => Err(DisplayError::ResourceDefNotFound),
    }
}
