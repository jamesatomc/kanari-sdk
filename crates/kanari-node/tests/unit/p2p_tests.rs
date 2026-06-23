use super::*;

#[test]
fn compressed_payload_round_trip() {
    let payload = "kanari".repeat(20_000);
    let compressed = gzip_string(&payload).unwrap();

    assert_eq!(decompress_payload(&compressed).unwrap(), payload);
}

#[test]
fn compressed_payload_rejects_excessive_expansion() {
    let payload = "x".repeat(MAX_DECOMPRESSED_PAYLOAD_SIZE + 1);
    let compressed = gzip_string(&payload).unwrap();

    let error = decompress_payload(&compressed).unwrap_err();
    assert!(error.to_string().contains("exceeds"));
}
