#[cfg(feature = "gas-v1")]
pub use crate::gas_v1::*;

#[cfg(all(not(feature = "gas-v1"), feature = "gas-v2"))]
pub use crate::gas_v2::*;

#[cfg(not(any(feature = "gas-v1", feature = "gas-v2")))]
compile_error!("Select a gas implementation with feature `gas-v1` or `gas-v2`");
