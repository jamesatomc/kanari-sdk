// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl BlockchainEngine {
    /// The account/sequence mempool was removed by the object-centric cutover.
    /// Submit `SignedObjectTransaction` through `submit_protocol_transaction`.
    pub fn submit_transactions_batch(
        &self,
        _signed_txs: Vec<SignedTransaction>,
    ) -> Result<Vec<Vec<u8>>> {
        anyhow::bail!("Legacy account transactions are disabled; use submit_protocol_transaction")
    }

    /// Immediate account execution is intentionally unavailable after cutover.
    pub fn execute_transaction_immediate(
        &self,
        _signed_tx: SignedTransaction,
    ) -> Result<(Vec<u8>, ChangeSet)> {
        anyhow::bail!("Legacy account execution is disabled; use execute_object_command_now")
    }

    pub(crate) fn pending_tx_count_for_sender(&self, _sender: &str) -> u64 {
        0
    }

    pub(crate) fn remove_pending_sender_counts(
        counts: &mut ahash::AHashMap<String, u64>,
        _transactions: &[SignedTransaction],
    ) {
        counts.clear();
    }

    pub(crate) fn normalize_addr(addr: &str) -> String {
        use std::str::FromStr;
        KanariAddress::from_str(addr)
            .map(|address| address.to_hex())
            .unwrap_or_else(|_| addr.trim_start_matches("0x").to_lowercase())
    }
}
