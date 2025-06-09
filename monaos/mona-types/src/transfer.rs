use serde::{Deserialize, Serialize};
use crate::address::Address;
use crate::object::ID;

/// Represents the ability to receive an object of type T.
/// Corresponds to `kanari_framework::transfer::Receiving<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receiving<T> {
    pub id: ID,
    pub version: u64,
    _phantom: std::marker::PhantomData<T>,
}

/// Transfer operations and types
pub struct Transfer;

impl Transfer {
    /// Transfer ownership of an object to a recipient
    pub fn transfer<T: TransferableObject>(obj: T, _recipient: Address) -> TransferResult<()> {
        // In a real implementation, this would interact with the VM/storage layer
        // For now, we just validate the operation
        if !obj.is_transferable() {
            return Err(TransferError::NotTransferable);
        }
        
        // Implementation would handle the actual transfer
        Ok(())
    }

    /// Transfer an object with store capability outside of its module
    pub fn public_transfer<T: PublicTransferableObject>(obj: T, recipient: Address) -> TransferResult<()> {
        if !obj.has_store() {
            return Err(TransferError::NoStoreAbility);
        }
        
        Self::transfer(obj, recipient)
    }

    /// Freeze an object, making it immutable
    pub fn freeze_object<T: TransferableObject>(obj: T) -> TransferResult<()> {
        if obj.is_shared() {
            return Err(TransferError::SharedObjectOperationNotSupported);
        }
        
        // Implementation would mark the object as frozen
        Ok(())
    }

    /// Freeze an object with store capability outside of its module
    pub fn public_freeze_object<T: PublicTransferableObject>(obj: T) -> TransferResult<()> {
        if !obj.has_store() {
            return Err(TransferError::NoStoreAbility);
        }
        
        Self::freeze_object(obj)
    }

    /// Turn an object into a mutable shared object
    pub fn share_object<T: TransferableObject>(obj: T) -> TransferResult<()> {
        if !obj.is_newly_created() {
            return Err(TransferError::SharedNonNewObject);
        }
        
        // Implementation would mark the object as shared
        Ok(())
    }

    /// Share an object with store capability outside of its module
    pub fn public_share_object<T: PublicTransferableObject>(obj: T) -> TransferResult<()> {
        if !obj.has_store() {
            return Err(TransferError::NoStoreAbility);
        }
        
        Self::share_object(obj)
    }    /// Receive an owned object using a Receiving argument
    pub fn receive<T: TransferableObject, Parent>(
        _parent: &mut Parent,
        _to_receive: Receiving<T>,
    ) -> TransferResult<T> {
        // Implementation would verify ownership and retrieve the object
        Err(TransferError::UnableToReceiveObject)
    }

    /// Receive an object with store capability
    pub fn public_receive<T: PublicTransferableObject, Parent>(
        parent: &mut Parent,
        to_receive: Receiving<T>,
    ) -> TransferResult<T> {
        Self::receive(parent, to_receive)
    }
}

/// Trait for objects that can be transferred
pub trait TransferableObject {
    /// Check if the object is transferable
    fn is_transferable(&self) -> bool { true }
    
    /// Check if the object is shared
    fn is_shared(&self) -> bool { false }
    
    /// Check if the object was newly created in this transaction
    fn is_newly_created(&self) -> bool { false }
    
    /// Get the object ID
    fn object_id(&self) -> ID;
}

/// Trait for objects that can be transferred publicly (have store ability)
pub trait PublicTransferableObject: TransferableObject {
    /// Check if the object has the store ability
    fn has_store(&self) -> bool { true }
}

impl<T> Receiving<T> {
    /// Create a new Receiving capability
    pub fn new(id: ID, version: u64) -> Self {
        Self {
            id,
            version,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Get the object ID
    pub fn id(&self) -> ID {
        self.id
    }

    /// Get the version
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// Transfer operation results
pub type TransferResult<T> = Result<T, TransferError>;

/// Transfer operation errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("Shared object was not created in the current transaction")]
    SharedNonNewObject,
    #[error("BCS serialization failed")]
    BCSSerializationFailure,
    #[error("Receiving object type mismatch")]
    ReceivingObjectTypeMismatch,
    #[error("Unable to receive object")]
    UnableToReceiveObject,
    #[error("Shared object operations not supported")]
    SharedObjectOperationNotSupported,
    #[error("Object is not transferable")]
    NotTransferable,
    #[error("Object does not have store ability")]
    NoStoreAbility,
}

/// Transfer error constants matching Move constants
pub mod error_constants {
    /// Shared an object that was previously created
    pub const E_SHARED_NON_NEW_OBJECT: u64 = 0;
    /// Serialization of the object failed
    pub const E_BCS_SERIALIZATION_FAILURE: u64 = 1;
    /// The object being received is not of the expected type
    pub const E_RECEIVING_OBJECT_TYPE_MISMATCH: u64 = 2;
    /// Object does not exist or is not accessible through the parent
    pub const E_UNABLE_TO_RECEIVE_OBJECT: u64 = 3;
    /// Shared object operations such as wrapping, freezing, and converting to owned are not allowed
    pub const E_SHARED_OBJECT_OPERATION_NOT_SUPPORTED: u64 = 4;
}

/// Transfer event types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectTransferEvent {
    pub object_id: ID,
    pub from: Address,
    pub to: Address,
    pub object_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectFreezeEvent {
    pub object_id: ID,
    pub object_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectShareEvent {
    pub object_id: ID,
    pub object_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectReceiveEvent {
    pub object_id: ID,
    pub parent_id: ID,
    pub object_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::UID;

    #[derive(Debug, Clone)]
    struct TestObject {
        id: UID,
        transferable: bool,
        shared: bool,
        newly_created: bool,
    }

    impl TransferableObject for TestObject {
        fn is_transferable(&self) -> bool {
            self.transferable
        }

        fn is_shared(&self) -> bool {
            self.shared
        }

        fn is_newly_created(&self) -> bool {
            self.newly_created
        }

        fn object_id(&self) -> ID {
            self.id.to_id()
        }
    }

    impl PublicTransferableObject for TestObject {}

    #[test]
    fn test_transfer_operations() {
        let obj = TestObject {
            id: UID::default(),
            transferable: true,
            shared: false,
            newly_created: false,
        };

        let recipient = Address::zero();
        let result = Transfer::transfer(obj, recipient);
        assert!(result.is_ok());
    }

    #[test]
    fn test_non_transferable_object() {
        let obj = TestObject {
            id: UID::default(),
            transferable: false,
            shared: false,
            newly_created: false,
        };

        let recipient = Address::zero();
        let result = Transfer::transfer(obj, recipient);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), TransferError::NotTransferable);
    }

    #[test]
    fn test_share_non_new_object() {
        let obj = TestObject {
            id: UID::default(),
            transferable: true,
            shared: false,
            newly_created: false,
        };

        let result = Transfer::share_object(obj);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), TransferError::SharedNonNewObject);
    }

    #[test]
    fn test_receiving_creation() {
        let id = ID::default();
        let version = 1;
        let receiving: Receiving<TestObject> = Receiving::new(id, version);
        
        assert_eq!(receiving.id(), id);
        assert_eq!(receiving.version(), version);
    }
}
