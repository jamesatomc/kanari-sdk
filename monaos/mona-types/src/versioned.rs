// Versioned objects for upgradeable contracts
// Corresponds to `kanari_framework::versioned` module
use serde::{Deserialize, Serialize};
use crate::object::{UID, ID};
use std::collections::HashMap;

/// A wrapper type that supports versioning of inner types
/// The inner type is stored as a dynamic field keyed by version
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versioned {
    pub id: UID,
    pub version: u64,
    // In a real implementation, this would use dynamic fields
    // For now, we'll simulate with a HashMap
    pub data: HashMap<u64, Vec<u8>>, // version -> serialized data
}

/// Hot potato object for version changes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionChangeCap {
    pub versioned_id: ID,
    pub old_version: u64,
}

impl Versioned {    /// Create a new versioned object with initial version and value
    pub fn new(init_version: u64) -> Self {
        Self {
            id: UID::new(ID::new(crate::address::Address::zero())), // Placeholder
            version: init_version,
            data: HashMap::new(),
        }
    }

    /// Create versioned object with specific ID
    pub fn new_with_id(id: UID, init_version: u64) -> Self {
        Self {
            id,
            version: init_version,
            data: HashMap::new(),
        }
    }

    /// Add initial value of type T
    pub fn add_initial_value<T: serde::Serialize>(&mut self, value: T) -> Result<(), VersionedError> {
        let serialized = bcs::to_bytes(&value)
            .map_err(|_| VersionedError::SerializationFailed)?;
        self.data.insert(self.version, serialized);
        Ok(())
    }

    /// Get the current version
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Load value of the current version
    pub fn load_value<T: serde::de::DeserializeOwned>(&self) -> Result<T, VersionedError> {
        let data = self.data.get(&self.version)
            .ok_or(VersionedError::ValueNotFound)?;
        
        bcs::from_bytes(data)
            .map_err(|_| VersionedError::DeserializationFailed)
    }

    /// Check if a version exists
    pub fn has_version(&self, version: u64) -> bool {
        self.data.contains_key(&version)
    }

    /// Remove value for upgrade (returns capability)
    pub fn remove_value_for_upgrade(&mut self) -> Result<(Vec<u8>, VersionChangeCap), VersionedError> {
        let data = self.data.remove(&self.version)
            .ok_or(VersionedError::ValueNotFound)?;
        
        let cap = VersionChangeCap {
            versioned_id: self.id.to_id(),
            old_version: self.version,
        };

        Ok((data, cap))
    }

    /// Upgrade to new version with new value
    pub fn upgrade<T: serde::Serialize>(
        &mut self,
        new_version: u64,
        new_value: T,
        cap: VersionChangeCap,
    ) -> Result<(), VersionedError> {
        // Validate capability
        if cap.versioned_id != self.id.to_id() {
            return Err(VersionedError::InvalidUpgrade);
        }
        
        if cap.old_version >= new_version {
            return Err(VersionedError::InvalidUpgrade);
        }

        // Serialize new value
        let serialized = bcs::to_bytes(&new_value)
            .map_err(|_| VersionedError::SerializationFailed)?;

        // Update version and data
        self.version = new_version;
        self.data.insert(new_version, serialized);

        Ok(())
    }

    /// Destroy versioned object and return current value
    pub fn destroy<T: serde::de::DeserializeOwned>(self) -> Result<T, VersionedError> {
        let data = self.data.get(&self.version)
            .ok_or(VersionedError::ValueNotFound)?;
        
        bcs::from_bytes(data)
            .map_err(|_| VersionedError::DeserializationFailed)
    }

    /// List all available versions
    pub fn list_versions(&self) -> Vec<u64> {
        let mut versions: Vec<u64> = self.data.keys().cloned().collect();
        versions.sort();
        versions
    }

    /// Get the size of data for a specific version
    pub fn data_size(&self, version: u64) -> Option<usize> {
        self.data.get(&version).map(|data| data.len())
    }
}

/// Versioned object errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionedError {
    #[error("Invalid upgrade operation")]
    InvalidUpgrade,
    #[error("Value not found for version")]
    ValueNotFound,
    #[error("Serialization failed")]
    SerializationFailed,
    #[error("Deserialization failed")]
    DeserializationFailed,
    #[error("Version already exists")]
    VersionAlreadyExists,
}

pub type VersionedResult<T> = Result<T, VersionedError>;
