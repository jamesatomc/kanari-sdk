from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        if new in text:
            return text
        raise RuntimeError(f"missing node cutover marker: {label}")
    return text.replace(old, new, 1)


def switch_network_transaction_type() -> None:
    path = Path("crates/kanari-node/src/sync.rs")
    text = path.read_text()
    text = text.replace(
        "use kanari_types::transaction::SignedTransaction;\n",
        "use kanari_types::signed_object_transaction::SignedObjectTransaction;\n",
    )
    text = text.replace(
        "Self::parse_message::<SignedTransaction>(&tx_data, \"transaction\")",
        "Self::parse_message::<SignedObjectTransaction>(&tx_data, \"object transaction\")",
    )
    old = '''            match self
                .engine
                .submit_transactions_batch(vec![signed_tx.clone()])
            {
                Ok(tx_hashes) => {
                    info!(
                        "Received transaction from network: 0x{}",
                        hex::encode(&tx_hashes[0])
                    );
                }
                Err(e) => {
                    warn!("Failed to submit transaction from network: {}", e);
                }
            }
'''
    new = '''            match self.engine.submit_protocol_transaction(signed_tx) {
                Ok(tx_hash) => {
                    info!(
                        "Received object transaction from network: 0x{}",
                        hex::encode(tx_hash)
                    );
                }
                Err(e) => {
                    warn!("Failed to submit object transaction from network: {}", e);
                }
            }
'''
    text = replace_once(text, old, new, "network object transaction admission")
    path.write_text(text)


switch_network_transaction_type()
