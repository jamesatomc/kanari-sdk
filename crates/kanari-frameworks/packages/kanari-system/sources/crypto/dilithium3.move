// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// NIST ML-DSA / Dilithium3 verification.
module kanari_system::dilithium3 {
    const PUBLIC_KEY_LENGTH: u64 = 1952;
    const SIGNATURE_LENGTH: u64 = 3293;

    public fun public_key_length(): u64 { PUBLIC_KEY_LENGTH }
    public fun signature_length(): u64 { SIGNATURE_LENGTH }

    native public fun verify(signature: &vector<u8>, public_key: &vector<u8>, message: &vector<u8>): bool;

    #[test]
    fun test_sizes() {
        assert!(public_key_length() == 1952, 0);
        assert!(signature_length() == 3293, 1);
    }

    #[test]
    fun test_rejects_malformed_input() {
        assert!(!verify(&x"00", &x"00", &x"6b616e617269"), 0);
    }

    #[test]
    fun test_rejects_empty_input() {
        assert!(!verify(&x"", &x"", &x""), 0);
    }

    #[test]
    fun test_rejects_truncated_signature() {
        assert!(!verify(&x"000102", &x"00", &x"6b616e617269"), 0);
    }
}
