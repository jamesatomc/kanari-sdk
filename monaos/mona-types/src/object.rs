use serde::{Deserialize, Serialize};
use crate::address::Address;

/// An object ID used to reference objects.
/// Corresponds to `kanari_framework::object::ID` in Move
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ID {
    /// We use `address` instead of `vector<u8>` for more compact serialization
    pub bytes: Address,
}

/// Globally unique IDs that define an object's ID in storage.
/// Corresponds to `kanari_framework::object::UID` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UID {
    pub id: ID,
}

impl ID {
    /// Create a new ID from an address
    pub fn new(address: Address) -> Self {
        Self { bytes: address }
    }    /// Get the raw bytes of an ID
    pub fn to_bytes(&self) -> [u8; 32] {
        *self.bytes.to_bytes()
    }

    /// Get the inner bytes as an address
    pub fn to_address(&self) -> Address {
        self.bytes
    }

    /// Make an ID from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            bytes: Address::new(bytes),
        }
    }

    /// Make an ID from an address
    pub fn from_address(address: Address) -> Self {
        Self { bytes: address }
    }
}

impl UID {
    /// Create a new UID from an ID
    pub fn new(id: ID) -> Self {
        Self { id }
    }

    /// Get the ID
    pub fn id(&self) -> &ID {
        &self.id
    }

    /// Convert to ID
    pub fn to_id(&self) -> ID {
        self.id
    }

    /// Get the address
    pub fn to_address(&self) -> Address {
        self.id.to_address()
    }

    /// Get raw bytes
    pub fn to_bytes(&self) -> [u8; 32] {
        self.id.to_bytes()
    }
}

/// Object system constants
pub mod constants {
    use crate::address::Address;

    /// The hardcoded ID for the singleton Kari System State Object
    pub const KARI_SYSTEM_STATE_OBJECT_ID: Address = Address::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5]);

    /// The hardcoded ID for the singleton Clock Object
    pub const KARI_CLOCK_OBJECT_ID: Address = Address::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6]);

    /// The hardcoded ID for the singleton AuthenticatorState Object
    pub const KARI_AUTHENTICATOR_STATE_ID: Address = Address::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7]);

    /// The hardcoded ID for the singleton Random Object
    pub const KARI_RANDOM_ID: Address = Address::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8]);

    /// The hardcoded ID for the singleton DenyList
    pub const KARI_DENY_LIST_OBJECT_ID: Address = Address::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 3, 0, 0]);
}

/// Object operation errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObjectError {
    #[error("Not system address")]
    NotSystemAddress,
}

/// Object error constants matching Move constants
pub mod error_constants {
    /// Sender is not @0x0 the system address
    pub const E_NOT_SYSTEM_ADDRESS: u64 = 0;
}

impl Default for ID {
    fn default() -> Self {
        Self::new(Address::zero())
    }
}

impl Default for UID {
    fn default() -> Self {
        Self::new(ID::default())
    }
}

impl From<Address> for ID {
    fn from(address: Address) -> Self {
        Self::new(address)
    }
}

impl From<ID> for Address {
    fn from(id: ID) -> Self {
        id.to_address()
    }
}

impl From<UID> for ID {
    fn from(uid: UID) -> Self {
        uid.id
    }
}

impl From<UID> for Address {
    fn from(uid: UID) -> Self {
        uid.to_address()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_creation() {
        let addr = Address::zero();
        let id = ID::new(addr);
        assert_eq!(id.to_address(), addr);
    }

    #[test]
    fn test_uid_creation() {
        let addr = Address::zero();
        let id = ID::new(addr);
        let uid = UID::new(id);
        assert_eq!(uid.to_address(), addr);
        assert_eq!(uid.id(), &id);
    }

    #[test]
    fn test_conversions() {
        let addr = Address::zero();
        let id: ID = addr.into();
        let uid = UID::new(id);
        
        let addr_from_id: Address = id.into();
        let addr_from_uid: Address = uid.into();
        let id_from_uid: ID = uid.into();
        
        assert_eq!(addr, addr_from_id);
        assert_eq!(addr, addr_from_uid);
        assert_eq!(id, id_from_uid);
    }
}
