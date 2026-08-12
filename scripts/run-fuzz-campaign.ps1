param(
    [ValidateRange(1, 168)]
    [int]$Hours = 8,

    [ValidateRange(1, 256)]
    [int]$Workers = 1,

    [switch]$IncludeIgnoredLongRuns,

    [switch]$SkipClippy
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$deadline = (Get-Date).AddHours($Hours)
$iteration = 0

Write-Host "Kanari fuzz/conflict campaign"
Write-Host "  deadline=$deadline workers=$Workers include_ignored=$IncludeIgnoredLongRuns"

Push-Location $repoRoot
try {
    if (-not $SkipClippy) {
        cargo clippy `
            -p kanari-move-runtime-v1 `
            -p kanari-core `
            -p kanari-node `
            -p kanari-rpc-server `
            --all-targets `
            -- -D warnings
    }

    while ((Get-Date) -lt $deadline) {
        $iteration += 1
        Write-Host "[$(Get-Date -Format o)] fuzz iteration $iteration"

        cargo test -p kanari-move-runtime-v1 scheduler -- --test-threads=$Workers
        cargo test -p kanari-move-runtime-v1 changeset -- --test-threads=$Workers
        cargo test -p kanari-core produce_dag_vertex -- --test-threads=$Workers
        cargo test -p kanari-core conflicting_speculative_wave_replays_to_strict_serial_root -- --test-threads=1
        cargo test -p kanari-node arbitrary_compressed_input_never_panics -- --test-threads=$Workers
        # Boundary/property regression coverage for the non-crypto attack surface.
        # These are intentionally part of the recurring campaign so changes to
        # BCS/RPC/object policy/consensus cannot silently reduce coverage.
        cargo test -p kanari-rpc-server rpc_adversarial_inputs_are_rejected_without_server_error -- --test-threads=$Workers
        cargo test -p kanari-rpc-server object_input -- --test-threads=$Workers
        cargo test -p kanari-move-runtime-v1 object_input -- --test-threads=$Workers
        cargo test -p kanari-move-runtime-v1 gas_object -- --test-threads=$Workers
        cargo test -p smt --all-targets -- --test-threads=$Workers

        if ($IncludeIgnoredLongRuns) {
            cargo test -p kanari-node long_run_malformed_compressed_payloads_are_bounded -- --ignored --test-threads=$Workers
            cargo test -p kanari-core long_run_byzantine_native_blocks_cannot_advance_checkpoint -- --ignored --test-threads=$Workers
        }
    }
} finally {
    Pop-Location
}

Write-Host "Kanari fuzz/conflict campaign completed: $iteration iteration(s) without failure."
