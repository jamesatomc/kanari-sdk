// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Gas v3.1: fivefold-discounted pricing versus gas v1.
//!
//! Prices are integer Mist values. The effective price is discounted, but any
//! non-zero requested price floors to at least one Mist per gas unit so a valid
//! priced transaction is never charged as zero.

pub use super::gas_v1::{GasConfig, GasError, GasEstimate, GasMeter, GasOperation, TransactionGas};

/// v3.1 discount divisor: v1 price / 5.
pub const V3_1_PRICE_DISCOUNT: u64 = 5;
pub const GAS_MODEL: &str = "v3.1";

pub fn effective_gas_price(requested: u64) -> u64 {
    if requested == 0 {
        return 0;
    }
    (requested / V3_1_PRICE_DISCOUNT).max(1)
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
        assert_eq!(effective_gas_price(4), 1);
        assert_eq!(effective_gas_price(1), 1);
        assert_eq!(effective_gas_price(0), 0);
        assert!(gas_price_is_valid(1));
        assert!(!gas_price_is_valid(0));
    }
}
