use super::*;

#[test]
fn test_audit_entry_creation() {
    let entry = AuditEntry::new(SecurityEvent::KeyGenerated)
        .with_resource("test-key")
        .with_actor("test-user")
        .with_details("Generated Ed25519 key");

    assert_eq!(entry.event, SecurityEvent::KeyGenerated);
    assert_eq!(entry.severity, EventSeverity::Info);
    assert!(entry.success);
    assert_eq!(entry.resource_id, Some("test-key".to_string()));
}

#[test]
fn test_event_severity() {
    assert_eq!(SecurityEvent::KeyGenerated.severity(), EventSeverity::Info);
    assert_eq!(
        SecurityEvent::AuthenticationFailure.severity(),
        EventSeverity::Error
    );
    assert_eq!(
        SecurityEvent::SuspiciousActivity.severity(),
        EventSeverity::Critical
    );
}

#[test]
fn test_audit_entry_json_serialization() {
    let entry = AuditEntry::new(SecurityEvent::WalletCreated)
        .with_resource("0x123")
        .with_success(true);

    let json = entry
        .to_json_line()
        .expect("Failed to serialize audit entry");
    assert!(json.contains("WalletCreated"));
    assert!(json.contains("0x123"));
}
