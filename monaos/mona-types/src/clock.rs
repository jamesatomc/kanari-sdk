// Clock system for timestamp management
// Corresponds to `kanari_framework::clock` module
use serde::{Deserialize, Serialize};
use crate::object::UID;

/// Singleton shared object that exposes time to Move calls.
/// This object is found at a fixed address and can only be read via immutable reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    pub id: UID,
    /// The clock's timestamp as milliseconds since Unix epoch
    pub timestamp_ms: u64,
}

impl Clock {
    /// Create a new clock (only called during genesis)
    pub fn new(id: UID, timestamp_ms: u64) -> Self {
        Self { id, timestamp_ms }
    }

    /// Get the current timestamp in milliseconds
    pub fn timestamp_ms(&self) -> u64 {
        self.timestamp_ms
    }

    /// Update the clock timestamp (system function)
    pub fn set_timestamp_ms(&mut self, timestamp_ms: u64) {
        self.timestamp_ms = timestamp_ms;
    }

    /// Get the timestamp as seconds (convenience function)
    pub fn timestamp_s(&self) -> u64 {
        self.timestamp_ms / 1000
    }

    /// Check if the clock is ahead of a given timestamp
    pub fn is_ahead_of(&self, timestamp_ms: u64) -> bool {
        self.timestamp_ms > timestamp_ms
    }
}

/// Clock error types
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClockError {
    #[error("Not authorized to modify clock")]
    NotAuthorized,
    #[error("Invalid timestamp")]
    InvalidTimestamp,
}

/// Clock operation results
pub type ClockResult<T> = Result<T, ClockError>;
