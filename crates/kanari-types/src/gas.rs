#[cfg(not(feature = "zero-gas"))]
pub use crate::gas_v1::*;

#[cfg(feature = "zero-gas")]
pub use crate::gas_v2::*;
