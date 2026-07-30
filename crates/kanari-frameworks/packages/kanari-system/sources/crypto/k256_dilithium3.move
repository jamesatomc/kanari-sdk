// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Hybrid K256 + Dilithium3 verification. Both components must verify.
module kanari_system::k256_dilithium3 {
    native public fun verify(
        signature: &vector<u8>,
        k256_public_key: &vector<u8>,
        dilithium3_public_key: &vector<u8>,
        message: &vector<u8>,
    ): bool;

    #[test]
    fun test_rejects_malformed_hybrid_signature() {
        assert!(!verify(&x"0000", &x"00", &x"00", &x"6b616e617269"), 0);
    }

    #[test]
    fun test_rejects_truncated_classical_segment() {
        // Prefix says 2 classical bytes, but only one byte follows.
        assert!(!verify(&x"0002aa", &x"00", &x"00", &x"6b616e617269"), 0);
    }

    #[test]
    fun test_rejects_empty_input() {
        assert!(!verify(&x"", &x"", &x"", &x""), 0);
    }
}
