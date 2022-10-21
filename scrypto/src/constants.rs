use crate::component::{ComponentAddress, PackageAddress};
use crate::resource::*;
use crate::{address, construct_address};

// After changing Radix Engine ID allocation, you will most likely need to update the addresses below.
//
// To obtain the new addresses, uncomment the println code in `id_allocator.rs` and
// run `cd radix-engine && cargo test -- bootstrap_receipt_should_match_constants --nocapture`.
//
// We've arranged the addresses in the order they're created in the genesis transaction.

/// The address of the sys-faucet package.
pub const SYS_FAUCET_PACKAGE: PackageAddress = construct_address!(
    EntityType::Package,
    117,
    149,
    161,
    192,
    155,
    192,
    68,
    56,
    79,
    186,
    128,
    155,
    199,
    188,
    92,
    59,
    83,
    241,
    146,
    178,
    126,
    213,
    55,
    167,
    164,
    201
);

/// The address of the account package.
pub const ACCOUNT_PACKAGE: PackageAddress = construct_address!(
    EntityType::Package,
    185,
    23,
    55,
    238,
    138,
    77,
    229,
    157,
    73,
    218,
    212,
    13,
    229,
    86,
    14,
    87,
    84,
    70,
    106,
    200,
    76,
    245,
    67,
    46,
    169,
    93
);

/// The ECDSA virtual resource address.
pub const ECDSA_SECP256K1_TOKEN: ResourceAddress = construct_address!(
    EntityType::Resource,
    146,
    35,
    6,
    166,
    209,
    58,
    246,
    56,
    102,
    182,
    136,
    201,
    16,
    55,
    25,
    208,
    75,
    20,
    192,
    96,
    188,
    72,
    153,
    166,
    19,
    181
);

/// The system token which allows access to system resources (e.g. setting epoch)
pub const SYSTEM_TOKEN: ResourceAddress = construct_address!(
    EntityType::Resource,
    173,
    130,
    50,
    141,
    112,
    34,
    61,
    91,
    174,
    38,
    130,
    96,
    179,
    4,
    93,
    204,
    113,
    220,
    243,
    95,
    55,
    167,
    67,
    74,
    9,
    105
);

/// The XRD resource address.
pub const RADIX_TOKEN: ResourceAddress = address!(
    EntityType::Resource,
    31,
    188,
    195,
    10,
    38,
    63,
    189,
    161,
    194,
    88,
    254,
    183,
    192,
    99,
    206,
    1,
    203,
    126,
    46,
    93,
    121,
    34,
    107,
    247,
    111,
    22
);

/// The address of the SysFaucet component
pub const SYS_FAUCET_COMPONENT: ComponentAddress = construct_address!(
    EntityType::NormalComponent,
    115,
    9,
    63,
    87,
    114,
    161,
    225,
    209,
    191,
    174,
    22,
    244,
    105,
    12,
    88,
    40,
    227,
    50,
    217,
    76,
    172,
    184,
    235,
    208,
    222,
    10
);

pub const SYS_SYSTEM_COMPONENT: ComponentAddress = construct_address!(
    EntityType::SystemComponent,
    172,
    110,
    120,
    193,
    250,
    70,
    187,
    76,
    68,
    171,
    211,
    30,
    43,
    73,
    30,
    13,
    198,
    37,
    110,
    194,
    242,
    109,
    76,
    165,
    200,
    50
);

/// The ED25519 virtual resource address.
pub const EDDSA_ED25519_TOKEN: ResourceAddress = address!(
    EntityType::Resource,
    112,
    80,
    185,
    38,
    180,
    181,
    171,
    151,
    101,
    224,
    68,
    235,
    5,
    132,
    5,
    4,
    142,
    77,
    126,
    195,
    109,
    190,
    183,
    241,
    137,
    99
);
