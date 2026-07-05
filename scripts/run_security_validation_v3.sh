#!/usr/bin/env bash
set -uo pipefail

ARTIFACT_DIR="${RUNNER_TEMP:-/tmp}/kanari-security-validation"
mkdir -p "$ARTIFACT_DIR"

run_transforms() {
  python3 scripts/security_remediation_batch1.py &&
  python3 scripts/security_remediation_consensus.py &&
  python3 scripts/security_remediation_node.py &&
  python3 scripts/security_remediation_checkpoint_votes_v2.py &&
  python3 scripts/security_remediation_state_commit.py &&
  python3 scripts/security_remediation_auth.py &&
  python3 scripts/security_remediation_fixups.py
}

set +e
run_transforms >"$ARTIFACT_DIR/transforms.log" 2>&1
TRANSFORM_STATUS=$?
echo "$TRANSFORM_STATUS" >"$ARTIFACT_DIR/transforms.status"
cat "$ARTIFACT_DIR/transforms.log"
if [[ $TRANSFORM_STATUS -ne 0 ]]; then
  exit $TRANSFORM_STATUS
fi

cargo fmt --all >"$ARTIFACT_DIR/rustfmt.log" 2>&1
FMT_STATUS=$?
echo "$FMT_STATUS" >"$ARTIFACT_DIR/rustfmt.status"
cat "$ARTIFACT_DIR/rustfmt.log"
if [[ $FMT_STATUS -ne 0 ]]; then
  exit $FMT_STATUS
fi

tar -czf "$ARTIFACT_DIR/security-transformed-source-v3.tar.gz" \
  crates/kanari-core \
  crates/kanari-node \
  crates/kanari-rpc-server \
  crates/kanari-auth \
  crates/run-auth \
  move-execution/v1/kanari-move-runtime-v1

cargo check \
  -p kanari-core \
  -p kanari-node \
  -p kanari-move-runtime-v1 \
  -p kanari-rpc-server \
  -p kanari-auth \
  -p run-auth \
  >"$ARTIFACT_DIR/cargo-check.log" 2>&1
CHECK_STATUS=$?
echo "$CHECK_STATUS" >"$ARTIFACT_DIR/cargo-check.status"
cat "$ARTIFACT_DIR/cargo-check.log"
if [[ $CHECK_STATUS -ne 0 ]]; then
  exit $CHECK_STATUS
fi

cargo test -p kanari-move-runtime-v1 \
  >"$ARTIFACT_DIR/runtime-tests.log" 2>&1 || exit $?
cargo test -p kanari-core --lib \
  >"$ARTIFACT_DIR/core-tests.log" 2>&1 || exit $?
cargo test -p kanari-auth \
  >"$ARTIFACT_DIR/auth-tests.log" 2>&1 || exit $?
cargo run -p kanari -- move test crates/kanari-frameworks/packages/kanari-system \
  >"$ARTIFACT_DIR/framework-move-tests.log" 2>&1 || exit $?
cargo run -p kanari -- move test sdk/kanari_pay/backend/dex_v1 \
  >"$ARTIFACT_DIR/dex-move-tests.log" 2>&1 || exit $?
cargo run -p kanari -- move test sdk/kanari_pay/backend/kanari_escrow \
  >"$ARTIFACT_DIR/escrow-move-tests.log" 2>&1 || exit $?

printf '0\n' >"$ARTIFACT_DIR/validation.status"
