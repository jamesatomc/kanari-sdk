// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

pub(crate) fn env_usize_clamped(name: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

pub(crate) struct NodeRuntimeConfig;

impl NodeRuntimeConfig {
    pub(crate) fn p2p_channel_capacity() -> usize {
        env_usize_clamped("KANARI_P2P_CHANNEL_CAPACITY", 1024, 16, 65_536)
    }

    pub(crate) fn max_concurrent_sync_messages() -> usize {
        env_usize_clamped("KANARI_MAX_CONCURRENT_SYNC_MESSAGES", 128, 1, 4096)
    }

    pub(crate) fn dag_vertices_per_response() -> usize {
        env_usize_clamped("KANARI_DAG_VERTICES_PER_RESPONSE", 8, 1, 64)
    }

    pub(crate) fn p2p_max_inflight_chunked_payloads() -> usize {
        env_usize_clamped("KANARI_P2P_MAX_INFLIGHT_CHUNKED_PAYLOADS", 16, 1, 256)
    }

    pub(crate) fn p2p_max_inflight_chunked_payloads_per_peer() -> usize {
        env_usize_clamped(
            "KANARI_P2P_MAX_INFLIGHT_CHUNKED_PAYLOADS_PER_PEER",
            4,
            1,
            64,
        )
    }

    pub(crate) fn p2p_max_concurrent_decompressions() -> usize {
        env_usize_clamped("KANARI_P2P_MAX_CONCURRENT_DECOMPRESSIONS", 8, 1, 128)
    }
}
