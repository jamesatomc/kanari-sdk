// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::url {
    use std::ascii::{Self, String};
    use std::vector;

    const E_INVALID_URL: u64 = 0;
    /// Max URL length to bound gas / storage DoS.
    const MAX_URL_LENGTH: u64 = 2048;
    const E_URL_TOO_LONG: u64 = 1;

    /// Standard Uniform Resource Locator (URL) string.
    struct Url has store, copy, drop {
        // TODO: validate URL format
        url: String,
    }

    /// Create a `Url` with no validation.
    /// SECURITY: `new_unsafe` bypasses scheme checks — only use for
    /// already-trusted or non-network identifiers. Prefer `new`.
    public fun new_unsafe(url: String): Url {
        Url { url }
    }

    /// Create a `Url` with no validation from bytes
    /// Note: this will abort if `bytes` is not valid ASCII
    public fun new_unsafe_from_bytes(bytes: vector<u8>): Url {
        let url = ascii::string(bytes);
        Url { url }
    }

    /// Create a URL after validating its scheme and characters.
    public fun new(url: String): Url {
        assert!(is_valid(&url), E_INVALID_URL);
        assert!((ascii::length(&url) as u64) <= MAX_URL_LENGTH, E_URL_TOO_LONG);
        Url { url }
    }

    /// Create a validated URL from ASCII bytes.
    public fun new_from_bytes(bytes: vector<u8>): Url {
        new(ascii::string(bytes))
    }

    /// Returns true for http/https URLs without whitespace.
    public fun is_valid(url: &String): bool {
        let bytes = ascii::as_bytes(url);
        let length = vector::length(bytes);
        if (length < 8) {
            return false
        };

        let http = *vector::borrow(bytes, 0) == 104
            && *vector::borrow(bytes, 1) == 116
            && *vector::borrow(bytes, 2) == 116
            && *vector::borrow(bytes, 3) == 112;
        let secure = length >= 9 && *vector::borrow(bytes, 4) == 115;
        let scheme_end = if (secure) 5 else 4;
        if (!http || *vector::borrow(bytes, scheme_end) != 58
            || *vector::borrow(bytes, scheme_end + 1) != 47
            || *vector::borrow(bytes, scheme_end + 2) != 47) {
            return false
        };

        let i = scheme_end + 3;
        if (i >= length) {
            return false
        };
        if (length > MAX_URL_LENGTH) {
            return false
        };
        let cursor = i;
        while (cursor < length) {
            let character = *vector::borrow(bytes, cursor);
            // Reject whitespace, C0 controls, and DEL — prevents header/CRLF injection.
            if (character <= 32 || character == 127) {
                return false
            };
            cursor = cursor + 1;
        };
        true
    }

    /// Get inner URL
    public fun inner_url(self: &Url): String{
        self.url
    }

    /// Update the inner URL.
    /// SECURITY: validates the new value — previously this bypassed `is_valid`,
    /// letting callers swap a validated `https://` URL for `javascript:`/`data:` etc.
    public fun update(self: &mut Url, url: String) {
        assert!(is_valid(&url), E_INVALID_URL);
        assert!((ascii::length(&url) as u64) <= MAX_URL_LENGTH, E_URL_TOO_LONG);
        self.url = url;
    }
}