use std::{io::Write, time::Duration};

use flate2::{Compression, write::GzEncoder};
use libp2p_core::PeerId;
use libp2p_identity::Keypair;
use proptest::prelude::*;
use tokio::sync::mpsc;

use super::{
    MAX_DECOMPRESSED_PAYLOAD_SIZE, P2PEventHandler, P2PMessage, P2PMessageChunk, P2PNetwork,
    P2PTopicKind, decompress_payload, is_recovery_control_message, outbound_priority,
};

#[test]
fn checkpoint_recovery_control_messages_outrank_best_effort_gossip() {
    assert!(
        outbound_priority(&P2PMessage::TargetedCheckpointRequest(
            super::CheckpointRequestMsg {
                sequence: 1,
                timestamp: 1,
                requester_peer_id: "requester".to_owned(),
                responder_peer_id: "responder".to_owned(),
            },
        )) < outbound_priority(&P2PMessage::PeerInfo(super::PeerInfoMsg {
            height: 0,
            peer_id: "peer".to_owned(),
            timestamp: 0,
            latest_checkpoint_hash: String::new(),
            latest_state_root: String::new(),
            total_transactions: 0,
        }))
    );
}

#[test]
fn chaos_does_not_delay_checkpoint_recovery_control_messages() {
    let request = P2PMessage::CheckpointRequest(7, 1);
    assert!(is_recovery_control_message(&request));
    assert!(!is_recovery_control_message(&P2PMessage::NewTransaction(
        "transaction".to_owned(),
    )));
}

fn test_handler() -> P2PEventHandler {
    let network = P2PNetwork::new(Keypair::generate_ed25519(), 0, false).unwrap();
    let (message_tx, _message_rx) = mpsc::channel(1);
    P2PEventHandler::new(network, message_tx)
}

fn chunked_message(message: &P2PMessage) -> (PeerId, Vec<P2PMessageChunk>) {
    let encoded = bincode::encode_to_vec(message, bincode::config::standard()).unwrap();
    let digest = kanari_crypto::hash_data_blake3(&encoded);
    let mut transfer_id = [0u8; 32];
    transfer_id.copy_from_slice(&digest);
    let split = encoded.len().div_ceil(2);
    let chunks = encoded
        .chunks(split)
        .enumerate()
        .map(|(index, data)| P2PMessageChunk {
            transfer_id,
            topic: P2PTopicKind::Checkpoint,
            index: index as u16,
            total: encoded.len().div_ceil(split) as u16,
            data: data.to_vec(),
        })
        .collect();
    (PeerId::random(), chunks)
}

#[tokio::test]
async fn compressed_payload_is_decompressed_off_loop_and_preserves_source() {
    let network = P2PNetwork::new(Keypair::generate_ed25519(), 0, false).unwrap();
    let (message_tx, mut message_rx) = mpsc::channel(1);
    let mut handler = P2PEventHandler::new(network, message_tx);
    let source = PeerId::random();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(b"checkpoint-data").unwrap();
    let compressed = encoder.finish().unwrap();

    handler.process_decoded_message(source, P2PMessage::CompressedCheckpoint(compressed));

    let received = tokio::time::timeout(Duration::from_secs(2), message_rx.recv())
        .await
        .expect("decompression timeout")
        .expect("decompressed message");
    assert_eq!(received.source, source);
    assert!(matches!(
        received.message,
        P2PMessage::NewCheckpoint(ref data) if data == "checkpoint-data"
    ));
}

#[tokio::test]
async fn chunk_reassembly_accepts_out_of_order_payload_once() {
    let mut handler = test_handler();
    let message = P2PMessage::CheckpointRequest(42, 7);
    let (peer, mut chunks) = chunked_message(&message);
    chunks.reverse();

    assert!(handler.accept_chunk(peer, chunks[0].clone()).is_none());
    let result = handler
        .accept_chunk(peer, chunks[1].clone())
        .expect("complete valid payload");
    assert!(matches!(result, P2PMessage::CheckpointRequest(42, 7)));
    assert!(handler.chunk_assemblies.is_empty());
}

#[tokio::test]
async fn chunk_reassembly_rejects_mixed_topics_and_clears_memory() {
    let mut handler = test_handler();
    let message = P2PMessage::CheckpointRequest(42, 7);
    let (peer, mut chunks) = chunked_message(&message);

    assert!(handler.accept_chunk(peer, chunks[0].clone()).is_none());
    chunks[1].topic = P2PTopicKind::Transaction;
    assert!(handler.accept_chunk(peer, chunks[1].clone()).is_none());
    assert!(handler.chunk_assemblies.is_empty());
}

#[test]
fn decompression_bomb_is_rejected_at_configured_limit() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&vec![b'x'; MAX_DECOMPRESSED_PAYLOAD_SIZE + 1])
        .unwrap();
    let compressed = encoder.finish().unwrap();

    let error = decompress_payload(compressed).unwrap_err();
    assert!(error.to_string().contains("exceeds"));
}

proptest! {
    #[test]
    fn arbitrary_compressed_input_never_panics(input in prop::collection::vec(any::<u8>(), 0..65_536)) {
        let _ = decompress_payload(input);
    }
}

/// Explicitly opt-in because this exercises 2k attacker-controlled payloads.
/// It is intended for the adversarial soak runner, not ordinary developer CI.
#[test]
#[ignore = "long-running P2P decompression/DoS soak test"]
fn long_run_malformed_compressed_payloads_are_bounded() {
    use proptest::test_runner::{Config, TestRunner};

    let mut runner = TestRunner::new(Config {
        cases: 2_048,
        max_shrink_iters: 0,
        ..Config::default()
    });
    let strategy = prop::collection::vec(any::<u8>(), 0..16_384);

    runner
        .run(&strategy, |input| {
            let _ = decompress_payload(input);
            Ok(())
        })
        .expect("untrusted compressed payload must never panic");
}
