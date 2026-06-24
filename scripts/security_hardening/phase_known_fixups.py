from __future__ import annotations

from .common import read, write


def apply() -> None:
    store_path = "move-execution/v1/kanari-move-runtime-v1/src/storage/persistent_store.rs"
    text = read(store_path)
    marker = '''    pub fn load<T: DeserializeOwned>(
        &self,
        key: &[u8],
    ) -> std::result::Result<Option<T>, PersistentStoreError> {
        match self.read_raw(key)? {
            Some(bytes) => Ok(Some(bcs::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }
'''
    addition = '''
    /// Check key existence without deserializing or scanning the database.
    pub fn contains_key(
        &self,
        key: &[u8],
    ) -> std::result::Result<bool, PersistentStoreError> {
        Ok(self.read_raw(key)?.is_some())
    }
'''
    if marker not in text:
        raise RuntimeError("PersistentStore::load insertion point not found")
    write(store_path, text.replace(marker, marker + addition, 1))

    mempool_path = "crates/kanari-core/src/engine/mempool.rs"
    text = read(mempool_path)
    old = '''        store
            .logical_entries()
            .ok()
            .is_some_and(|entries| entries.iter().any(|(entry_key, _)| entry_key == &key))'''
    new = '''        store.contains_key(&key).unwrap_or(false)'''
    if old not in text:
        raise RuntimeError("mempool replay scan not found")
    write(mempool_path, text.replace(old, new, 1))

    checkpoint_path = "crates/kanari-core/src/engine/apply_checkpoint.rs"
    text = read(checkpoint_path)
    if old not in text:
        raise RuntimeError("checkpoint replay scan not found")
    write(checkpoint_path, text.replace(old, new, 1))

    consensus_path = "crates/kanari-core/src/consensus.rs"
    text = read(consensus_path)
    old_digest = '''    pub fn certificate_signing_digest(&self) -> Result<[u8; 32]> {
        let checkpoint_hash = self.hash()?;
        let bytes = bcs::to_bytes(&(
            b"kanari:checkpoint-certificate:v1".as_slice(),
            checkpoint_hash,
        ))?;
        Ok(vertex_id_from_hash_bytes(&hash_data_blake3(&bytes)))
    }'''
    new_digest = '''    pub fn certificate_signing_digest(
        &self,
        epoch: u64,
        round: u64,
        committee_digest: &[u8],
    ) -> Result<[u8; 32]> {
        let checkpoint_hash = self.hash()?;
        let bytes = bcs::to_bytes(&(
            b"kanari:checkpoint-certificate:v2".as_slice(),
            checkpoint_hash,
            epoch,
            round,
            committee_digest,
        ))?;
        Ok(vertex_id_from_hash_bytes(&hash_data_blake3(&bytes)))
    }'''
    if old_digest not in text:
        raise RuntimeError("checkpoint certificate digest block not found")
    text = text.replace(old_digest, new_digest, 1)

    old_attach = '''        use ed25519_dalek::Signer;
        let digest = self.certificate_signing_digest()?;
        self.certificate = Some(CheckpointCertificate {
            epoch,
            round,
            committee_digest: Self::committee_digest(public_keys)?,'''
    new_attach = '''        use ed25519_dalek::Signer;
        let committee_digest = Self::committee_digest(public_keys)?;
        let digest = self.certificate_signing_digest(epoch, round, &committee_digest)?;
        self.certificate = Some(CheckpointCertificate {
            epoch,
            round,
            committee_digest,'''
    if old_attach not in text:
        raise RuntimeError("checkpoint certificate attach block not found")
    text = text.replace(old_attach, new_attach, 1)

    old_verify = '''        anyhow::ensure!(
            certificate.committee_digest == Self::committee_digest(public_keys)?,
            "checkpoint committee digest mismatch"
        );
        let quorum = authority_count.saturating_mul(2) / 3 + 1;
        let digest = self.certificate_signing_digest()?;'''
    new_verify = '''        let expected_committee_digest = Self::committee_digest(public_keys)?;
        anyhow::ensure!(
            certificate.committee_digest == expected_committee_digest,
            "checkpoint committee digest mismatch"
        );
        let quorum = authority_count.saturating_mul(2) / 3 + 1;
        let digest = self.certificate_signing_digest(
            certificate.epoch,
            certificate.round,
            &certificate.committee_digest,
        )?;'''
    if old_verify not in text:
        raise RuntimeError("checkpoint certificate verify block not found")
    write(consensus_path, text.replace(old_verify, new_verify, 1))

    engine_path = "crates/kanari-core/src/engine.rs"
    text = read(engine_path)
    old_budget = "Some((tx.gas_limit(), tx.gas_price()))"
    new_budget = "Some((tx.gas_limit().saturating_sub(base_gas_used), tx.gas_price()))"
    if text.count(old_budget) != 2:
        raise RuntimeError("expected two runtime gas budget call sites")
    write(engine_path, text.replace(old_budget, new_budget))
