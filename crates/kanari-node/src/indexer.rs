// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Blockchain indexer integration for kanari-node
//!
//! This module integrates the kanari-indexer with the node to automatically
//! index blocks as they are committed to the blockchain.

use anyhow::{Context, Result};
use kanari_indexer::Indexer;
use kanari_types::error::KanariError;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::info;

/// Manages the blockchain indexer (wrapper with utility methods)
pub struct NodeIndexer {
    indexer: Arc<Mutex<Indexer>>,
}

impl NodeIndexer {
    /// Create a new node indexer
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        // Ensure data directory exists
        std::fs::create_dir_all(&data_dir)
            .context(format!("Failed to create data directory: {:?}", data_dir))?;

        let db_path = data_dir.join("indexer.db");

        let config = kanari_indexer::IndexerConfig {
            db_path: db_path.clone(),
            in_memory: false,
            batch_size: 100,
        };

        let indexer = Indexer::new(config).context("Failed to initialize blockchain indexer")?;

        let indexer_arc = Arc::new(Mutex::new(indexer));

        info!("Node indexer initialized at {:?}", db_path);

        Ok(Self {
            indexer: indexer_arc,
        })
    }

    fn lock_indexer(&self) -> Result<std::sync::MutexGuard<'_, Indexer>> {
        self.indexer.lock().map_err(|e| {
            KanariError::OperationFailed {
                context: "Failed to acquire indexer lock",
                details: e.to_string(),
            }
            .into()
        })
    }

    /// Get reference to the underlying indexer
    pub fn indexer(&self) -> &Arc<Mutex<Indexer>> {
        &self.indexer
    }

    /// Index a block
    pub fn index_block(&self, block: &kanari_types::block::Block) -> Result<()> {
        let height = block.header.height;
        let idx = self.lock_indexer()?;

        idx.index_block(block)
            .with_context(|| format!("Failed to index block #{}", height))?;

        if height.is_multiple_of(100) {
            info!("[INDEXER] Indexed block #{}", height);
        }

        Ok(())
    }

    /// Get indexer statistics
    pub fn get_stats(&self) -> Result<String> {
        let idx = self.lock_indexer()?;

        let stats = idx
            .get_statistics()
            .context("Failed to get indexer statistics")?;
        Ok(format!("{}", stats))
    }
}
