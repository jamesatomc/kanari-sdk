param(
    [ValidateRange(1, 168)]
    [int]$Hours = 4,

    [Parameter(Mandatory = $true)]
    [string[]]$Senders,

    [Parameter(Mandatory = $true)]
    [string]$Recipient,

    [Parameter(Mandatory = $true)]
    [string]$Password,

    [string]$KeystorePath = "$env:USERPROFILE\.kanari\kanari_config\kanari.keystore",

    [ValidateRange(1, 1000000)]
    [int]$CountPerSender = 250,

    [ValidateRange(0.000000001, 1000000000)]
    [double]$Amount = 0.000000001,

    [ValidateRange(1, 1000000)]
    [int]$FaucetCoinsPerSender = 16,

    [switch]$AutoCoinFanout,

    [ValidateRange(0, 20)]
    [int]$ChaosRoundsPerIteration = 4,

    [ValidateRange(0, 20)]
    [int]$CrashDuringLoadRoundsPerIteration = 0,

    [ValidateSet('follower', 'leader', 'two-node', 'round-robin', 'client-ingress')]
    [string]$CrashDuringLoadPattern = 'follower',

    [ValidateRange(1, 3600)]
    [int]$CrashDuringLoadFirstDelaySec = 10,

    [ValidateRange(1, 3600)]
    [int]$CrashDuringLoadRestartDelaySec = 0,

    [ValidateRange(0, 20)]
    [int]$RecoveryAuditRoundsPerIteration = 1,

    [ValidateRange(0, 3600)]
    [int]$ProfileIntervalSec = 15,

    [switch]$StartLinuxPerfRecorders,

    [ValidateRange(1, 10000)]
    [int]$PerfSampleHz = 99,

    [ValidateRange(1, 3600)]
    [int]$PerfDurationSec = 30,

    [ValidateRange(0, 30000)]
    [int]$P2pPublishDelayMs = 250,

    [ValidateRange(0, 8)]
    [int]$P2pDuplicatePublishes = 2,

    [ValidateRange(16, 65536)]
    [int]$P2pChannelCapacity = 8192,

    [ValidateRange(1, 4096)]
    [int]$MaxConcurrentSyncMessages = 512,

    [ValidateRange(1, 300)]
    [int]$RootSyncTimeoutSec = 300,

    [ValidateSet('release', 'debug')]
    [string]$BuildProfile = 'release',

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$chaosScript = Join-Path $repoRoot 'scripts\run-local-four-node-parallel-tx-chaos.ps1'
if (-not (Test-Path -LiteralPath $chaosScript)) {
    throw "Missing chaos runner: $chaosScript"
}

$deadline = (Get-Date).AddHours($Hours)
$iteration = 0

Write-Host "Kanari network chaos campaign"
Write-Host "  deadline=$deadline"
Write-Host "  senders=$($Senders -join ', ')"
Write-Host "  count_per_sender=$CountPerSender chaos_rounds=$ChaosRoundsPerIteration"
Write-Host "  crash_during_load_rounds=$CrashDuringLoadRoundsPerIteration pattern=$CrashDuringLoadPattern recovery_audit_rounds=$RecoveryAuditRoundsPerIteration"
Write-Host "  p2p_delay_ms=$P2pPublishDelayMs duplicate_publishes=$P2pDuplicatePublishes"

while ((Get-Date) -lt $deadline) {
    $iteration += 1
    $baseP2pPort = 23000 + (($iteration - 1) * 100)
    $baseRpcPort = $baseP2pPort + 1
    Write-Host "[$(Get-Date -Format o)] chaos iteration $iteration base_p2p=$baseP2pPort base_rpc=$baseRpcPort"

    & $chaosScript `
        -Senders $Senders `
        -Recipient $Recipient `
        -Password $Password `
        -KeystorePath $KeystorePath `
        -FundSenders `
        -FaucetAmount 1 `
        -FaucetCoinsPerSender $FaucetCoinsPerSender `
        -AutoCoinFanout:$AutoCoinFanout `
        -P2pChannelCapacity $P2pChannelCapacity `
        -MaxConcurrentSyncMessages $MaxConcurrentSyncMessages `
        -P2pPublishDelayMs $P2pPublishDelayMs `
        -P2pDuplicatePublishes $P2pDuplicatePublishes `
        -CountPerSender $CountPerSender `
        -Amount $Amount `
        -ChaosRounds $ChaosRoundsPerIteration `
        -CrashDuringLoadRounds $CrashDuringLoadRoundsPerIteration `
        -CrashDuringLoadPattern $CrashDuringLoadPattern `
        -CrashDuringLoadFirstDelaySec $CrashDuringLoadFirstDelaySec `
        -CrashDuringLoadRestartDelaySec $CrashDuringLoadRestartDelaySec `
        -RecoveryAuditRounds $RecoveryAuditRoundsPerIteration `
        -ProfileIntervalSec $ProfileIntervalSec `
        -StartLinuxPerfRecorders:$StartLinuxPerfRecorders `
        -PerfSampleHz $PerfSampleHz `
        -PerfDurationSec $PerfDurationSec `
        -RootSyncTimeoutSec $RootSyncTimeoutSec `
        -BaseP2pPort $baseP2pPort `
        -BaseRpcPort $baseRpcPort `
        -BuildProfile $BuildProfile `
        -SkipBuild:($SkipBuild -or $iteration -gt 1)
}

Write-Host "Kanari network chaos campaign completed: $iteration iteration(s) without failure."
