use crate::{CurveType, hd_wallet};

#[test]
fn test_derive_rejects_post_quantum_curve() {
    // Known BIP-39 test mnemonic (do not use in production)
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let password = "";
    let path = "m/44'/60'/0'/0/0";

    let res = hd_wallet::derive_keypair_from_path(mnemonic, password, path, CurveType::Dilithium3);

    match res {
        Err(hd_wallet::HdError::DerivationFailed(msg)) => {
            assert!(
                msg.contains("Post-quantum"),
                "unexpected error message: {}",
                msg
            );
        }
        other => panic!("expected DerivationFailed for PQC curve, got: {:?}", other),
    }
}
