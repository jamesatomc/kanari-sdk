use super::*;
use tempfile::TempDir;

#[test]
fn test_peer_store_save_load() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("peers.json");

    // Create and save
    let mut store = PeerStore::new(file_path.clone());
    let peer_id = PeerId::random();
    store.add_peer(peer_id, vec![]);
    store.save().unwrap();

    // Load
    let loaded = PeerStore::load(file_path).unwrap();
    assert_eq!(loaded.peers.len(), 1);
}
