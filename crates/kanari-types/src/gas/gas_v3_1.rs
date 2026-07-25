// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Gas v3.1: fivefold-discounted pricing versus gas v1.
//!
//! Prices are integer Mist values. The effective price is therefore rounded
//! down when the requested price is not divisible by five.

pub use super::gas_v1::{GasConfig, GasError, GasEstimate, GasMeter, GasOperation, TransactionGas};

/// v3.1 discount divisor: v1 price / 5.
pub const V3_1_PRICE_DISCOUNT: u64 = 5;
pub const GAS_MODEL: &str = "v3.1";

pub fn effective_gas_price(requested: u64) -> u64 {
    requested / V3_1_PRICE_DISCOUNT
}

pub fn gas_price_is_valid(requested: u64) -> bool {
    requested > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_fivefold_discount() {
        assert_eq!(effective_gas_price(5), 1);
        assert_eq!(effective_gas_price(100), 20);
        assert_eq!(effective_gas_price(4), 0);
        assert!(gas_price_is_valid(1));
        assert!(!gas_price_is_valid(0));
    }
}
