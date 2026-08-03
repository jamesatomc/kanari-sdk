use super::*;

#[test]
fn test_key_metadata_creation() {
    let metadata = KeyMetadata::new("test-key");
    assert_eq!(metadata.key_id, "test-key");
    assert_eq!(metadata.rotation_count, 0);
    assert!(!metadata.rotation_due);
}

#[test]
fn test_rotation_manager() {
    let mut manager = KeyRotationManager::new();
    manager.register_key("key1".to_string());
    manager.register_key("key2".to_string());

    let stats = manager.get_statistics();
    assert_eq!(stats.total_keys, 2);
    assert_eq!(stats.keys_due_for_rotation, 0);
    assert_eq!(stats.total_rotations, 0);
}

#[test]
fn test_should_not_rotate_new_key() {
    let manager = KeyRotationManager::new();
    let metadata = KeyMetadata::new("test-key");

    assert!(!metadata.should_rotate(manager.get_policy()));
}
