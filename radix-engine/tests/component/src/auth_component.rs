use scrypto::prelude::*;

blueprint! {
    struct AuthComponent {
        some_non_fungible: NonFungibleAddress,
    }

    impl AuthComponent {
        pub fn create_component(some_non_fungible: NonFungibleAddress) -> ComponentId {
            Self { some_non_fungible }
                .instantiate()
                .auth(
                    "get_secret",
                    auth!(require!(SchemaPath::new().field("some_non_fungible"))),
                )
                .globalize()
        }

        pub fn get_secret(&self) -> String {
            "Secret".to_owned()
        }

        pub fn update_auth(&mut self, some_non_fungible: NonFungibleAddress) {
            self.some_non_fungible = some_non_fungible;
        }
    }
}
