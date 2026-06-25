from __future__ import annotations

import sys

sys.dont_write_bytecode = True

from security_hardening import (
    phase1_financial,
    phase2_consensus,
    phase2_replay,
    phase3_gas,
    phase3_native_fix,
    phase4_channels,
    phase4_persistence_network,
    phase5_auth,
    phase_atomic_commit,
    phase_checkpoint_votes,
    phase_consensus_exports,
    phase_consensus_progress,
    phase_known_fixups,
    phase_move_changeset,
    phase_move_overlay,
    phase_move_parser,
    phase_mysticeti_authority,
    phase_mysticeti_backend,
    phase_mysticeti_digest,
    phase_mysticeti_network,
    phase_mysticeti_production,
    phase_mysticeti_vertex,
    phase_node_channel_fixups,
    phase_node_consensus_loop,
    phase_post,
    phase_protocol_limits,
    phase_runtime_reload,
    phase_vote_accept,
    phase_vote_fields,
    phase_vote_prepare,
    phase_vote_sync,
    phase_vote_transport,
)

PATCHSET_VERSION = 18


def main() -> None:
    phase1_financial.apply()
    phase2_consensus.apply()
    phase2_replay.apply()
    phase3_gas.apply()
    phase3_native_fix.apply()
    phase4_persistence_network.apply()
    phase4_channels.apply()
    phase_node_channel_fixups.apply()
    phase5_auth.apply()
    phase_post.apply()
    phase_known_fixups.apply()
    phase_checkpoint_votes.apply()
    phase_move_changeset.apply()
    phase_move_parser.apply()
    phase_move_overlay.apply()
    phase_runtime_reload.apply()
    phase_mysticeti_digest.apply()
    phase_mysticeti_vertex.apply()
    phase_mysticeti_backend.apply()
    phase_mysticeti_authority.apply()
    phase_vote_fields.apply()
    phase_mysticeti_production.apply()
    phase_vote_prepare.apply()
    phase_vote_accept.apply()
    phase_mysticeti_network.apply()
    phase_consensus_progress.apply()
    phase_consensus_exports.apply()
    phase_vote_transport.apply()
    phase_vote_sync.apply()
    phase_node_consensus_loop.apply()
    phase_protocol_limits.apply()
    phase_atomic_commit.apply()
    print(f"security hardening patchset v{PATCHSET_VERSION} applied")


if __name__ == "__main__":
    main()
