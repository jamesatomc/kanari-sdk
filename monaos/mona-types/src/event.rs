use serde::{Deserialize, Serialize};

/// Emit a custom Move event, sending the data offchain.
/// Corresponds to `kanari_framework::event` module functionality
pub trait EventEmitter {
    /// Emit an event of type T
    fn emit<T: Clone + serde::Serialize>(&self, event: T);
}

/// A trait for types that can be emitted as events
pub trait Event: Clone + serde::Serialize + serde::de::DeserializeOwned {}

/// Automatically implement Event for types that satisfy the requirements
impl<T> Event for T where T: Clone + serde::Serialize + serde::de::DeserializeOwned {}

/// Event data wrapper for serialization/storage
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventData {
    /// The type name of the event
    pub event_type: String,
    /// Serialized event data
    pub data: Vec<u8>,
    /// Timestamp when the event was emitted
    pub timestamp: u64,
    /// Transaction hash that emitted this event
    pub tx_hash: Vec<u8>,
}

impl EventData {
    /// Create new event data
    pub fn new<T: Event>(
        event: T,
        timestamp: u64,
        tx_hash: Vec<u8>,
    ) -> Result<Self, EventError> {
        let event_type = std::any::type_name::<T>().to_string();
        let data = bcs::to_bytes(&event)
            .map_err(|_| EventError::SerializationFailure)?;
        
        Ok(Self {
            event_type,
            data,
            timestamp,
            tx_hash,
        })
    }

    /// Deserialize the event data back to the original type
    pub fn deserialize<T: Event>(&self) -> Result<T, EventError> {
        bcs::from_bytes(&self.data)
            .map_err(|_| EventError::DeserializationFailure)
    }

    /// Get the event type name
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    /// Get the raw event data
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// Get the transaction hash
    pub fn tx_hash(&self) -> &[u8] {
        &self.tx_hash
    }
}

/// Default event emitter that stores events in memory
#[derive(Debug, Clone, Default)]
pub struct MemoryEventEmitter {
    events: std::sync::Arc<std::sync::Mutex<Vec<EventData>>>,
}

impl MemoryEventEmitter {
    /// Create a new memory event emitter
    pub fn new() -> Self {
        Self {
            events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Get all emitted events
    pub fn get_events(&self) -> Vec<EventData> {
        self.events.lock().unwrap().clone()
    }

    /// Clear all events
    pub fn clear_events(&self) {
        self.events.lock().unwrap().clear();
    }

    /// Get events by type
    pub fn get_events_by_type(&self, event_type: &str) -> Vec<EventData> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.event_type == event_type)
            .cloned()
            .collect()
    }
}

impl EventEmitter for MemoryEventEmitter {
    fn emit<T: Clone + serde::Serialize>(&self, event: T) {
        // In a real implementation, you'd get these from the current transaction context
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let tx_hash = vec![0u8; 32]; // Placeholder

        // Create event data directly to avoid trait bound issues
        let event_type = std::any::type_name::<T>().to_string();
        if let Ok(data) = bcs::to_bytes(&event) {
            let event_data = EventData {
                event_type,
                data,
                timestamp,
                tx_hash,
            };
            self.events.lock().unwrap().push(event_data);
        }
    }
}

/// Event system errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    #[error("Failed to serialize event data")]
    SerializationFailure,
    #[error("Failed to deserialize event data")]
    DeserializationFailure,
    #[error("Event type mismatch")]
    TypeMismatch,
}

/// Common event types used in the system

/// Balance change event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceChangeEvent {
    pub account: crate::address::Address,
    pub old_balance: u64,
    pub new_balance: u64,
    pub change_type: BalanceChangeType,
}

/// Type of balance change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BalanceChangeType {
    Mint,
    Burn,
    Transfer,
    Split,
    Join,
}

/// Coin creation event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoinCreatedEvent {
    pub coin_type: String,
    pub creator: crate::address::Address,
    pub initial_supply: u64,
    pub metadata: CoinMetadataEvent,
}

/// Coin metadata event data
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoinMetadataEvent {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub description: String,
    pub icon_url: Option<String>,
}

/// Transfer event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferEvent {
    pub from: crate::address::Address,
    pub to: crate::address::Address,
    pub amount: u64,
    pub coin_type: String,
}

/// Object creation event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectCreatedEvent {
    pub object_id: crate::object::ID,
    pub object_type: String,
    pub creator: crate::address::Address,
}

/// Object deletion event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDeletedEvent {
    pub object_id: crate::object::ID,
    pub object_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;

    #[test]
    fn test_event_emission() {
        let emitter = MemoryEventEmitter::new();
        
        let event = TransferEvent {
            from: Address::zero(),
            to: Address::zero(),
            amount: 100,
            coin_type: "KARI".to_string(),
        };
        
        emitter.emit(event.clone());
        
        let events = emitter.get_events();
        assert_eq!(events.len(), 1);
        
        let deserialized: TransferEvent = events[0].deserialize().unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn test_event_filtering() {
        let emitter = MemoryEventEmitter::new();
        
        let transfer_event = TransferEvent {
            from: Address::zero(),
            to: Address::zero(),
            amount: 100,
            coin_type: "KARI".to_string(),
        };
        
        let balance_event = BalanceChangeEvent {
            account: Address::zero(),
            old_balance: 0,
            new_balance: 100,
            change_type: BalanceChangeType::Mint,
        };
        
        emitter.emit(transfer_event);
        emitter.emit(balance_event);
        
        let all_events = emitter.get_events();
        assert_eq!(all_events.len(), 2);
        
        let transfer_events = emitter.get_events_by_type(std::any::type_name::<TransferEvent>());
        assert_eq!(transfer_events.len(), 1);
    }
}
