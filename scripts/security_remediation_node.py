#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def save(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def replace_required(text: str, old: str, new: str, label: str) -> str:
    if new in text:
        return text
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one source pattern, found {count}")
    return text.replace(old, new, 1)


# ---------------------------------------------------------------------------
# P2P: bounded channels, signed-source envelope, and claimed-identity binding.
# ---------------------------------------------------------------------------
path = "crates/kanari-node/src/p2p.rs"
s = load(path)
s = replace_required(
    s,
    "const MAX_DECOMPRESSED_PAYLOAD_SIZE: usize = 8 * 1024 * 1024;",
    "const MAX_DECOMPRESSED_PAYLOAD_SIZE: usize = 8 * 1024 * 1024;\npub const P2P_INBOUND_QUEUE_CAPACITY: usize = 1024;\npub const P2P_OUTBOUND_QUEUE_CAPACITY: usize = 1024;",
    "P2P queue capacities",
)
s = replace_required(
    s,
    "/// P2P message types\n#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]\npub enum P2PMessage",
    "/// P2P message types\n#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]\npub enum P2PMessage",
    "P2P enum marker",
)
if "pub struct AuthenticatedP2PMessage" not in s:
    marker = "#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]\npub struct PeerInfoMsg"
    envelope = """#[derive(Debug, Clone)]
pub struct AuthenticatedP2PMessage {
    /// Signed gossipsub author, not the forwarding peer.
    pub source_peer_id: String,
    pub message: P2PMessage,
}

"""
    if marker not in s:
        raise RuntimeError("authenticated message insertion marker missing")
    s = s.replace(marker, envelope + marker, 1)
s = replace_required(
    s,
    ".validation_mode(ValidationMode::Permissive)",
    ".validation_mode(ValidationMode::Strict)",
    "strict gossipsub validation",
)
s = s.replace(
    "pub message_tx: mpsc::UnboundedSender<P2PMessage>,",
    "pub message_tx: mpsc::Sender<AuthenticatedP2PMessage>,",
)
s = s.replace(
    "pub outgoing_rx: Option<mpsc::UnboundedReceiver<P2PMessage>>",
    "pub outgoing_rx: Option<mpsc::Receiver<P2PMessage>>",
)
s = s.replace(
    "pub fn new(network: P2PNetwork, message_tx: mpsc::UnboundedSender<P2PMessage>) -> Self",
    "pub fn new(network: P2PNetwork, message_tx: mpsc::Sender<AuthenticatedP2PMessage>) -> Self",
)
s = s.replace(
    "pub fn with_outgoing(mut self, outgoing_rx: mpsc::UnboundedReceiver<P2PMessage>) -> Self",
    "pub fn with_outgoing(mut self, outgoing_rx: mpsc::Receiver<P2PMessage>) -> Self",
)
old = """    fn forward_message(&mut self, msg: P2PMessage, context: &str) -> bool {
        if self.message_forwarding_closed || self.message_tx.is_closed() {
            if !self.message_forwarding_closed {
                warn!(
                    "{}: receiver channel is closed; suppressing further incoming P2P forwards",
                    context
                );
                self.message_forwarding_closed = true;
            }
            return false;
        }

        match self.message_tx.send(msg) {
            Ok(_) => true,
            Err(e) => {
                warn!(
                    "{}: {}; suppressing further incoming P2P forwards",
                    context, e
                );
                self.message_forwarding_closed = true;
                false
            }
        }
    }"""
new = """    fn forward_message(&mut self, source: &PeerId, msg: P2PMessage, context: &str) -> bool {
        if self.message_forwarding_closed || self.message_tx.is_closed() {
            if !self.message_forwarding_closed {
                warn!("{}: receiver channel is closed", context);
                self.message_forwarding_closed = true;
            }
            return false;
        }

        let envelope = AuthenticatedP2PMessage {
            source_peer_id: source.to_string(),
            message: msg,
        };
        match self.message_tx.try_send(envelope) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("{}: inbound P2P queue is full; dropping message", context);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("{}: receiver channel is closed", context);
                self.message_forwarding_closed = true;
                false
            }
        }
    }

    fn claimed_identity_matches(source: &PeerId, msg: &P2PMessage) -> bool {
        let source = source.to_string();
        match msg {
            P2PMessage::PeerInfo(info) => info.peer_id == source,
            P2PMessage::DagVertexRebroadcast(message) => message.sender_peer_id == source,
            P2PMessage::DagVertexRequest(request) => request.requester_peer_id == source,
            P2PMessage::DagVertexResponse(response) => response.responder_peer_id == source,
            P2PMessage::TargetedCheckpointRequest(request) => request.requester_peer_id == source,
            P2PMessage::TargetedCheckpointResponse(response) => response.responder_peer_id == source,
            P2PMessage::CompressedTargetedCheckpointResponse(response) => {
                response.responder_peer_id == source
            }
            _ => true,
        }
    }"""
s = replace_required(s, old, new, "bounded authenticated forward")
s = s.replace(
    "        compressed_data: &[u8],\n        make_message:",
    "        source: &PeerId,\n        compressed_data: &[u8],\n        make_message:",
)
s = s.replace(
    "Ok(data) => self.forward_message(make_message(data), send_context),",
    "Ok(data) => self.forward_message(source, make_message(data), send_context),",
)
s = s.replace(
    "        resp: &CompressedCheckpointResponseMsg,\n        send_context:",
    "        source: &PeerId,\n        resp: &CompressedCheckpointResponseMsg,\n        send_context:",
)
s = s.replace(
    "            Ok(checkpoint_data) => self.forward_message(\n                P2PMessage::TargetedCheckpointResponse",
    "            Ok(checkpoint_data) => self.forward_message(\n                source,\n                P2PMessage::TargetedCheckpointResponse",
)
# Bind to signed message author (message.source), never merely the propagation hop.
old = """                    Ok((msg, _)) => {
                        Self::log_received_message(&propagation_source, &msg);

                        match &msg {"""
new = """                    Ok((msg, _)) => {
                        let authenticated_source = message.source.unwrap_or(propagation_source);
                        if !Self::claimed_identity_matches(&authenticated_source, &msg) {
                            warn!(
                                "[P2P] Dropping message with spoofed embedded peer identity from {}",
                                authenticated_source
                            );
                            return;
                        }
                        Self::log_received_message(&authenticated_source, &msg);

                        match &msg {"""
s = replace_required(s, old, new, "authenticated gossipsub source")
s = s.replace(
    "self.forward_decompressed_message(\n                                    compressed_data,",
    "self.forward_decompressed_message(\n                                    &authenticated_source,\n                                    compressed_data,",
)
s = s.replace(
    "self.forward_targeted_checkpoint_response(\n                                    resp,",
    "self.forward_targeted_checkpoint_response(\n                                    &authenticated_source,\n                                    resp,",
)
s = s.replace(
    "self.forward_message(msg, \"[P2P] Failed to forward P2P message\");",
    "self.forward_message(\n                            &authenticated_source,\n                            msg,\n                            \"[P2P] Failed to forward P2P message\",\n                        );",
)
save(path, s)

# ---------------------------------------------------------------------------
# Sync manager: bounded outgoing queue with explicit backpressure.
# ---------------------------------------------------------------------------
path = "crates/kanari-node/src/sync.rs"
s = load(path)
s = s.replace(
    "network_tx: mpsc::UnboundedSender<P2PMessage>,",
    "network_tx: mpsc::Sender<P2PMessage>,",
)
s = s.replace(
    "network_tx: mpsc::UnboundedSender<P2PMessage>,",
    "network_tx: mpsc::Sender<P2PMessage>,",
)
old = """    fn send_network_message(&self, msg: P2PMessage, context: &str) -> bool {
        match self.network_tx.send(msg) {
            Ok(_) => true,
            Err(e) => {
                error!("{}: {}", context, e);
                false
            }
        }
    }"""
new = """    fn send_network_message(&self, msg: P2PMessage, context: &str) -> bool {
        match self.network_tx.try_send(msg) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("{}: outbound P2P queue is full", context);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                error!("{}: outbound P2P queue is closed", context);
                false
            }
        }
    }"""
s = replace_required(s, old, new, "bounded sync outbound")
save(path, s)

# ---------------------------------------------------------------------------
# Node app: persistent identity, bounded queues, no task per inbound message,
# and consensus private key file permissions.
# ---------------------------------------------------------------------------
path = "crates/kanari-node/src/app.rs"
s = load(path)
s = s.replace(
    "use crate::p2p::{P2PEventHandler, P2PMessage, P2PNetwork};",
    "use crate::p2p::{\n    AuthenticatedP2PMessage, P2PEventHandler, P2PMessage, P2PNetwork,\n    P2P_INBOUND_QUEUE_CAPACITY, P2P_OUTBOUND_QUEUE_CAPACITY,\n};",
)
old = """pub fn configure_consensus_signing_key(
    engine: &mut BlockchainEngine,
    private_key_hex: &str,
    public_keys_path: &std::path::Path,
) -> Result<()> {
    let private_key = decode_hex_bytes("consensus private key seed", private_key_hex, 32)?;"""
new = """pub fn configure_consensus_signing_key(
    engine: &mut BlockchainEngine,
    private_key_path: &std::path::Path,
    public_keys_path: &std::path::Path,
) -> Result<()> {
    ensure_secret_file_permissions(private_key_path)?;
    let private_key_hex = std::fs::read_to_string(private_key_path).map_err(|e| {
        KanariError::OperationFailed {
            context: "failed to read consensus private key file",
            details: format!("{}: {}", private_key_path.display(), e),
        }
    })?;
    let private_key = decode_hex_bytes(
        "consensus private key seed",
        private_key_hex.trim(),
        32,
    )?;"""
s = replace_required(s, old, new, "consensus private key file")
if "fn ensure_secret_file_permissions" not in s:
    marker = "fn log_shutdown() {"
    helper = """fn ensure_secret_file_permissions(path: &std::path::Path) -> Result<()> {
    let metadata = std::fs::metadata(path).map_err(|e| KanariError::OperationFailed {
        context: "failed to inspect secret key file",
        details: format!("{}: {}", path.display(), e),
    })?;
    if !metadata.is_file() {
        anyhow::bail!("Secret key path is not a regular file: {}", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "Secret key file {} must have mode 0600 or stricter; found {:o}",
                path.display(),
                mode
            );
        }
    }
    Ok(())
}

fn load_or_create_p2p_identity(data_dir: &std::path::Path) -> Result<Keypair> {
    use std::io::Write;
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("p2p-identity.key");
    if path.exists() {
        ensure_secret_file_permissions(&path)?;
        let encoded = std::fs::read(&path)?;
        return Keypair::from_protobuf_encoding(&encoded)
            .map_err(|e| anyhow::anyhow!("Invalid persisted P2P identity: {}", e));
    }

    let keypair = Keypair::generate_ed25519();
    let encoded = keypair
        .to_protobuf_encoding()
        .map_err(|e| anyhow::anyhow!("Failed to encode P2P identity: {}", e))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(&encoded)?;
    file.sync_all()?;
    ensure_secret_file_permissions(&path)?;
    Ok(keypair)
}

"""
    if marker not in s:
        raise RuntimeError("secret helper insertion marker missing")
    s = s.replace(marker, helper + marker, 1)
s = s.replace(
    "network_tx: &tokio::sync::mpsc::UnboundedSender<P2PMessage>",
    "network_tx: &tokio::sync::mpsc::Sender<P2PMessage>",
)
s = s.replace(
    "match network_tx.send(msg) {",
    "match network_tx.try_send(msg) {",
)
s = replace_required(
    s,
    """        Err(e) => {
            tracing::warn!("{}: {}", failure_context, e);
            false
        }""",
    """        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            tracing::warn!("{}: outbound queue is full", failure_context);
            false
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            tracing::warn!("{}: outbound queue is closed", failure_context);
            false
        }""",
    "app bounded queue send",
)
s = s.replace(
    "network_tx: &tokio::sync::mpsc::UnboundedSender<P2PMessage>",
    "network_tx: &tokio::sync::mpsc::Sender<P2PMessage>",
)
s = replace_required(
    s,
    """    let (p2p_msg_tx, mut p2p_msg_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();
    let (network_tx, network_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();

    let keypair = Keypair::generate_ed25519();""",
    """    let (p2p_msg_tx, mut p2p_msg_rx) = tokio::sync::mpsc::channel::<AuthenticatedP2PMessage>(
        P2P_INBOUND_QUEUE_CAPACITY,
    );
    let (network_tx, network_rx) =
        tokio::sync::mpsc::channel::<P2PMessage>(P2P_OUTBOUND_QUEUE_CAPACITY);

    let keypair = load_or_create_p2p_identity(&data_dir)?;""",
    "bounded channels and stable identity",
)
old = """            while let Some(msg) = p2p_msg_rx.recv().await {
                let sync = sync_for_messages.clone();
                match tokio::spawn(async move {
                    sync.handle_message(msg).await;
                })
                .await
                {
                    Ok(()) => {}
                    Err(e) if e.is_panic() => {
                        tracing::error!(
                            "[P2P] Sync message handler panicked; continuing to process incoming messages: {}",
                            e
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "[P2P] Sync message handler task failed; continuing to process incoming messages: {}",
                            e
                        );
                    }
                }
            }"""
new = """            while let Some(envelope) = p2p_msg_rx.recv().await {
                sync_for_messages.handle_message(envelope.message).await;
            }"""
s = replace_required(s, old, new, "remove task per message")
save(path, s)

# ---------------------------------------------------------------------------
# CLI: key path instead of secret hex and secure keygen permissions.
# ---------------------------------------------------------------------------
path = "crates/kanari-node/src/main.rs"
s = load(path)
s = s.replace(
    "/// Local Ed25519 consensus private key seed as 32-byte hex\n        #[arg(long)]\n        consensus_private_key_hex: String,",
    "/// File containing the local 32-byte Ed25519 consensus private seed as hex.\n        /// The file must have mode 0600 or stricter on Unix.\n        #[arg(long)]\n        consensus_private_key: std::path::PathBuf,",
)
s = s.replace("consensus_private_key_hex,", "consensus_private_key,")
s = s.replace("&consensus_private_key_hex,", "&consensus_private_key,")
old = "std::fs::write(private_key_path, private_seed)?;"
new = """{
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&private_key_path)?;
            file.write_all(private_seed.as_bytes())?;
            file.sync_all()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&private_key_path, std::fs::Permissions::from_mode(0o600))?;
            }
        }"""
s = replace_required(s, old, new, "secure consensus keygen")
save(path, s)

print("node transport, queue, and key hardening applied or already present")
