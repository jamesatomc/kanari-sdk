from __future__ import annotations

import re
from .common import read, write


def apply() -> None:
    path = "move-execution/v1/kanari-move-runtime-v1/src/state.rs"
    text = read(path)
    text = text.replace(
        "let use_incremental_smt = if self.store.get_db().is_some() {\n                true",
        "let use_incremental_smt = if self.store.get_db().is_some() {\n                !self.smt_dirty",
        1,
    )

    pattern = re.compile(
        r"    /// Commit pending overlay changes to the persistent store and update SMT\n"
        r"    pub fn commit\(&mut self\) -> Result<\(\)> \{.*?\n"
        r"    \}\n\n"
        r"    // Helper to write to overlay",
        re.S,
    )
    replacement = '''    /// Commit canonical state and checkpoint metadata in one backend batch.
    pub fn commit_with_extra_raw_changes(
        &mut self,
        extra_updates: &[(Vec<u8>, Vec<u8>)],
        extra_deletes: &[Vec<u8>],
    ) -> Result<()> {
        let mut updates = Vec::with_capacity(self.overlay.len() + extra_updates.len());
        let mut deletes = Vec::with_capacity(self.overlay.len() + extra_deletes.len());
        for (key, value) in &self.overlay {
            match value {
                Some(value) => updates.push((key.clone(), value.clone())),
                None => deletes.push(key.clone()),
            }
        }
        updates.extend_from_slice(extra_updates);
        deletes.extend_from_slice(extra_deletes);

        let (smt_updates, smt_deletes) = self.smt_changes_from_pending_delta();
        self.store.apply_raw_changes(&updates, &deletes)?;

        if !self.smt_dirty {
            if let Some(smt) = &self.smt {
                let is_in_memory = self.store.get_db().is_none();
                let change_count = smt_updates.len().saturating_add(smt_deletes.len());
                if is_in_memory && change_count > IN_MEMORY_SMT_INCREMENTAL_THRESHOLD {
                    self.smt_dirty = true;
                } else if let Err(error) = (|| -> Result<()> {
                    if !smt_updates.is_empty() {
                        smt.insert(&smt_updates)?;
                    }
                    if !smt_deletes.is_empty() {
                        smt.delete(&smt_deletes)?;
                    }
                    Ok(())
                })() {
                    log::error!("SMT cache update failed after canonical batch commit: {}", error);
                    self.smt_dirty = true;
                }
            }
        }

        self.persisted_canonical_root_entries = self.canonical_root_entries.clone();
        self.pending_smt_changes.clear();
        self.overlay.clear();
        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        self.commit_with_extra_raw_changes(&[], &[])
    }

    // Helper to write to overlay'''
    text, count = pattern.subn(replacement, text, count=1)
    if count != 1:
        raise RuntimeError("StateManager commit method was not found")

    startup_marker = '''        state
            .ensure_smt_initialized()
            .context("Failed to initialize state SMT")?;
'''
    startup_replacement = startup_marker + '''
        if let Some(tree) = &state.smt {
            let expected = smt::compute_sparse_root(
                &state
                    .canonical_root_entries
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            if tree.root_hash().map(|root| root.to_vec()).unwrap_or_default() != expected {
                log::warn!("Persisted SMT cache differs from canonical state; using materialized roots");
                state.smt_dirty = true;
            }
        }
'''
    if startup_marker not in text:
        raise RuntimeError("SMT startup marker was not found")
    text = text.replace(startup_marker, startup_replacement, 1)
    write(path, text)
