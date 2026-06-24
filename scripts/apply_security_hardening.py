from __future__ import annotations

import sys

sys.dont_write_bytecode = True

from security_hardening import (
    phase1_financial,
    phase2_consensus,
    phase2_replay,
    phase3_gas,
    phase4_channels,
    phase4_persistence_network,
    phase5_auth,
    phase_known_fixups,
    phase_post,
)

PATCHSET_VERSION = 8


def main() -> None:
    phase1_financial.apply()
    phase2_consensus.apply()
    phase2_replay.apply()
    phase3_gas.apply()
    phase4_persistence_network.apply()
    phase4_channels.apply()
    phase5_auth.apply()
    phase_post.apply()
    phase_known_fixups.apply()
    print(f"security hardening patchset v{PATCHSET_VERSION} applied")


if __name__ == "__main__":
    main()
