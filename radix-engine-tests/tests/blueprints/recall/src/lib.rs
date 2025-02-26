use scrypto::api::*;
use scrypto::engine::scrypto_env::ScryptoEnv;
use scrypto::prelude::*;

#[blueprint]
mod recall {
    struct RecallTest {
        vault: Vault,
    }

    impl RecallTest {
        pub fn new() -> Global<RecallTest> {
            let bucket = ResourceBuilder::new_fungible(OwnerRole::None)
                .mint_roles(mint_roles! {
                    minter => rule!(allow_all);
                    minter_updater => rule!(deny_all);
                })
                .burn_roles(burn_roles! {
                    burner => rule!(allow_all);
                    burner_updater => rule!(deny_all);
                })
                .recall_roles(recall_roles! {
                    recaller => rule!(allow_all);
                    recaller_updater => rule!(deny_all);
                })
                .metadata(metadata! {
                    init {
                        "name" => "TestToken".to_owned(), locked;
                    }
                })
                .mint_initial_supply(500);

            Self {
                vault: Vault::with_bucket(bucket),
            }
            .instantiate()
            .prepare_to_globalize(OwnerRole::None)
            .globalize()
        }

        pub fn recall_on_internal_vault(&self) -> Bucket {
            scrypto_decode(
                &ScryptoEnv
                    .call_method_advanced(
                        self.vault.0.as_node_id(),
                        true,
                        ObjectModuleId::Main,
                        VAULT_RECALL_IDENT,
                        scrypto_args!(Decimal::ONE),
                    )
                    .unwrap(),
            )
            .unwrap()
        }

        pub fn recall_on_direct_access_ref(reference: InternalAddress) -> Bucket {
            scrypto_decode(
                &ScryptoEnv
                    .call_method_advanced(
                        reference.as_node_id(),
                        true,
                        ObjectModuleId::Main,
                        VAULT_RECALL_IDENT,
                        scrypto_args!(Decimal::ONE),
                    )
                    .unwrap(),
            )
            .unwrap()
        }
    }
}
