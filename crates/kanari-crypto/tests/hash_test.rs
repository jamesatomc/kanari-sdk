// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_crypto::{
    HashAlgorithm, hash_data, hash_data_blake3, hash_data_blake3_array, hash_data_blake3_chunks,
    hash_data_blake3_chunks_array, hash_data_sha2_256, hash_data_sha2_256_chunks,
    hash_data_sha2_512, hash_data_sha2_512_chunks, hash_data_sha3_256_chunks, hash_data_sha3_512,
    hash_data_sha3_512_chunks, hash_data_shake256, hash_data_shake256_chunks,
    hash_data_shake256_custom, hash_data_shake256_custom_chunks, hash_data_with_algorithm,
    hash_data_with_algorithm_chunks,
};

#[test]
fn default_hash_is_sha3_256_and_has_stable_length() {
    let data = b"kanari hash api regression test";

    let default_hash = hash_data(data);
    let explicit_hash = hash_data_with_algorithm(data, HashAlgorithm::Sha3_256);

    assert_eq!(default_hash.len(), 32);
    assert_eq!(default_hash, explicit_hash);
}

#[test]
fn named_hash_wrappers_match_explicit_algorithm_dispatch() {
    let data = b"kanari flutter bridge uses these public wrappers";

    assert_eq!(
        hash_data_sha3_512(data),
        hash_data_with_algorithm(data, HashAlgorithm::Sha3_512)
    );
    assert_eq!(
        hash_data_sha2_256(data),
        hash_data_with_algorithm(data, HashAlgorithm::Sha2_256)
    );
    assert_eq!(
        hash_data_sha2_512(data),
        hash_data_with_algorithm(data, HashAlgorithm::Sha2_512)
    );
    assert_eq!(
        hash_data_blake3(data),
        hash_data_with_algorithm(data, HashAlgorithm::Blake3)
    );
    assert_eq!(
        hash_data_shake256(data),
        hash_data_with_algorithm(data, HashAlgorithm::Shake256)
    );
}

#[test]
fn hash_output_lengths_are_contractual_for_callers() {
    let data = b"length compatibility";

    assert_eq!(hash_data(data).len(), 32);
    assert_eq!(hash_data_sha2_256(data).len(), 32);
    assert_eq!(hash_data_blake3(data).len(), 32);
    assert_eq!(hash_data_shake256(data).len(), 32);
    assert_eq!(hash_data_sha2_512(data).len(), 64);
    assert_eq!(hash_data_sha3_512(data).len(), 64);
    assert_eq!(hash_data_shake256_custom(data, 64).len(), 64);
}

#[test]
fn shake256_custom_output_extends_default_prefix() {
    let data = b"shake xof deterministic prefix";

    let default_output = hash_data_shake256(data);
    let extended_output = hash_data_shake256_custom(data, 64);

    assert_eq!(default_output, extended_output[..32]);
}

#[test]
fn supported_hashes_are_deterministic_and_domain_distinct() {
    let data = b"same input, different algorithms";

    let sha3_256_a = hash_data(data);
    let sha3_256_b = hash_data(data);
    let sha2_256 = hash_data_sha2_256(data);
    let blake3 = hash_data_blake3(data);
    let shake256 = hash_data_shake256(data);

    assert_eq!(sha3_256_a, sha3_256_b);
    assert_ne!(sha3_256_a, sha2_256);
    assert_ne!(sha3_256_a, blake3);
    assert_ne!(sha3_256_a, shake256);
    assert_ne!(sha2_256, blake3);
    assert_ne!(sha2_256, shake256);
    assert_ne!(blake3, shake256);
}

#[test]
fn blake3_chunked_hash_matches_materialized_input() {
    let chunks: [&[u8]; 4] = [b"\x00", b"key-hash", b":", b"value"];
    let mut materialized = Vec::new();
    for chunk in chunks {
        materialized.extend_from_slice(chunk);
    }

    assert_eq!(
        hash_data_blake3_chunks(&chunks),
        hash_data_blake3(&materialized)
    );
    assert_eq!(
        hash_data_blake3_chunks_array(&chunks),
        hash_data_blake3_array(&materialized)
    );
    assert_eq!(
        hash_data_blake3_chunks_array(&chunks).to_vec(),
        hash_data_blake3(&materialized)
    );
}

#[test]
fn chunked_hashes_match_materialized_input_for_all_algorithms() {
    let chunks: [&[u8]; 5] = [b"kanari", b":", b"hash", b":", b"chunks"];
    let mut materialized = Vec::new();
    for chunk in chunks {
        materialized.extend_from_slice(chunk);
    }

    assert_eq!(
        hash_data_sha2_256_chunks(&chunks),
        hash_data_sha2_256(&materialized)
    );
    assert_eq!(
        hash_data_sha2_512_chunks(&chunks),
        hash_data_sha2_512(&materialized)
    );
    assert_eq!(hash_data_sha3_256_chunks(&chunks), hash_data(&materialized));
    assert_eq!(
        hash_data_sha3_512_chunks(&chunks),
        hash_data_sha3_512(&materialized)
    );
    assert_eq!(
        hash_data_shake256_chunks(&chunks),
        hash_data_shake256(&materialized)
    );
    assert_eq!(
        hash_data_shake256_custom_chunks(&chunks, 64),
        hash_data_shake256_custom(&materialized, 64)
    );

    for algorithm in [
        HashAlgorithm::Sha2_256,
        HashAlgorithm::Sha2_512,
        HashAlgorithm::Sha3_256,
        HashAlgorithm::Sha3_512,
        HashAlgorithm::Blake3,
        HashAlgorithm::Shake256,
    ] {
        assert_eq!(
            hash_data_with_algorithm_chunks(&chunks, algorithm),
            hash_data_with_algorithm(&materialized, algorithm)
        );
    }
}

#[test]
fn root_hash_exports_stay_compatible_with_module_paths() {
    let data = b"module path compatibility";

    assert_eq!(kanari_crypto::hashs::hash_data(data), hash_data(data));
    assert_eq!(kanari_crypto::hash::hash_data(data), hash_data(data));
}
