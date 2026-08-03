use super::*;
use tempfile::TempDir;

#[test]
fn test_backup_metadata_creation() {
    let metadata =
        BackupMetadata::new(5, true, "abc123".to_string()).with_description("Test backup");

    assert_eq!(metadata.key_count, 5);
    assert!(metadata.has_mnemonic);
    assert_eq!(metadata.checksum, "abc123");
    assert_eq!(metadata.description, Some("Test backup".to_string()));
}

#[test]
fn test_backup_manager_creation() {
    let temp_dir = TempDir::new().unwrap();
    let manager = BackupManager::new(temp_dir.path().to_path_buf());

    assert_eq!(manager.get_backup_dir(), temp_dir.path());
}

#[test]
fn test_list_empty_backups() {
    let temp_dir = TempDir::new().unwrap();
    let manager = BackupManager::new(temp_dir.path().to_path_buf());

    let backups = manager.list_backups().unwrap();
    assert_eq!(backups.len(), 0);
}
