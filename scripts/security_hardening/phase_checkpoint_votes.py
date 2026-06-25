from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "crates/kanari-core/src/consensus.rs"
    text = read(path)
    marker = '''#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointCertificate {
    pub epoch: u64,
    pub round: u64,
    pub committee_digest: Vec<u8>,
    pub signatures: Vec<CheckpointAuthoritySignature>,
}
'''
    addition = marker + '''
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointVote {
    pub checkpoint_id: VertexId,
    pub sequence: u64,
    pub epoch: u64,
    pub round: u64,
    pub authority: AuthorityId,
    pub signature: Vec<u8>,
}

impl CheckpointVote {
    pub fn new(
        mut checkpoint: Checkpoint,
        epoch: u64,
        round: u64,
        authority: AuthorityId,
        signing_key: &ed25519_dalek::SigningKey,
        public_keys: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
        use ed25519_dalek::Signer;
        checkpoint.certificate = None;
        let committee_digest = Checkpoint::committee_digest(public_keys)?;
        let digest = checkpoint.certificate_signing_digest(epoch, round, &committee_digest)?;
        let checkpoint_id = vertex_id_from_hash_bytes(&checkpoint.hash()?);
        Ok(Self {
            checkpoint_id,
            sequence: checkpoint.sequence,
            epoch,
            round,
            authority,
            signature: signing_key.sign(&digest).to_bytes().to_vec(),
        })
    }

    pub fn verify_for_checkpoint(
        &self,
        checkpoint: &Checkpoint,
        public_keys: &BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        anyhow::ensure!(
            vertex_id_from_hash_bytes(&checkpoint.hash()?) == self.checkpoint_id,
            "checkpoint vote digest does not match the local draft"
        );
        anyhow::ensure!(checkpoint.sequence == self.sequence, "checkpoint vote sequence mismatch");
        let public_key = public_keys
            .get(&self.authority)
            .ok_or_else(|| anyhow::anyhow!("unknown checkpoint voter {}", self.authority))?;
        let public_key: [u8; 32] = public_key
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid checkpoint voter public key length"))?;
        let signature: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid checkpoint vote signature length"))?;
        let committee_digest = Checkpoint::committee_digest(public_keys)?;
        let digest = checkpoint.certificate_signing_digest(
            self.epoch,
            self.round,
            &committee_digest,
        )?;
        let key = ed25519_dalek::VerifyingKey::from_bytes(&public_key)?;
        let signature = ed25519_dalek::Signature::from_bytes(&signature);
        use ed25519_dalek::Verifier;
        key.verify(&digest, &signature)
            .map_err(|_| anyhow::anyhow!("invalid checkpoint vote signature"))?;
        Ok(())
    }
}
'''
    if marker not in text:
        raise RuntimeError("CheckpointCertificate declaration not found")
    write(path, text.replace(marker, addition, 1))
