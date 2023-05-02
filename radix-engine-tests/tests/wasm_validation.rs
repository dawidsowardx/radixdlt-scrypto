use radix_engine::vm::wasm::{InvalidMemory, PrepareError, WasmValidator};
use scrypto_unit::*;

#[test]
fn test_large_data() {
    let code = wat2wasm(&include_str!("wasm/large_data.wat"));
    let schema = single_function_package_schema("Test", "f");
    let result = WasmValidator::default().validate(&code, &schema);

    assert!(matches!(
        result,
        Err(PrepareError::NotInstantiatable { .. })
    ));
}

#[test]
fn test_large_memory() {
    let code = wat2wasm(&include_str!("wasm/large_memory.wat"));
    let schema = single_function_package_schema("Test", "f");
    let result = WasmValidator::default().validate(&code, &schema);

    assert_eq!(
        Err(PrepareError::InvalidMemory(
            InvalidMemory::InitialMemorySizeLimitExceeded
        )),
        result
    );
}
