#![no_main]
// Disabled: Windows MSVC requires LLVM/ASAN which is not available
// Use `cargo test --test fuzz_*` instead for property-based testing

use kanari_crypto::{
    CurveType, generate_keypair,
    signatures::{sign_message, verify_signature},
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Split input into message and curve type indicator
    if data.len() < 2 {
        return;
    }

    let curve_indicator = data[0];
    let message = &data[1..];

    // Try different curve types based on input
    let curve_type = match curve_indicator % 3 {
        0 => CurveType::K256,
        1 => CurveType::P256,
        2 => CurveType::Ed25519,
        _ => return,
    };

    // Generate a keypair for fuzzing
    let Ok(keypair) = generate_keypair(curve_type) else {
        return;
    };

    // Sign the message
    let Ok(signature) = sign_message(&keypair.private_key, message, curve_type) else {
        return;
    };

    // Verify with tagged address (should succeed)
    let tagged_addr = keypair.tagged_address();
    let Ok(verify_result) = verify_signature(&tagged_addr, message, &signature) else {
        return;
    };

    // Verification should succeed for valid signature
    assert!(verify_result, "Valid signature should verify successfully");

    // Test with corrupted signature (should fail or return false)
    if !signature.is_empty() {
        let mut corrupted_sig = signature.clone();
        corrupted_sig[0] = corrupted_sig[0].wrapping_add(1);

        if let Ok(result) = verify_signature(&tagged_addr, message, &corrupted_sig) {
            // Corrupted signature should NOT verify successfully
            assert!(!result, "Corrupted signature should fail verification");
        }
    }

    // Test with wrong message (should fail or return false)
    if !message.is_empty() {
        let mut wrong_message = message.to_vec();
        wrong_message[0] = wrong_message[0].wrapping_add(1);

        if let Ok(result) = verify_signature(&tagged_addr, &wrong_message, &signature) {
            // Wrong message should NOT verify successfully
            assert!(!result, "Signature should not verify for different message");
        }
    }

    // Test with untagged address (should return error)
    let untagged_result = verify_signature(&keypair.address, message, &signature);
    assert!(
        untagged_result.is_err(),
        "Verification should require tagged address"
    );
});
