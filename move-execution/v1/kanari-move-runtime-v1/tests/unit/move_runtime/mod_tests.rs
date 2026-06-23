use super::*;
use serde::Deserialize;
#[allow(clippy::duplicate_mod)]
#[path = "../test_support.rs"]
mod test_support;

use test_support::test_addr;

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct SerializedTxContext {
    sender: AccountAddress,
    tx_hash: Vec<u8>,
    ids_created: u64,
    epoch_timestamp_ms: u64,
    sponsor: u64,
}

#[test]
fn tx_context_without_timestamp_is_deterministic() -> Result<()> {
    let runtime = MoveRuntime::new_with_natives_in_memory(vec![])?;
    let sender = test_addr("0x1111")?;

    let first = runtime.build_tx_context_bytes(Some(sender), None, None)?;
    let second = runtime.build_tx_context_bytes(Some(sender), None, None)?;
    let ctx: SerializedTxContext = bcs::from_bytes(&first)?;

    assert_eq!(first, second);
    assert_eq!(ctx.sender, sender);
    assert_eq!(ctx.epoch_timestamp_ms, 0);
    assert_eq!(ctx.ids_created, 0);
    assert_eq!(ctx.sponsor, 0);

    Ok(())
}

#[test]
fn tx_context_uses_canonical_timestamp_and_hash() -> Result<()> {
    let runtime = MoveRuntime::new_with_natives_in_memory(vec![])?;
    let sender = test_addr("0x1111")?;
    let tx_hash = vec![7u8; 32];

    let bytes = runtime.build_tx_context_bytes(Some(sender), Some(42), Some(&tx_hash))?;
    let ctx: SerializedTxContext = bcs::from_bytes(&bytes)?;

    assert_eq!(ctx.sender, sender);
    assert_eq!(ctx.tx_hash, tx_hash);
    assert_eq!(ctx.epoch_timestamp_ms, 42);

    Ok(())
}
