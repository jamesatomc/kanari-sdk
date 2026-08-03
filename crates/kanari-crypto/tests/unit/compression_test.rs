use super::*;

#[test]
fn test_compression_roundtrip() {
    let original = b"This is some test data that should compress well due to repetition. \
                     This is some test data that should compress well due to repetition.";

    let compressed = compress_data(original).unwrap();
    let decompressed = decompress_data(&compressed).unwrap();

    assert_eq!(decompressed, original);
    assert!(compressed.len() < original.len());
}
