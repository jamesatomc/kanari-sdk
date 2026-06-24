from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "crates/kanari-core/src/consensus.rs"
    text = read(path)
    text = text.replace("use std::sync::Arc;", "use std::collections::{BTreeMap, HashSet};\nuse std::sync::Arc;", 1)
    marker = "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct Checkpoint {"
    certificate_types = '''#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointAuthoritySignature {
    pub authority: AuthorityId,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointCertificate {
    pub epoch: u64,
    pub round: u64,
    pub committee_digest: Vec<u8>,
    pub signatures: Vec<CheckpointAuthoritySignature>,
}

'''
    if marker not in text:
        raise RuntimeError("checkpoint marker not found")
    text = text.replace(marker, certificate_types + marker, 1)
    text = text.replace(
        '''    pub timestamp: u64,
    pub prev_checkpoint_hash: Vec<u8>,
}''',
        '''    pub timestamp: u64,
    pub prev_checkpoint_hash: Vec<u8>,
    #[serde(default)]
    pub gas_schedule_hash: Vec<u8>,
    #[serde(default)]
    pub certificate: Option<CheckpointCertificate>,
}''',
        1,
    )
    text = text.replace(
        '''            timestamp,
            prev_checkpoint_hash,
        }''',
        '''            timestamp,
            prev_checkpoint_hash,
            gas_schedule_hash: kanari_types::GasConfig::default().consensus_hash().to_vec(),
            certificate: None,
        }''',
        1,
    )
    text = text.replace(
        '''        let serialized = bcs::to_bytes(&(
            self.sequence,
            &tx_hashes,
            &self.state_root,
            &self.prev_checkpoint_hash,
        ))?;''',
        '''        let serialized = bcs::to_bytes(&(
            b"kanari:checkpoint:v2".as_slice(),
            self.sequence,
            &self.vertices,
            &tx_hashes,
            &self.state_root,
            self.timestamp,
            &self.prev_checkpoint_hash,
            &self.gas_schedule_hash,
        ))?;''',
        1,
    )
    text = text.replace(
        '''            timestamp: 0,
            prev_checkpoint_hash: vec![0u8; 32],
        }''',
        '''            timestamp: 0,
            prev_checkpoint_hash: vec![0u8; 32],
            gas_schedule_hash: kanari_types::GasConfig::default().consensus_hash().to_vec(),
            certificate: None,
        }''',
        1,
    )
    methods = '''
    pub fn certificate_signing_digest(&self) -> Result<[u8; 32]> {
        let checkpoint_hash = self.hash()?;
        let bytes = bcs::to_bytes(&(
            b"kanari:checkpoint-certificate:v1".as_slice(),
            checkpoint_hash,
        ))?;
        Ok(vertex_id_from_hash_bytes(&hash_data_blake3(&bytes)))
    }

    pub fn committee_digest(public_keys: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
        let bytes = bcs::to_bytes(&(b"kanari:committee:v1".as_slice(), public_keys))?;
        Ok(hash_data_blake3(&bytes))
    }

    pub fn attach_single_authority_certificate(
        &mut self,
        authority: String,
        key: &ed25519_dalek::SigningKey,
        public_keys: &BTreeMap<String, Vec<u8>>,
        epoch: u64,
        round: u64,
    ) -> Result<()> {
        use ed25519_dalek::Signer;
        let digest = self.certificate_signing_digest()?;
        self.certificate = Some(CheckpointCertificate {
            epoch,
            round,
            committee_digest: Self::committee_digest(public_keys)?,
            signatures: vec![CheckpointAuthoritySignature {
                authority,
                signature: key.sign(&digest).to_bytes().to_vec(),
            }],
        });
        Ok(())
    }

    pub fn verify_certificate(
        &self,
        public_keys: &BTreeMap<String, Vec<u8>>,
        authority_count: usize,
    ) -> Result<()> {
        if self.sequence == 0 {
            return Ok(());
        }
        anyhow::ensure!(authority_count > 0, "checkpoint committee is empty");
        anyhow::ensure!(
            self.gas_schedule_hash == kanari_types::GasConfig::default().consensus_hash(),
            "checkpoint gas schedule does not match the local protocol schedule"
        );
        let certificate = self
            .certificate
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("checkpoint is missing a quorum certificate"))?;
        anyhow::ensure!(
            certificate.committee_digest == Self::committee_digest(public_keys)?,
            "checkpoint committee digest mismatch"
        );
        let quorum = authority_count.saturating_mul(2) / 3 + 1;
        let digest = self.certificate_signing_digest()?;
        let mut seen = HashSet::new();
        let mut valid = 0usize;
        for authority_signature in &certificate.signatures {
            if !seen.insert(authority_signature.authority.clone()) {
                anyhow::bail!("duplicate checkpoint certificate signer");
            }
            let key_bytes = public_keys
                .get(&authority_signature.authority)
                .ok_or_else(|| anyhow::anyhow!("unknown checkpoint certificate signer"))?;
            let key_bytes: [u8; 32] = key_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid checkpoint public key length"))?;
            let signature_bytes: [u8; 64] = authority_signature
                .signature
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid checkpoint signature length"))?;
            let key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)?;
            let signature = ed25519_dalek::Signature::from_bytes(&signature_bytes);
            use ed25519_dalek::Verifier;
            key.verify(&digest, &signature)
                .map_err(|_| anyhow::anyhow!("invalid checkpoint certificate signature"))?;
            valid += 1;
        }
        anyhow::ensure!(valid >= quorum, "checkpoint certificate has {valid} signatures; quorum is {quorum}");
        Ok(())
    }
'''
    text = text.replace("    pub fn genesis() -> Self {", methods + "\n    pub fn genesis() -> Self {", 1)
    write(path, text)

    path = "crates/kanari-core/src/engine/queries.rs"
    text = read(path)
    target = '''        if checkpoint.sequence != stats.height + 1 {
            warn!(
                "[SYNC] Checkpoint #{} is not consecutive (need {})",
                checkpoint.sequence,
                stats.height + 1
            );
            anyhow::bail!(
                "Cannot sync checkpoint #{}: current height is {}",
                checkpoint.sequence,
                stats.height
            );
        }
'''
    addition = '''
        checkpoint.verify_certificate(&self.consensus_public_keys, self.authorities.len())?;
        let latest_timestamp = self
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .latest_checkpoint()
            .timestamp;
        anyhow::ensure!(checkpoint.timestamp >= latest_timestamp, "checkpoint timestamp moved backwards");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        anyhow::ensure!(
            checkpoint.timestamp <= now_ms.saturating_add(5 * 60 * 1000),
            "checkpoint timestamp exceeds allowed future drift"
        );
'''
    if text.count(target) != 1:
        raise RuntimeError("checkpoint sync insertion point not found")
    write(path, text.replace(target, target + addition, 1))

    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    text = text.replace(
        '''    pub fn produce_vertex(&self) -> Result<CheckpointProductionInfo> {
        let policy = {''',
        '''    pub fn produce_vertex(&self) -> Result<CheckpointProductionInfo> {
        let authority_count = self
            .consensus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .authorities
            .len();
        anyhow::ensure!(
            authority_count == 1 || cfg!(test),
            "multi-validator checkpoint production is frozen until authenticated Mysticeti remote blocks and committed sub-DAG certificates are wired end-to-end"
        );
        let policy = {''',
        1,
    )
    old = '''        let mut transactions = self.engine.pending_transactions_snapshot();
        transactions.sort_by(|a, b| {
            a.transaction
                .sender_address()
                .cmp(b.transaction.sender_address())
                .then_with(|| {
                    a.transaction
                        .sequence_number()
                        .cmp(&b.transaction.sequence_number())
                })
                .then_with(|| a.transaction_hash().cmp(b.transaction_hash()))
        });
        let tx_count = transactions.len();'''
    new = '''        let mut pending = self.engine.pending_transactions_snapshot();
        pending.sort_by(|a, b| {
            a.transaction
                .sender_address()
                .cmp(b.transaction.sender_address())
                .then_with(|| {
                    a.transaction
                        .sequence_number()
                        .cmp(&b.transaction.sequence_number())
                })
                .then_with(|| a.transaction_hash().cmp(b.transaction_hash()))
        });
        const MAX_VERTEX_TRANSACTIONS: usize = 10_000;
        const MAX_VERTEX_BYTES: usize = 8 * 1024 * 1024;
        let gas_config = GasConfig::default();
        let mut transactions = Vec::new();
        let mut declared_gas = 0u64;
        let mut encoded_bytes = 0usize;
        for tx in pending {
            let next_gas = declared_gas.saturating_add(tx.transaction.gas_limit());
            let tx_bytes = bcs::to_bytes(&tx)?.len();
            if transactions.len() >= MAX_VERTEX_TRANSACTIONS
                || next_gas > gas_config.max_gas_per_block
                || encoded_bytes.saturating_add(tx_bytes) > MAX_VERTEX_BYTES
            {
                break;
            }
            declared_gas = next_gas;
            encoded_bytes = encoded_bytes.saturating_add(tx_bytes);
            transactions.push(tx);
        }
        let tx_count = transactions.len();'''
    if old not in text:
        raise RuntimeError("producer transaction selection block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        ".execute_tx_waves_deterministic_parallel_with_receipts(\n                        transactions.clone(),",
        ".execute_tx_waves_strict_serial_with_receipts(\n                        transactions.clone(),",
        1,
    )
    old = '''        let checkpoint = Checkpoint::new(
            self.engine.get_stats().height.saturating_add(1),
            vec![vertex.id],
            vertex.transactions.clone(),
            vertex.metadata.state_root.clone(),
            vertex.timestamp,
            prev_hash,
        );'''
    new = '''        let mut checkpoint = Checkpoint::new(
            self.engine.get_stats().height.saturating_add(1),
            vec![vertex.id],
            vertex.transactions.clone(),
            vertex.metadata.state_root.clone(),
            vertex.timestamp,
            prev_hash,
        );
        let authority_count = self
            .consensus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .authorities
            .len();
        if authority_count == 1 {
            checkpoint.attach_single_authority_certificate(
                self.authority_id.clone(),
                &self.local_signing_key,
                &self.authority_public_keys,
                0,
                vertex.round,
            )?;
        } else {
            anyhow::bail!("refusing to finalize an uncertified multi-validator checkpoint");
        }'''
    if old not in text:
        raise RuntimeError("checkpoint construction block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        '''        if vertex.transactions.len() != vertex.metadata.tx_count {
            anyhow::bail!("Transaction count mismatch");
        }''',
        '''        if vertex.transactions.len() != vertex.metadata.tx_count {
            anyhow::bail!("Transaction count mismatch");
        }
        const MAX_VERTEX_TRANSACTIONS: usize = 10_000;
        const MAX_VERTEX_BYTES: usize = 8 * 1024 * 1024;
        anyhow::ensure!(vertex.transactions.len() <= MAX_VERTEX_TRANSACTIONS, "DAG vertex transaction limit exceeded");
        let mut vertex_bytes = 0usize;
        let mut declared_gas = 0u64;
        let gas_config = GasConfig::default();
        for tx in vertex.transactions.iter() {
            vertex_bytes = vertex_bytes.saturating_add(bcs::to_bytes(tx)?.len());
            declared_gas = declared_gas.saturating_add(tx.transaction.gas_limit());
        }
        anyhow::ensure!(vertex_bytes <= MAX_VERTEX_BYTES, "DAG vertex byte limit exceeded");
        anyhow::ensure!(declared_gas <= gas_config.max_gas_per_block, "DAG vertex gas limit exceeded");''',
        1,
    )
    text = text.replace(
        '''        Ok(())
    }

    pub fn add_network_vertex''',
        '''        anyhow::ensure!(
            parent_authors.len() >= consensus.mysticeti.quorum_threshold(),
            "DAG vertex does not reference a quorum of parent authorities"
        );
        Ok(())
    }

    pub fn add_network_vertex''',
        1,
    )
    write(path, text)
