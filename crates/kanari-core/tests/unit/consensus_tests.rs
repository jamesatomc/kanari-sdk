use super::*;

#[test]
fn dag_vertex_verify_allows_external_vertex_id_when_payload_hash_is_cached() {
    let tx = SignedTransaction::new(kanari_types::transaction::Transaction::PublishModule {
        sender: "0x1".to_string(),
        module_bytes: vec![1, 2, 3],
        module_name: "example".to_string(),
        gas_limit: 1,
        gas_price: 1,
        sequence_number: 0,
    });
    let mut vertex = DagVertex::new(
        1,
        "auth1".to_string(),
        "kanari-v2-mysticeti".to_string(),
        Vec::new(),
        vec![tx],
        vec![7u8; 32],
        123,
    );
    let payload_hash = vertex.compute_hash().unwrap();
    vertex.id = [9u8; 32];

    vertex.verify_payload_consistency().unwrap();
    assert_eq!(vertex.compute_hash().unwrap(), payload_hash);
}

#[test]
fn dag_vertex_signing_digest_rejects_tampered_cached_digest() {
    let tx = SignedTransaction::new(kanari_types::transaction::Transaction::PublishModule {
        sender: "0x1".to_string(),
        module_bytes: vec![1, 2, 3],
        module_name: "example".to_string(),
        gas_limit: 1,
        gas_price: 1,
        sequence_number: 0,
    });
    let mut vertex = DagVertex::new(
        1,
        "auth1".to_string(),
        "kanari-v2-mysticeti".to_string(),
        Vec::new(),
        vec![tx],
        vec![7u8; 32],
        123,
    );
    vertex.cached_signing_digest = Some([3u8; 32].to_vec());

    let error = vertex.signing_digest().unwrap_err();
    assert!(error.to_string().contains("signing digest mismatch"));
}

fn test_committee(
    size: usize,
) -> (
    std::collections::BTreeMap<String, Vec<u8>>,
    Vec<(String, ed25519_dalek::SigningKey)>,
) {
    let mut public_keys = std::collections::BTreeMap::new();
    let mut signers = Vec::new();
    for index in 0..size {
        let authority = format!("authority-{index}");
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[(index as u8) + 1; 32]);
        public_keys.insert(
            authority.clone(),
            signing_key.verifying_key().to_bytes().to_vec(),
        );
        signers.push((authority, signing_key));
    }
    (public_keys, signers)
}

fn checkpoint_fixture() -> Checkpoint {
    Checkpoint::new(
        1,
        vec![[7u8; 32]],
        Vec::<SignedTransaction>::new(),
        vec![8u8; 32],
        1_000,
        vec![0u8; 32],
    )
}

#[test]
fn checkpoint_vote_binds_the_complete_local_draft() {
    let (public_keys, signers) = test_committee(4);
    let checkpoint = checkpoint_fixture();
    let vote = CheckpointVote::new(
        checkpoint.clone(),
        3,
        9,
        signers[0].0.clone(),
        &signers[0].1,
        &public_keys,
    )
    .unwrap();
    vote.verify_for_checkpoint(&checkpoint, &public_keys)
        .unwrap();

    let mut tampered = checkpoint;
    tampered.state_root[0] ^= 1;
    assert!(vote.verify_for_checkpoint(&tampered, &public_keys).is_err());
}

#[test]
fn checkpoint_certificate_requires_two_thirds_plus_one_unique_signers() {
    let (public_keys, signers) = test_committee(4);
    let mut checkpoint = checkpoint_fixture();
    let committee_digest = Checkpoint::committee_digest(&public_keys).unwrap();
    let mut signatures = Vec::new();
    for (authority, signing_key) in signers.iter().take(3) {
        let vote = CheckpointVote::new(
            checkpoint.clone(),
            0,
            11,
            authority.clone(),
            signing_key,
            &public_keys,
        )
        .unwrap();
        signatures.push(CheckpointAuthoritySignature {
            authority: vote.authority,
            signature: vote.signature,
        });
    }
    checkpoint.certificate = Some(CheckpointCertificate {
        epoch: 0,
        round: 11,
        committee_digest,
        signatures,
    });
    checkpoint.verify_certificate(&public_keys, 4).unwrap();

    checkpoint.certificate.as_mut().unwrap().signatures.pop();
    assert!(checkpoint.verify_certificate(&public_keys, 4).is_err());
}

#[test]
fn non_genesis_checkpoint_without_certificate_is_rejected() {
    let (public_keys, _) = test_committee(4);
    let checkpoint = checkpoint_fixture();
    assert!(checkpoint.verify_certificate(&public_keys, 4).is_err());
}
