from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "crates/kanari-node/src/p2p.rs"
    text = read(path)
    text = text.replace(
        "pub message_tx: mpsc::UnboundedSender<P2PMessage>",
        "pub message_tx: mpsc::Sender<P2PMessage>",
        1,
    )
    text = text.replace(
        "pub outgoing_rx: Option<mpsc::UnboundedReceiver<P2PMessage>>",
        "pub outgoing_rx: Option<mpsc::Receiver<P2PMessage>>",
        1,
    )
    text = text.replace(
        "message_tx: mpsc::UnboundedSender<P2PMessage>",
        "message_tx: mpsc::Sender<P2PMessage>",
        1,
    )
    text = text.replace(
        "outgoing_rx: mpsc::UnboundedReceiver<P2PMessage>",
        "outgoing_rx: mpsc::Receiver<P2PMessage>",
        1,
    )
    old = '''        match self.message_tx.send(msg) {
            Ok(_) => true,
            Err(e) => {
                warn!(
                    "{}: {}; suppressing further incoming P2P forwards",
                    context, e
                );
                self.message_forwarding_closed = true;
                false
            }
        }'''
    new = '''        match self.message_tx.try_send(msg) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("{}: bounded incoming P2P queue is full; dropping message", context);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("{}: receiver channel is closed", context);
                self.message_forwarding_closed = true;
                false
            }
        }'''
    if old not in text:
        raise RuntimeError("P2P forward send block not found")
    write(path, text.replace(old, new, 1))

    path = "crates/kanari-node/src/sync.rs"
    text = read(path)
    if text.count("mpsc::UnboundedSender<P2PMessage>") != 2:
        raise RuntimeError("expected two SyncManager unbounded sender types")
    text = text.replace("mpsc::UnboundedSender<P2PMessage>", "mpsc::Sender<P2PMessage>")
    old = '''        match self.network_tx.send(msg) {
            Ok(_) => true,
            Err(e) => {
                error!("{}: {}", context, e);
                false
            }
        }'''
    new = '''        match self.network_tx.try_send(msg) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("{}: bounded outgoing P2P queue is full; dropping message", context);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                error!("{}: outgoing P2P queue is closed", context);
                false
            }
        }'''
    if old not in text:
        raise RuntimeError("SyncManager send block not found")
    write(path, text.replace(old, new, 1))

    path = "crates/kanari-node/src/app.rs"
    text = read(path)
    old = '''    let (p2p_msg_tx, mut p2p_msg_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();
    let (network_tx, network_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();'''
    new = '''    const P2P_CHANNEL_CAPACITY: usize = 1024;
    let (p2p_msg_tx, mut p2p_msg_rx) =
        tokio::sync::mpsc::channel::<P2PMessage>(P2P_CHANNEL_CAPACITY);
    let (network_tx, network_rx) =
        tokio::sync::mpsc::channel::<P2PMessage>(P2P_CHANNEL_CAPACITY);'''
    if old not in text:
        raise RuntimeError("node P2P channel creation block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        '''                network_tx_for_rpc
                    .send(P2PMessage::NewTransaction(payload))''',
        '''                network_tx_for_rpc
                    .try_send(P2PMessage::NewTransaction(payload))''',
        1,
    )
    write(path, text)
