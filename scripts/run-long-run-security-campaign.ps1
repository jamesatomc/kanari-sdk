param(
    [ValidateRange(1, 10080)]
    [int]$Minutes = 30,

    [Parameter(Mandatory = $true)]
    [string[]]$Senders,

    [Parameter(Mandatory = $true)]
    [string]$Recipient,

    [string]$Password = $env:KANARI_LOAD_PASSWORD,

    [string]$KeystorePath = "$env:USERPROFILE\.kanari\kanari_config\kanari.keystore",

    [ValidateRange(1, 1000000)]
    [int]$CountPerSender = 250,

    [ValidateRange(1, 1000000)]
    [int]$FaucetCoinsPerSender = 0,

    [ValidateRange(1, 64)]
    [int]$FanoutBatchSize = 64,

    [ValidateSet('follower', 'leader', 'two-node', 'round-robin', 'client-ingress')]
    [string]$CrashDuringLoadPattern = 'two-node',

    [ValidateRange(0, 20)]
    [int]$CrashDuringLoadRoundsPerIteration = 2,

    [ValidateRange(0, 20)]
    [int]$ChaosRoundsPerIteration = 4,

    [ValidateRange(0, 20)]
    [int]$RecoveryAuditRoundsPerIteration = 2,

    [ValidateRange(0, 30000)]
    [int]$P2pPublishDelayMs = 0,

    [ValidateRange(0, 8)]
    [int]$P2pDuplicatePublishes = 0,

    [ValidateRange(1, 512)]
    [int]$RpcConcurrency = 64,

    [ValidateRange(1, 100000)]
    [int]$RpcRequests = 1000,

    [switch]$IncludeOversizedRpc,

    [ValidateSet('release', 'debug')]
    [string]$BuildProfile = 'release',

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$chaosCampaign = Join-Path $repoRoot 'scripts\run-chaos-network-campaign.ps1'
$adversarialSoak = Join-Path $repoRoot 'scripts\run-adversarial-soak.ps1'

if (-not (Test-Path -LiteralPath $chaosCampaign)) {
    throw "Missing network chaos campaign runner: $chaosCampaign"
}
if (-not (Test-Path -LiteralPath $adversarialSoak)) {
    throw "Missing adversarial soak runner: $adversarialSoak"
}
if ([string]::IsNullOrWhiteSpace($Password)) {
    throw "Set KANARI_LOAD_PASSWORD or pass -Password for the temporary load-test keystore."
}
if (-not (Test-Path -LiteralPath $KeystorePath)) {
    throw "Keystore not found: $KeystorePath"
}

$runId = Get-Date -Format 'yyyyMMdd-HHmmss'
$runRoot = Join-Path $repoRoot ".codex-runlogs\security-campaign-$runId"
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

# Reserve one transfer coin and one independent gas coin per transaction.
# The chaos runner will enforce the same invariant; setting it here makes the
# public campaign entry point hard to misconfigure for high-volume runs.
$requiredCoinFanout = (2 * $CountPerSender) + 2
if ($FaucetCoinsPerSender -le 0) {
    $FaucetCoinsPerSender = $requiredCoinFanout
} elseif ($FaucetCoinsPerSender -lt $requiredCoinFanout) {
    throw "FaucetCoinsPerSender=$FaucetCoinsPerSender is too low for CountPerSender=$CountPerSender. Use at least $requiredCoinFanout."
}

$chaosMinutes = [Math]::Max(1, [int][Math]::Floor($Minutes * 0.80))
$soakMinutes = [Math]::Max(1, $Minutes - $chaosMinutes)
$baseP2pPort = 24000
$baseRpcPort = 24001

Write-Host "Kanari long-run security campaign"
Write-Host "  duration=${Minutes}m chaos=${chaosMinutes}m adversarial_soak=${soakMinutes}m"
Write-Host "  run_logs=$runRoot"
Write-Host "  senders=$($Senders -join ', ') recipient=$Recipient"
Write-Host "  count_per_sender=$CountPerSender fanout=$FaucetCoinsPerSender crash_pattern=$CrashDuringLoadPattern"
Write-Host "  p2p_delay_ms=$P2pPublishDelayMs duplicate_publishes=$P2pDuplicatePublishes"
Write-Host "  base_p2p=$baseP2pPort base_rpc=$baseRpcPort rpc_probe_requests_per_node=$RpcRequests"

& $chaosCampaign `
    -Minutes $chaosMinutes `
    -Senders $Senders `
    -Recipient $Recipient `
    -Password $Password `
    -KeystorePath $KeystorePath `
    -CountPerSender $CountPerSender `
    -FaucetCoinsPerSender $FaucetCoinsPerSender `
    -FanoutBatchSize $FanoutBatchSize `
    -AutoCoinFanout `
    -ChaosRoundsPerIteration $ChaosRoundsPerIteration `
    -CrashDuringLoadRoundsPerIteration $CrashDuringLoadRoundsPerIteration `
    -CrashDuringLoadPattern $CrashDuringLoadPattern `
    -RecoveryAuditRoundsPerIteration $RecoveryAuditRoundsPerIteration `
    -P2pPublishDelayMs $P2pPublishDelayMs `
    -P2pDuplicatePublishes $P2pDuplicatePublishes `
    -RpcAdversarialRequestsPerNode $RpcRequests `
    -RpcAdversarialConcurrency $RpcConcurrency `
    -IncludeOversizedRpcAdversarial:$IncludeOversizedRpc `
    -BaseP2pPort $baseP2pPort `
    -BaseRpcPort $baseRpcPort `
    -BuildProfile $BuildProfile `
    -SkipBuild:$SkipBuild

& $adversarialSoak `
    -Minutes $soakMinutes `
    -Workers 1

Write-Host "Kanari long-run security campaign completed successfully."
