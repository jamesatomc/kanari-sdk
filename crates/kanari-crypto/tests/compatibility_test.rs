use kanari_crypto::{
    CurveType, Keystore, Wallet,
    encryption::{decrypt_data, encrypt_data},
    generate_keypair,
};
use move_core_types::account_address::AccountAddress;
use serde_json::{Value, json};
use std::path::Path;
use std::str::FromStr;

const PASSWORD: &str = "StrongPassw0rd!";

#[test]
fn legacy_keystore_missing_new_metadata_loads_and_roundtrips() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("kanari.keystore");

    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "keys": {},
            "mnemonic": {
                "addresses": []
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let keystore = Keystore::load_from_path(&path).unwrap();
    assert!(keystore.keys.is_empty());
    assert!(keystore.get_mnemonic_addresses().is_empty());
    assert!(!keystore.version.is_empty());

    let reloaded = Keystore::load_from_path(&path).unwrap();
    assert!(reloaded.keys.is_empty());
    assert!(reloaded.last_modified.is_some());
}

#[test]
fn legacy_array_encrypted_key_format_upgrades_and_decrypts() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("kanari.keystore");
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let encrypted = encrypt_data(b"legacy wallet bytes", PASSWORD).unwrap();
    let encrypted_value = serde_json::to_value(encrypted).unwrap();

    let legacy_encrypted = json!({
        "ciphertext_array": encrypted_value["ciphertext"]
            .as_str()
            .and_then(|ciphertext| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, ciphertext).ok())
            .unwrap(),
        "nonce_array": encrypted_value["nonce"]
            .as_str()
            .and_then(|nonce| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, nonce).ok())
            .unwrap(),
        "salt": encrypted_value["salt"].clone()
    });

    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "keys": {
                keypair.address.clone(): legacy_encrypted
            },
            "mnemonic": {
                "addresses": [keypair.address.clone()]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let keystore = Keystore::load_from_path(&path).unwrap();
    let loaded = keystore.get_wallet(&keypair.address).unwrap();
    assert_eq!(
        decrypt_data(loaded, PASSWORD).unwrap(),
        b"legacy wallet bytes"
    );

    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let saved_key = &saved["keys"][&keypair.address];
    assert!(
        saved_key["ciphertext"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(saved_key["nonce"].as_str().is_some_and(|s| !s.is_empty()));
}

#[test]
fn legacy_wallet_toml_missing_optional_secret_fields_uses_safe_defaults() {
    let address = AccountAddress::from_str(
        "0x3ba63b92aac5f2bff87e580e820b61faf1c5fe9ae12f0bc8addd931a340b3146",
    )
    .unwrap();
    let legacy_toml = format!(
        r#"
address = "{}"
curve_type = "Ed25519"
"#,
        address
    );

    let wallet: Wallet = toml::from_str(&legacy_toml).unwrap();
    assert_eq!(wallet.address, address);
    assert_eq!(wallet.curve_type, CurveType::Ed25519);
    assert!(wallet.private_key.is_empty());
    assert!(wallet.seed_phrase.is_empty());
    assert!(wallet.derivation_path.is_none());
}

#[test]
fn current_wallet_toml_without_derivation_path_stays_compatible() {
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let wallet = Wallet::new(
        AccountAddress::from_str(&keypair.address).unwrap(),
        keypair.export_private_key_secure().to_string(),
        String::new(),
        None,
        CurveType::K256,
    );

    let mut value = toml::to_string(&wallet).unwrap();
    value = value
        .lines()
        .filter(|line| !line.starts_with("derivation_path"))
        .collect::<Vec<_>>()
        .join("\n");

    let parsed: Wallet = toml::from_str(&value).unwrap();
    assert_eq!(parsed.address, wallet.address);
    assert_eq!(parsed.curve_type, wallet.curve_type);
    assert!(parsed.derivation_path.is_none());
}

fn load_keystore_fixture_into_temp(fixture: &str) -> Keystore {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("kanari.keystore");
    std::fs::write(&path, fixture).unwrap();
    let loaded = Keystore::load_from_path(&path).unwrap();
    loaded.validate().unwrap();
    loaded
}

#[test]
fn corpus_legacy_keystore_fixtures_load_validate_and_upgrade() {
    let fixtures = [
        include_str!("fixtures/legacy_keystore_v0_2_3_minimal.json"),
        include_str!("fixtures/legacy_keystore_v0_2_4_metadata.json"),
    ];

    for fixture in fixtures {
        let loaded = load_keystore_fixture_into_temp(fixture);
        assert!(
            !loaded.version.is_empty(),
            "legacy keystore corpus must be upgraded with a version"
        );
        assert!(loaded.keys.is_empty());
    }
}

#[test]
fn corpus_legacy_wallet_fixtures_parse_with_safe_defaults() {
    let fixtures = [
        include_str!("fixtures/legacy_wallet_v0_2_3_minimal.toml"),
        include_str!("fixtures/legacy_wallet_v0_2_4_with_empty_secrets.toml"),
    ];

    for fixture in fixtures {
        let wallet: Wallet = toml::from_str(fixture).unwrap();
        assert_ne!(wallet.address, AccountAddress::ZERO);
        assert!(
            wallet.private_key.is_empty(),
            "legacy corpus must not inject unexpected private key material"
        );
        assert!(
            wallet.seed_phrase.is_empty(),
            "legacy corpus must not inject unexpected mnemonic material"
        );
        assert!(wallet.derivation_path.is_none());
    }
}

#[test]
fn compatibility_fixture_paths_are_kept_in_repo() {
    for path in [
        "tests/fixtures/legacy_keystore_v0_2_3_minimal.json",
        "tests/fixtures/legacy_keystore_v0_2_4_metadata.json",
        "tests/fixtures/legacy_wallet_v0_2_3_minimal.toml",
        "tests/fixtures/legacy_wallet_v0_2_4_with_empty_secrets.toml",
    ] {
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(path).exists(),
            "missing compatibility corpus fixture: {path}"
        );
    }
}
