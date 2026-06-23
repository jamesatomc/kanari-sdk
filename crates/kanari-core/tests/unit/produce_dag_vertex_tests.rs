use super::*;
#[path = "test_support.rs"]
mod test_support;

use test_support::{authority_key, signed_transfer};

fn signed_network_vertex(
    author: &str,
    signing_key: &ed25519_dalek::SigningKey,
    round: u64,
    parents: Vec<VertexId>,
) -> DagVertex {
    let tx = signed_transfer(0);
    let mut vertex = DagVertex::new(
        round,
        author.to_string(),
        "kanari-v2-mysticeti".to_string(),
        parents,
        vec![tx],
        vec![7u8; 32],
        123,
    );
    use ed25519_dalek::Signer;
    vertex.signature = signing_key
        .sign(&vertex.signing_digest().unwrap())
        .to_bytes()
        .to_vec();
    vertex
}

#[test]
fn test_dag_engine_defaults_to_mysticeti_protocol() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let signing_key = authority_key(11);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        signing_key.verifying_key().to_bytes().to_vec(),
    );
    let dag_engine = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ],
        signing_key,
        public_keys,
    )
    .unwrap();
    let protocol = dag_engine
        .consensus
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .protocol();

    assert_eq!(protocol.protocol, "mysticeti");
    assert_eq!(protocol.wave_length, 3);
    assert_eq!(protocol.direct_commit_quorum, 3);
    assert!(protocol.pipeline);
    assert!(protocol.leader_wait);
}

#[test]
fn test_dag_engine_secure_constructor_rejects_mismatched_local_key() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let expected = authority_key(11);
    let wrong = authority_key(33);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        expected.verifying_key().to_bytes().to_vec(),
    );

    let result = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec!["auth1".to_string()],
        wrong,
        public_keys,
    );

    assert!(result.is_err());
}

#[test]
fn test_add_network_vertex_accepts_valid_remote_vertex() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let local_key = authority_key(11);
    let remote_key = authority_key(22);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        local_key.verifying_key().to_bytes().to_vec(),
    );
    public_keys.insert(
        "auth2".to_string(),
        remote_key.verifying_key().to_bytes().to_vec(),
    );
    let dag_engine = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec!["auth1".to_string(), "auth2".to_string()],
        local_key,
        public_keys,
    )
    .unwrap();

    let vertex = signed_network_vertex("auth2", &remote_key, 1, vec![]);
    dag_engine.add_network_vertex(vertex).unwrap();

    let consensus = dag_engine
        .consensus
        .read()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(consensus.vertices.len(), 1);
    assert_eq!(consensus.vertices[0].author, "auth2");
}

#[test]
fn test_add_network_vertex_rejects_invalid_signature() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let local_key = authority_key(11);
    let remote_key = authority_key(22);
    let wrong_key = authority_key(33);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        local_key.verifying_key().to_bytes().to_vec(),
    );
    public_keys.insert(
        "auth2".to_string(),
        remote_key.verifying_key().to_bytes().to_vec(),
    );
    let dag_engine = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec!["auth1".to_string(), "auth2".to_string()],
        local_key,
        public_keys,
    )
    .unwrap();

    let vertex = signed_network_vertex("auth2", &wrong_key, 1, vec![]);
    let error = dag_engine.add_network_vertex(vertex).unwrap_err();
    assert!(error.to_string().contains("Invalid DAG vertex signature"));
}

#[test]
fn test_add_network_vertex_rejects_payload_modified_after_signing() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let local_key = authority_key(11);
    let remote_key = authority_key(22);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        local_key.verifying_key().to_bytes().to_vec(),
    );
    public_keys.insert(
        "auth2".to_string(),
        remote_key.verifying_key().to_bytes().to_vec(),
    );
    let dag_engine = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec!["auth1".to_string(), "auth2".to_string()],
        local_key,
        public_keys,
    )
    .unwrap();

    let mut vertex = signed_network_vertex("auth2", &remote_key, 1, vec![]);
    vertex.metadata.state_root[0] ^= 0xff;

    let error = dag_engine.add_network_vertex(vertex).unwrap_err();
    assert!(error.to_string().contains("Invalid DAG vertex signature"));
}

#[test]
fn test_add_network_vertex_rejects_missing_parent() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let local_key = authority_key(11);
    let remote_key = authority_key(22);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        local_key.verifying_key().to_bytes().to_vec(),
    );
    public_keys.insert(
        "auth2".to_string(),
        remote_key.verifying_key().to_bytes().to_vec(),
    );
    let dag_engine = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec!["auth1".to_string(), "auth2".to_string()],
        local_key,
        public_keys,
    )
    .unwrap();

    let vertex = signed_network_vertex("auth2", &remote_key, 2, vec![[9u8; 32]]);
    let error = dag_engine.add_network_vertex(vertex).unwrap_err();
    assert!(error.to_string().contains("Missing parent"));
}
