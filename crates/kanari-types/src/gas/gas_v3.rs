// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Gas v3: integer-priced gas with a tenfold discount versus gas v1.
//!
//! Gas prices are denominated in Mist (`u64`), so the discount is applied with
//! integer division. Requests below ten Mist per unit therefore round down to
//! zero; callers that need a non-zero charge should submit at least 10 Mist.

pub use super::gas_v1::{GasConfig, GasError, GasEstimate, GasMeter, GasOperation, TransactionGas};

/// v3 reduces the effective v1 price by a factor of ten.
pub const V3_PRICE_DISCOUNT: u64 = 10;
pub const GAS_MODEL: &str = "v3";

pub fn effective_gas_price(requested: u64) -> u64 {
    requested / V3_PRICE_DISCOUNT
}

pub fn gas_price_is_valid(requested: u64) -> bool {
    requested > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_tenfold_discount() {
        assert_eq!(effective_gas_price(10), 1);
        assert_eq!(effective_gas_price(100), 10);
        assert_eq!(effective_gas_price(9), 0);
        assert!(gas_price_is_valid(1));
        assert!(!gas_price_is_valid(0));
    }
}
