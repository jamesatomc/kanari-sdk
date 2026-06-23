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
