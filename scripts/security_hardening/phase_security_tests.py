from .common import read, write


def apply():
    path = "crates/kanari-core/tests/unit/consensus_tests.rs"
    text = read(path)
    sentinel = "fn checkpoint_vote_binds_the_complete_local_draft()"
    if sentinel in text:
        return
    text += r'''

fn test_committee(size: usize) -> (
    std::collections::BTreeMap<String, Vec<u8>>,
    Vec<(String, ed25519_dalek::SigningKey)>,
) {
    let mut public_keys = std::collections::BTreeMap::new();
    let mut signers = Vec::new();
    for index in 0..size {
        let authority = format!("authority-{index}");
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[(index as u8) + 1; 32]);
        public_keys.insert(authority.clone(), signing_key.verifying_key().to_bytes().to_vec());
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
    vote.verify_for_checkpoint(&checkpoint, &public_keys).unwrap();

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

    checkpoint
        .certificate
        .as_mut()
        .unwrap()
        .signatures
        .pop();
    assert!(checkpoint.verify_certificate(&public_keys, 4).is_err());
}

#[test]
fn non_genesis_checkpoint_without_certificate_is_rejected() {
    let (public_keys, _) = test_committee(4);
    let checkpoint = checkpoint_fixture();
    assert!(checkpoint.verify_certificate(&public_keys, 4).is_err());
}
'''
    write(path, text)
