// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod address;
pub mod balance;
pub mod block;
pub mod clock;
pub mod coin;
pub mod collection;
pub mod deny_list;
pub mod error;
pub mod event;
pub mod gas_coin;
pub mod object;
pub mod pay;
pub mod transaction;
pub mod transfer;
pub mod tx_context;

/// Active gas model facade. Switch the selected implementation inside
/// [`gas`] while keeping downstream imports stable.
pub mod gas;
pub use gas::{
    GAS_MODEL, GasConfig, GasError, GasEstimate, GasMeter, GasOperation, TransactionGas,
    effective_gas_price, gas_price_is_valid,
};

// Move Standard Library bindings.
//
// Re-export Move stdlib bindings at crate root for easier access and to align
// Rust API names with Move module names.
pub mod stdlib;
pub use stdlib::*;
