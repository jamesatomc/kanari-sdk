from .common import read, write


def apply():
    path = "crates/kanari-node/src/app.rs"
    text = read(path)
    text = text.replace(
        '''    let mut pending_gossip_ready_at: Option<Instant> = None;

    loop {''',
        '''    let mut pending_gossip_ready_at: Option<Instant> = None;
    let mut last_consensus_step = Instant::now() - Duration::from_secs(1);

    loop {''',
        1,
    )
    text = text.replace(
        '''        let should_produce_pending = stats.pending_transactions > 0 && pending_gossip_ready;

        if should_produce_pending {
            match engine.produce_checkpoint() {''',
        '''        let consensus_due = last_consensus_step.elapsed() >= Duration::from_millis(250)
            && engine.dag_needs_progress().unwrap_or(false);
        let should_produce = (stats.pending_transactions > 0 && pending_gossip_ready)
            || consensus_due;

        if should_produce {
            last_consensus_step = Instant::now();
            match engine.produce_checkpoint() {''',
        1,
    )
    marker = '''                    if let Some(ref node_idx) = node_indexer {'''
    vote_broadcast = '''                    for vote in block_info.checkpoint_votes {
                        serialize_and_queue_message(
                            &network_tx,
                            &vote,
                            P2PMessage::CheckpointVote,
                            "Failed to serialize checkpoint vote",
                            "Failed to queue checkpoint vote",
                        );
                    }

'''
    if marker not in text:
        raise RuntimeError("node indexer marker not found")
    text = text.replace(marker, vote_broadcast + marker, 1)
    write(path, text)
