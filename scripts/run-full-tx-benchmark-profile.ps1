param(
    [int[]]$TxTargets = @(1000, 10000),

    [ValidateRange(2, 1000)]
    [int]$Wallets = 60,

    [ValidateRange(1, 64)]
    [int]$FanoutBatchSize = 64,

    [ValidateRange(2, 1024)]
    [int]$CoinReserveBuffer = 32,

    [Parameter(Mandatory = $true)]
    [string]$Password,

    [string]$KeystorePath = "$env:USERPROFILE\.kanari\kanari_config\kanari.keystore",

    [string]$Recipient = '0x3141a487d7a5382bb435c0ad39a6060067765e60e45b50953a0050bcf24b03a3',

    [string[]]$BaseSenders = @(
        '0x3ba63b92aac5f2bff87e580e820b61faf1c5fe9ae12f0bc8addd931a340b3146',
        '0x3141a487d7a5382bb435c0ad39a6060067765e60e45b50953a0050bcf24b03a3'
    ),

    [ValidateRange(0.000000001, 1000000000)]
    [double]$Amount = 0.000000001,

    [ValidateRange(16, 65536)]
    [int]$P2pChannelCapacity = 8192,

    [ValidateRange(1, 4096)]
    [int]$MaxConcurrentSyncMessages = 512,

    [ValidateRange(0, 20)]
    [int]$ChaosRounds = 0,

    [ValidateRange(0, 20)]
    [int]$CrashDuringLoadRounds = 0,

    [ValidateSet('follower', 'leader', 'two-node', 'round-robin', 'client-ingress')]
    [string]$CrashDuringLoadPattern = 'follower',

    [ValidateRange(1, 3600)]
    [int]$CrashDuringLoadFirstDelaySec = 10,

    [ValidateRange(1, 3600)]
    [int]$CrashDuringLoadRestartDelaySec = 0,

    [ValidateRange(0, 20)]
    [int]$RecoveryAuditRounds = 1,

    [ValidateRange(0, 3600)]
    [int]$ProfileIntervalSec = 5,

    [switch]$StartLinuxPerfRecorders,

    [ValidateRange(1, 10000)]
    [int]$PerfSampleHz = 99,

    [ValidateRange(1, 3600)]
    [int]$PerfDurationSec = 30,

    [ValidateRange(1, 300)]
    [int]$RootSyncTimeoutSec = 300,

    [ValidateSet('release', 'debug')]
    [string]$BuildProfile = 'release',

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$loadScript = Join-Path $repoRoot 'scripts\run-local-four-node-parallel-tx-chaos.ps1'
if (-not (Test-Path -LiteralPath $loadScript)) {
    throw "Missing load runner: $loadScript"
}

if ($Wallets -lt $BaseSenders.Count) {
    throw "Wallets=$Wallets is smaller than BaseSenders=$($BaseSenders.Count)"
}

$summaryPath = Join-Path $repoRoot ".codex-runlogs\full-tx-benchmark-profile-$(Get-Date -Format 'yyyyMMdd-HHmmss').csv"
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $summaryPath) | Out-Null
'timestamp,target_txs,wallets,count_per_sender,requested_txs,base_p2p_port,base_rpc_port,status' |
    Set-Content -LiteralPath $summaryPath

Write-Host "Kanari full tx benchmark profile"
Write-Host "  targets=$($TxTargets -join ', ') wallets=$Wallets"
Write-Host "  profile_interval_sec=$ProfileIntervalSec recovery_audit_rounds=$RecoveryAuditRounds"
Write-Host "  crash_rounds=$CrashDuringLoadRounds crash_pattern=$CrashDuringLoadPattern"
Write-Host "  summary=$summaryPath"

$iteration = 0
foreach ($target in $TxTargets) {
    if ($target -lt 1) {
        throw "TxTargets must contain positive values"
    }
    $iteration += 1
    $countPerSender = [int][Math]::Ceiling($target / [double]$Wallets)
    $requestedTxs = $countPerSender * $Wallets
    $tempWalletCount = $Wallets - $BaseSenders.Count
    $baseP2pPort = 25000 + (($iteration - 1) * 100)
    $baseRpcPort = $baseP2pPort + 1

    Write-Host "[$(Get-Date -Format o)] benchmark target=$target requested=$requestedTxs wallets=$Wallets count_per_sender=$countPerSender"
    try {
        & $loadScript `
            -Senders $BaseSenders `
            -Recipient $Recipient `
            -Password $Password `
            -KeystorePath $KeystorePath `
            -TempWalletCount $tempWalletCount `
            -CountPerSender $countPerSender `
            -Amount $Amount `
            -FundSenders `
            -AutoCoinFanout `
            -FanoutBatchSize $FanoutBatchSize `
            -CoinReserveBuffer $CoinReserveBuffer `
            -P2pChannelCapacity $P2pChannelCapacity `
            -MaxConcurrentSyncMessages $MaxConcurrentSyncMessages `
            -ChaosRounds $ChaosRounds `
            -CrashDuringLoadRounds $CrashDuringLoadRounds `
            -CrashDuringLoadPattern $CrashDuringLoadPattern `
            -CrashDuringLoadFirstDelaySec $CrashDuringLoadFirstDelaySec `
            -CrashDuringLoadRestartDelaySec $CrashDuringLoadRestartDelaySec `
            -RecoveryAuditRounds $RecoveryAuditRounds `
            -ProfileIntervalSec $ProfileIntervalSec `
            -StartLinuxPerfRecorders:$StartLinuxPerfRecorders `
            -PerfSampleHz $PerfSampleHz `
            -PerfDurationSec $PerfDurationSec `
            -RootSyncTimeoutSec $RootSyncTimeoutSec `
            -BaseP2pPort $baseP2pPort `
            -BaseRpcPort $baseRpcPort `
            -BuildProfile $BuildProfile `
            -SkipBuild:($SkipBuild -or $iteration -gt 1)
        "$(Get-Date -Format o),$target,$Wallets,$countPerSender,$requestedTxs,$baseP2pPort,$baseRpcPort,ok" |
            Add-Content -LiteralPath $summaryPath
    } catch {
        "$(Get-Date -Format o),$target,$Wallets,$countPerSender,$requestedTxs,$baseP2pPort,$baseRpcPort,failed" |
            Add-Content -LiteralPath $summaryPath
        throw
    }
}

Write-Host "Kanari full tx benchmark profile complete: $summaryPath"
