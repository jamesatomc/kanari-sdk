use super::MoveVMState;
use anyhow::Result;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::StructTag;
use std::str::FromStr;

#[test]
fn delete_resource_removes_saved_value() -> Result<()> {
    let state = MoveVMState::new_in_memory()?;
    let owner = AccountAddress::from_hex_literal("0x1234")?;
    let tag = StructTag::from_str("0x2::coin::Coin<0x2::kanari::KANARI>")?;
    let bytes = vec![1u8, 2, 3, 4];

    state.save_resource(&owner, &tag, &bytes)?;
    assert_eq!(state.get_resource(&owner, &tag), Some(bytes));

    state.delete_resource(&owner, &tag)?;
    assert_eq!(state.get_resource(&owner, &tag), None);
    Ok(())
}
