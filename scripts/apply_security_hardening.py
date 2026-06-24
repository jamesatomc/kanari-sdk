from __future__ import annotations

from security_hardening import (
    phase0_ci,
    phase1_financial,
    phase2_consensus,
    phase2_replay,
    phase3_gas,
    phase4_persistence_network,
    phase5_auth,
    phase_post,
)

PATCHSET_VERSION = 2


def main() -> None:
    phase0_ci.apply()
    phase1_financial.apply()
    phase2_consensus.apply()
    phase2_replay.apply()
    phase3_gas.apply()
    phase4_persistence_network.apply()
    phase5_auth.apply()
    phase_post.apply()
    print(f"security hardening patchset v{PATCHSET_VERSION} applied")


if __name__ == "__main__":
    main()
