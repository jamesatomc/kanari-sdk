from pathlib import Path

engine = Path("crates/kanari-core/src/object_checkpoint_engine.rs")
text = engine.read_text()
method = r'''
    pub fn object_checkpoint(
        &self,
        sequence: u64,
    ) -> Result<Option<ObjectCheckpointRecord>> {
        self.state_read().store.load(&checkpoint_key(sequence))
    }
'''
marker = "    pub fn object_checkpoint_for_transaction(\n"
if "pub fn object_checkpoint(" not in text:
    if marker not in text:
        raise RuntimeError("object checkpoint transaction lookup marker not found")
    text = text.replace(marker, method + "\n" + marker, 1)
engine.write_text(text)

api = Path("crates/kanari-rpc-api/src/lib.rs")
text = api.read_text()
if "use kanari_types::signed_object_transaction::SignedObjectTransaction;" not in text:
    text = text.replace(
        "use kanari_types::event::Event;\n",
        "use kanari_types::event::Event;\nuse kanari_types::signed_object_transaction::SignedObjectTransaction;\n",
        1,
    )
struct_start = text.index("pub struct FullBlockData {")
struct_end = text.index("\n}", struct_start)
struct_text = text[struct_start:struct_end]
if "object_transactions" not in struct_text:
    tx_line = "    pub transactions: Vec<SignedTransaction>,\n"
    if tx_line not in struct_text:
        raise RuntimeError("FullBlockData transaction field not found")
    struct_text = struct_text.replace(
        tx_line,
        tx_line + "    #[serde(default)]\n    pub object_transactions: Vec<SignedObjectTransaction>,\n",
        1,
    )
    text = text[:struct_start] + struct_text + text[struct_end:]
api.write_text(text)

queries = Path("crates/kanari-core/src/engine/queries.rs")
text = queries.read_text()
start = text.index("    pub fn get_account_info(&self, address: &str) -> Option<AccountInfo> {")
end = text.index("\n    pub fn get_module_bytecode", start)
account = r'''    pub fn get_account_info(&self, address: &str) -> Option<AccountInfo> {
        let owner = KanariAddress::parse_to_account_address(address).ok()?;
        let state = self.state_read();
        let owned_objects = self.resolve_account_objects(&state, &owner);
        let token_balances = state
            .object_balances(&owner)
            .ok()?
            .into_iter()
            .map(|balance| (balance.token_type, balance.balance))
            .collect();
        Some(AccountInfo {
            address: owner.to_hex_literal(),
            sequence_number: 0,
            modules: Vec::new(),
            token_balances,
            owned_objects: Some(owned_objects),
        })
    }
'''
text = text[:start] + account + text[end:]

start = text.index("    pub fn get_block(&self, height: u64) -> Option<BlockData> {")
end = text.index("\n    pub fn get_checkpoint_sync", start)
blocks = r'''    pub fn get_block(&self, height: u64) -> Option<BlockData> {
        self.object_checkpoint(height).ok().flatten().map(|checkpoint| BlockData {
            height: checkpoint.sequence,
            timestamp: checkpoint.timestamp_ms,
            hash: hex::encode(checkpoint.digest),
            prev_hash: hex::encode(checkpoint.previous_digest),
            state_root: hex::encode(checkpoint.state_root),
            tx_count: 1,
            events: Vec::new(),
        })
    }

    pub fn get_full_block(&self, height: u64) -> Option<FullBlockData> {
        self.object_checkpoint(height)
            .ok()
            .flatten()
            .map(|checkpoint| FullBlockData {
                height: checkpoint.sequence,
                timestamp: checkpoint.timestamp_ms,
                hash: hex::encode(checkpoint.digest),
                prev_hash: hex::encode(checkpoint.previous_digest),
                state_root: hex::encode(checkpoint.state_root),
                tx_count: 1,
                events: Vec::new(),
                transactions: Vec::new(),
                object_transactions: vec![checkpoint.transaction],
                vertices: Vec::new(),
            })
    }
'''
text = text[:start] + blocks + text[end:]
queries.write_text(text)
