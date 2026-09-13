// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Base64 and Base64URL encoding/decoding utilities
module kanari_system::base64 {
    /// Max input length to bound native alloc / gas grief (1MB).
    const MAX_BASE64_LENGTH: u64 = 1048576;
    /// Input exceeds `MAX_BASE64_LENGTH`.
    const EInputTooLong: u64 = 0;
    #[allow(unused_const)]
    /// Native decoding failed (malformed encoding).
    /// NOTE: the native aborts on bad encoding; callers cannot distinguish
    /// bad-encoding aborts from OOM — validate inputs before calling.
    const EDecodeFailed: u64 = 1;

    /// Decodes a base64 or base64url encoded string into bytes
    /// Supports both standard base64 and base64url (URL-safe) encoding.
    /// Aborts with `EInputTooLong` when `input` exceeds 1MB; aborts on
    /// malformed encoding (native).
    public fun decode(input: &vector<u8>): vector<u8> {
        assert!(std::vector::length(input) <= MAX_BASE64_LENGTH, EInputTooLong);
        native_decode(input)
    }

    native fun native_decode(input: &vector<u8>): vector<u8>;

    /// Encodes bytes into base64 string.
    /// Aborts with `EInputTooLong` when `input` exceeds 1MB.
    public fun encode(input: &vector<u8>): vector<u8> {
        assert!(std::vector::length(input) <= MAX_BASE64_LENGTH, EInputTooLong);
        native_encode(input)
    }

    native fun native_encode(input: &vector<u8>): vector<u8>;

    /// Max allowed input length for `encode`/`decode`.
    public fun max_length(): u64 { MAX_BASE64_LENGTH }
}
