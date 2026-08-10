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

#[cfg(test)]
mod tests {
    use super::{gas_v1, gas_v2, gas_v3, gas_v3_1};

    type GasPolicy = (&'static str, fn(u64) -> u64, fn(u64) -> bool);

    #[test]
    fn priced_gas_models_never_turn_valid_nonzero_price_into_zero_cost() {
        let priced_models: &[GasPolicy] = &[
            (
                gas_v1::GAS_MODEL,
                gas_v1::effective_gas_price,
                gas_v1::gas_price_is_valid,
            ),
            (
                gas_v3::GAS_MODEL,
                gas_v3::effective_gas_price,
                gas_v3::gas_price_is_valid,
            ),
            (
                gas_v3_1::GAS_MODEL,
                gas_v3_1::effective_gas_price,
                gas_v3_1::gas_price_is_valid,
            ),
        ];

        for (model, effective_gas_price, gas_price_is_valid) in priced_models {
            for requested in 1..=10 {
                assert!(
                    !gas_price_is_valid(requested) || effective_gas_price(requested) > 0,
                    "{model} accepted gas price {requested} but mapped it to zero cost"
                );
            }
        }
    }

    #[test]
    fn zero_fee_model_explicitly_maps_every_price_to_zero_cost() {
        assert!(gas_v2::gas_price_is_valid(0));
        assert_eq!(gas_v2::effective_gas_price(0), 0);
        assert_eq!(gas_v2::effective_gas_price(1), 0);
        assert_eq!(gas_v2::GAS_MODEL, "v2");
    }
}
