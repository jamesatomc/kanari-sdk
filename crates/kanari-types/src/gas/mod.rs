// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Gas model selector.
//!
//! Keep consumers importing `kanari_types::gas::*` or crate-root re-exports
//! while switching the active policy in one place.

pub mod gas_v1;

pub mod gas_v2;

pub mod gas_v3;

pub mod gas_v3_1;

/// Select the active gas implementation here.
///
/// Change the selector to `v1`, `v2`, `v3`, or `v3_1` to choose the active
/// pricing policy. All consumers re-export the selected policy from this
/// module.
macro_rules! select_gas_impl {
    (v1) => {
        pub use self::gas_v1::*;
    };
    (v2) => {
        pub use self::gas_v2::*;
    };
    (v3) => {
        pub use self::gas_v3::*;
    };
    (v3_1) => {
        pub use self::gas_v3_1::*;
    };
}

select_gas_impl!(v2);
