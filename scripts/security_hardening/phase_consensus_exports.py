from .common import read, write


def apply():
    path = "crates/kanari-core/src/lib.rs"
    text = read(path)
    text = text.replace(
        '''    Checkpoint, ConsensusRuntimeProtocol, DagProductionPolicy, DagVertex, PersistentDagState,
    VertexId,
};''',
        '''    Checkpoint, CheckpointVote, ConsensusRuntimeProtocol, DagProductionPolicy, DagVertex,
    PersistentDagState, VertexId,
};''',
        1,
    )
    text = text.replace(
        '''    BlockData, BlockchainEngine, BlockchainStats, CheckpointProductionInfo, CheckpointSyncData,
    FullBlockData, TransactionExecutionReceipt,
};''',
        '''    BlockData, BlockchainEngine, BlockchainStats, CheckpointProductionInfo, CheckpointSyncData,
    ConsensusUpdate, FullBlockData, TransactionExecutionReceipt,
};''',
        1,
    )
    write(path, text)
