// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_types::transaction::SignedTransaction;

/// Transaction scheduler used by deterministic checkpoint execution.
///
/// Security note: transaction arguments and native effects are not yet represented by
/// a complete deterministic read/write set. Until that exists, every transaction is
/// placed in its own wave. This preserves canonical transaction order and prevents two
/// transactions with an unobserved conflict from executing against the same snapshot.
pub struct TransactionScheduler;

impl TransactionScheduler {
    /// Produce one transaction per wave in the original order.
    ///
    /// Parallel scheduling may be reintroduced only after conflict discovery covers
    /// all Move resources, object inputs, native operations, module publication, and
    /// dynamic-field accesses, with deterministic post-execution revalidation.
    pub fn schedule(transactions: Vec<SignedTransaction>) -> Vec<Vec<SignedTransaction>> {
        transactions.into_iter().map(|tx| vec![tx]).collect()
    }
}

#[cfg(test)]
#[path = "../tests/unit/scheduler_tests.rs"]
mod tests;
