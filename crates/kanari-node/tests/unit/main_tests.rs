use super::validate_start_authority_config;

#[test]
fn start_requires_both_authority_fields() {
    assert!(validate_start_authority_config(&None, &None).is_err());
    assert!(validate_start_authority_config(&Some("0x1".to_string()), &None).is_err());
    assert!(validate_start_authority_config(&None, &Some(vec!["0x1".to_string()])).is_err());
}

#[test]
fn start_rejects_empty_authority_list() {
    assert!(validate_start_authority_config(&Some("0x1".to_string()), &Some(vec![])).is_err());
}

#[test]
fn start_accepts_complete_authority_config() {
    assert!(
        validate_start_authority_config(
            &Some("0x1".to_string()),
            &Some(vec!["0x1".to_string(), "0x2".to_string()]),
        )
        .is_ok()
    );
}
