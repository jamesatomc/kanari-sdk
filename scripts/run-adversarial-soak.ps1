param(
    [ValidateRange(1, 168)]
    [int]$Hours = 8,
    [ValidateRange(0, 10080)]
    [int]$Minutes = 0,
    [ValidateRange(1, 256)]
    [int]$Workers = 1,
    [string[]]$RpcUrl = @(),
    [ValidateRange(1, 100000)]
    [int]$RpcRequests = 500,
    [ValidateRange(1, 512)]
    [int]$RpcConcurrency = 32,
    [switch]$IncludeOversizedRpc
)

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$rpcProbeScript = Join-Path $repoRoot 'scripts\run-rpc-load-dos.ps1'
$duration = if ($Minutes -gt 0) {
    [TimeSpan]::FromMinutes($Minutes)
} else {
    [TimeSpan]::FromHours($Hours)
}
$deadline = (Get-Date).Add($duration)
$iteration = 0

Write-Host "Kanari adversarial soak: runs until $deadline with $Workers worker(s)."
Write-Host "Covers bounded P2P decompression/DoS and Byzantine Mysticeti native blocks."
if ($RpcUrl.Count -gt 0) {
    if (-not (Test-Path -LiteralPath $rpcProbeScript)) {
        throw "Missing RPC adversarial probe: $rpcProbeScript"
    }
    Write-Host "RPC adversarial probe enabled for: $($RpcUrl -join ', ')"
}

while ((Get-Date) -lt $deadline) {
    $iteration += 1
    Write-Host "[$(Get-Date -Format o)] iteration $iteration"

    cargo test -p kanari-node long_run_malformed_compressed_payloads_are_bounded -- --ignored --test-threads=$Workers
    cargo test -p kanari-core long_run_byzantine_native_blocks_cannot_advance_checkpoint -- --ignored --test-threads=$Workers
    cargo test -p kanari-rpc-server rpc_adversarial_inputs_are_rejected_without_server_error -- --test-threads=$Workers

    foreach ($url in $RpcUrl) {
        & $rpcProbeScript `
            -RpcUrl $url `
            -Requests $RpcRequests `
            -Concurrency $RpcConcurrency `
            -IncludeMalformed `
            -IncludeOversized:$IncludeOversizedRpc
    }
}

Write-Host "Adversarial soak completed: $iteration iteration(s) without a test failure."
