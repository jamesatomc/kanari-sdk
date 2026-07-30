// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Gas v3: integer-priced gas with a tenfold discount versus gas v1.
//!
//! Gas prices are denominated in Mist (`u64`), so the discount is applied with
//! integer division. Any non-zero requested price floors to at least one Mist
//! per gas unit so valid priced transactions are never charged as zero.

pub use super::gas_v1::{GasConfig, GasError, GasEstimate, GasMeter, GasOperation, TransactionGas};

/// v3 reduces the effective v1 price by a factor of ten.
pub const V3_PRICE_DISCOUNT: u64 = 10;
pub const GAS_MODEL: &str = "v3";

pub fn effective_gas_price(requested: u64) -> u64 {
    if requested == 0 {
        return 0;
    }
    (requested / V3_PRICE_DISCOUNT).max(1)
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
        assert_eq!(effective_gas_price(9), 1);
        assert_eq!(effective_gas_price(1), 1);
        assert_eq!(effective_gas_price(0), 0);
        assert!(gas_price_is_valid(1));
        assert!(!gas_price_is_valid(0));
    }
}
