// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::address::Address;
use anyhow::{Context, Result, ensure};
use kanari_crypto::hash_data_blake3;
use move_core_types::account_address::AccountAddress;
use move_core_types::{identifier::Identifier, language_storage::ModuleId};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Canonical object identifier used by the object-centric transaction model.
///
/// An address and an object ID share the same 32-byte representation, but they
/// have different protocol semantics. Keeping a distinct type prevents code
/// from accidentally treating an object dependency as an account resource.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectID(AccountAddress);

impl ObjectID {
    pub const ZERO: Self = Self(AccountAddress::ZERO);

    pub fn new(address: AccountAddress) -> Self {
        Self(address)
    }

    pub fn from_hex_literal(value: &str) -> Result<Self> {
        Ok(Self(AccountAddress::from_hex_literal(value)?))
    }

    pub fn address(self) -> AccountAddress {
        self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }

    pub fn to_hex_literal(self) -> String {
        self.0.to_hex_literal()
    }
}

impl From<AccountAddress> for ObjectID {
    fn from(value: AccountAddress) -> Self {
        Self::new(value)
    }
}

impl From<ObjectID> for AccountAddress {
    fn from(value: ObjectID) -> Self {
        value.address()
    }
}

impl fmt::Display for ObjectID {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.to_hex_literal())
    }
}

/// Digest of the canonical object representation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectDigest(pub [u8; 32]);

impl ObjectDigest {
    pub const ZERO: Self = Self([0; 32]);

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == 32,
            "Object digest must contain exactly 32 bytes"
        );
        let mut digest = [0u8; 32];
        digest.copy_from_slice(bytes);
        Ok(Self(digest))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for ObjectDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

pub type ObjectVersion = u64;

/// An immutable reference to one exact object state.
///
/// Transactions consume `(id, version, digest)`, not only an object ID. This is
/// the object-centric replacement for account sequence numbers and prevents a
/// signed transaction from silently reading a newer object version.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub object_id: ObjectID,
    pub version: ObjectVersion,
    pub digest: ObjectDigest,
}

impl ObjectRef {
    pub fn new(object_id: ObjectID, version: ObjectVersion, digest: ObjectDigest) -> Self {
        Self {
            object_id,
            version,
            digest,
        }
    }
}

/// Protocol ownership of an object.
///
/// `AddressOwner` and `Immutable` objects can use the owned-object fast path.
/// `Shared` objects require consensus ordering. `ObjectOwner` represents a
/// child object whose authority is derived from its parent object.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    AddressOwner(AccountAddress),
    ObjectOwner(ObjectID),
    Shared {
        initial_shared_version: ObjectVersion,
    },
    Immutable,
}

impl Owner {
    pub fn address_owner(&self) -> Option<AccountAddress> {
        match self {
            Self::AddressOwner(address) => Some(*address),
            _ => None,
        }
    }

    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Shared { .. })
    }

    pub fn is_immutable(&self) -> bool {
        matches!(self, Self::Immutable)
    }

    pub fn requires_consensus(&self) -> bool {
        self.is_shared()
    }

    pub fn sender_can_mutate(&self, sender: AccountAddress) -> bool {
        matches!(self, Self::AddressOwner(owner) if *owner == sender)
    }
}

/// Canonical object metadata committed by validators.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub id: ObjectID,
    pub version: ObjectVersion,
    pub digest: ObjectDigest,
    pub owner: Owner,
    pub previous_transaction: Option<[u8; 32]>,
}

impl ObjectMetadata {
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(self.id, self.version, self.digest)
    }
}

/// Compute a deterministic digest over every consensus-relevant object field.
pub fn compute_object_digest(
    id: ObjectID,
    version: ObjectVersion,
    owner: &Owner,
    type_name: &str,
    contents: &[u8],
    previous_transaction: Option<[u8; 32]>,
) -> Result<ObjectDigest> {
    let canonical = bcs::to_bytes(&(
        id,
        version,
        owner,
        type_name,
        contents,
        previous_transaction,
    ))?;
    ObjectDigest::from_bytes(&hash_data_blake3(&canonical))
}

/// UID wrapper used by Move `object::UID` (contains an address)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UIDRecord {
    pub addr: AccountAddress,
}

impl UIDRecord {
    /// Create a new UIDRecord from an AccountAddress
    pub fn new(addr: AccountAddress) -> Self {
        Self { addr }
    }

    /// Return the underlying address
    pub fn address(&self) -> AccountAddress {
        self.addr
    }

    /// Convenience: construct from hex literal string like "0x1"
    pub fn from_hex_literal(hex: &str) -> Result<Self> {
        let addr = AccountAddress::from_hex_literal(hex).context("invalid address")?;
        Ok(Self::new(addr))
    }
}

/// ID wrapper used by Move `object::ID` (contains an address)
/// Added to support DEX/DeFi features where copyable IDs are needed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IDRecord {
    pub bytes: AccountAddress,
}

impl IDRecord {
    /// Create a new IDRecord from an AccountAddress
    pub fn new(bytes: AccountAddress) -> Self {
        Self { bytes }
    }

    /// Return the underlying address
    pub fn address(&self) -> AccountAddress {
        self.bytes
    }

    /// Convenience: construct from hex literal string like "0x1"
    pub fn from_hex_literal(hex: &str) -> Result<Self> {
        let bytes = AccountAddress::from_hex_literal(hex).context("invalid address")?;
        Ok(Self::new(bytes))
    }
}

/// Object module constants and utilities
pub struct ObjectModule;

impl ObjectModule {
    pub const OBJECT_MODULE: &'static str = "object";

    /// Name of the UID struct in Move
    pub const UID_STRUCT: &'static str = "UID";

    /// Name of the ID struct in Move
    pub const ID_STRUCT: &'static str = "ID";

    /// Get the module ID for kanari_system::object
    pub fn get_module_id() -> Result<ModuleId> {
        let address = AccountAddress::from_hex_literal(Address::KANARI_SYSTEM_ADDRESS)
            .context("Invalid system address")?;

        let module_name =
            Identifier::new(Self::OBJECT_MODULE).context("Invalid object module name")?;

        Ok(ModuleId::new(address, module_name))
    }

    /// Get function names used in object module
    pub fn function_names() -> ObjectFunctions {
        ObjectFunctions {
            new: "new",
            uid_to_inner: "uid_to_inner",
            id_from_address: "id_from_address",
            id_to_address: "id_to_address",
            id_to_bytes: "id_to_bytes",
            uid_address: "uid_address",
            uid_to_u64: "uid_to_u64",
            uid_to_bytes: "uid_to_bytes",
            id_bytes: "id_bytes",
            save_object: "save_object",
            delete: "delete",
        }
    }
}

/// Object module function names
pub struct ObjectFunctions {
    pub new: &'static str,
    pub uid_to_inner: &'static str,
    pub id_from_address: &'static str,
    pub id_to_address: &'static str,
    pub id_to_bytes: &'static str,
    pub uid_address: &'static str,
    pub uid_to_u64: &'static str,
    pub uid_to_bytes: &'static str,
    pub id_bytes: &'static str,
    pub save_object: &'static str,
    pub delete: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uid_record_from_hex() {
        use crate::address::Address as KanariAddress;

        let uid = UIDRecord::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();
        let expected = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();
        assert_eq!(uid.addr, expected);
    }

    #[test]
    fn test_id_record_from_hex() {
        use crate::address::Address as KanariAddress;

        let id = IDRecord::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();
        let expected = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();
        assert_eq!(id.bytes, expected);
    }

    #[test]
    fn object_digest_changes_with_version() {
        let id = ObjectID::from_hex_literal("0x42").unwrap();
        let owner = Owner::AddressOwner(AccountAddress::from_hex_literal("0x7").unwrap());
        let first = compute_object_digest(
            id,
            1,
            &owner,
            "0x2::coin::Coin<0x2::kanari::KANARI>",
            &[1, 2, 3],
            None,
        )
        .unwrap();
        let second = compute_object_digest(
            id,
            2,
            &owner,
            "0x2::coin::Coin<0x2::kanari::KANARI>",
            &[1, 2, 3],
            None,
        )
        .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn shared_owner_requires_consensus() {
        let owner = Owner::Shared {
            initial_shared_version: 1,
        };
        assert!(owner.requires_consensus());
        assert!(!owner.sender_can_mutate(AccountAddress::ZERO));
    }
}
