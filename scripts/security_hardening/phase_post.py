from __future__ import annotations

from . import phase1_zero_fee, phase_supply_checks
from .common import read, write


def apply() -> None:
    phase1_zero_fee.apply()
    phase_supply_checks.apply()

    path = "crates/kanari-node/src/sync.rs"
    text = read(path)
    before = """        let encoded_size = bcs::to_bytes(&checkpoint)
            .map(|bytes| bytes.len())
            .unwrap_or(self.max_buffer_bytes);
        let mut byte_count = self
            .buffered_checkpoint_bytes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut buffer = self.checkpoint_buffer_guard();"""
    after = """        let encoded_size = bcs::to_bytes(&checkpoint)
            .map(|bytes| bytes.len())
            .unwrap_or(self.max_buffer_bytes);
        let mut buffer = self.checkpoint_buffer_guard();
        let mut byte_count = self
            .buffered_checkpoint_bytes
            .lock()
            .unwrap_or_else(|e| e.into_inner());"""
    if before not in text:
        raise RuntimeError("sync byte accounting block not found")
    write(path, text.replace(before, after, 1))

    path = "crates/kanari-core/src/consensus.rs"
    text = read(path)
    before = """            self.gas_schedule_hash == kanari_types::GasConfig::default().consensus_hash(),
            \"checkpoint gas schedule does not match the local protocol schedule\""""
    after = """            self.gas_schedule_hash.as_slice()
                == kanari_types::GasConfig::default().consensus_hash().as_slice(),
            \"checkpoint gas schedule does not match the local protocol schedule\""""
    if before not in text:
        raise RuntimeError("gas schedule check not found")
    write(path, text.replace(before, after, 1))
