//! Consensus gas schedule.
//!
//! The active schedule must not depend on Cargo features because validators built
//! with different features would disagree on transaction validity and state roots.
pub use crate::gas_v1::*;
