from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old in text:
        return text.replace(old, new, 1)
    if new in text:
        return text
    raise RuntimeError(f"missing operational visibility marker: {label}")


rpc = Path("crates/kanari-rpc-server/src/lib.rs")
rpc_text = rpc.read_text()
rpc_text = replace_once(
    rpc_text,
    '''        object_transaction::GET_PENDING_OBJECT_TRANSACTIONS => {
            object_transaction::pending(&state, &request).await
        }
''',
    '''        object_transaction::GET_PENDING_OBJECT_TRANSACTIONS => {
            object_transaction::pending(&state, &request).await
        }
        object_transaction::CANCEL_OBJECT_TRANSACTION => {
            object_transaction::cancel(&state, &request).await
        }
''',
    "object cancellation route",
)
rpc.write_text(rpc_text)

queries = Path("crates/kanari-core/src/engine/queries.rs")
query_text = queries.read_text()
query_text = replace_once(
    query_text,
    '''        let pending_transactions = self.pending_transaction_len();
        let total_transactions =
            self.committed_transaction_count_for_stats(chain.get_transaction_count());
''',
    '''        let object_pending = self.pending_object_transaction_len().unwrap_or_else(|error| {
            warn!("Failed to read object transaction pending count: {}", error);
            0
        });
        let pending_transactions = self
            .pending_transaction_len()
            .saturating_add(object_pending);
        let total_transactions =
            self.committed_transaction_count_for_stats(chain.get_transaction_count());
''',
    "object pending stats",
)
query_text = replace_once(
    query_text,
    '''                let mut payload_hashes = std::collections::HashSet::new();
                let mut indexed_hashes = std::collections::HashSet::new();

                for (key, _) in &entries {
                    if let Some(hash) = key.strip_prefix(b"tx_payload/") {
                        payload_hashes.insert(hash.to_vec());
                    } else if let Some(hash) = key.strip_prefix(b"tx_index/") {
                        indexed_hashes.insert(hash.to_vec());
                    }
                }

                payload_hashes
                    .intersection(&indexed_hashes)
                    .count()
                    .max(fallback_count)
''',
    '''                let mut payload_hashes = std::collections::HashSet::new();
                let mut indexed_hashes = std::collections::HashSet::new();
                let mut executed_object_hashes = std::collections::HashSet::new();

                for (key, _) in &entries {
                    if let Some(hash) = key.strip_prefix(b"tx_payload/") {
                        payload_hashes.insert(hash.to_vec());
                    } else if let Some(hash) = key.strip_prefix(b"tx_index/") {
                        indexed_hashes.insert(hash.to_vec());
                    } else if let Some(hash) = key.strip_prefix(b"object_tx:executed:") {
                        executed_object_hashes.insert(hash.to_vec());
                    }
                }

                payload_hashes
                    .intersection(&indexed_hashes)
                    .count()
                    .saturating_add(executed_object_hashes.len())
                    .max(fallback_count)
''',
    "object committed stats",
)
queries.write_text(query_text)
