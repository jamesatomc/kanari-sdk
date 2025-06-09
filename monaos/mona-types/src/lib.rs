
// Core blockchain and Move VM types for mona-vm integration
// This crate provides Rust types that correspond to Move framework types

pub mod address;
pub mod balance;
pub mod bag;
pub mod clock;
pub mod coin;
pub mod crypto;
pub mod display;
pub mod event;
pub mod gas_coin;
pub mod object;
pub mod random;
pub mod table;
pub mod token;
pub mod transfer;
pub mod tx_context;
pub mod utils;
pub mod vec_collections;
pub mod versioned;

// Re-export common types for convenience
pub use address::Address;
pub use balance::{Balance, Supply};
pub use bag::Bag;
pub use clock::Clock;
pub use coin::{Coin, CoinMetadata, TreasuryCap, DenyCap, ObjectId};
pub use crypto::{blake2b256, keccak256, sha256, ed25519_verify};
pub use display::{Display, Url};
pub use event::{Event, EventData, EventEmitter, MemoryEventEmitter};
pub use object::{ID, UID};
pub use random::Random;
pub use table::{Table, LinkedTable};
pub use token::{Token, TokenPolicy, TokenPolicyCap, ActionRequest, TokenAction};
pub use transfer::{Transfer, Receiving, TransferableObject, PublicTransferableObject};
pub use tx_context::TxContext;
pub use utils::{Utils, Validator, constants};
pub use vec_collections::{VecMap, VecSet, PriorityQueue};
pub use versioned::Versioned;

// Re-export KARI-specific types and gas fee operations
pub use gas_coin::{KARI, KariCoin, KariBalance, KariTreasuryCap, KariMetadata, KariOps};
pub use gas_coin::{calculate_gas_fee, format_gas_fee_display, validate_transaction_funds};
pub use gas_coin::{KariTransfer, KariBurn, KariMint, NetworkStats};

/// Common result type used throughout mona-types
pub type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = std::result::Result<T, E>;