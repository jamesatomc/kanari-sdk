param(
    [ValidateRange(1, 1000000)]
    [int]$CountPerSender = 5000,

    [ValidateRange(0.000000001, 1000000000)]
    [double]$Amount = 0.000000001,

    [ValidateRange(0, 1000)]
    [int]$TempWalletCount = 0,

    [ValidateRange(1, 1000000)]
    [int]$FaucetCoinsPerSender = 16,

    [switch]$AutoCoinFanout,

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
    [int]$RecoveryAuditRounds = 0,

    [ValidateRange(0, 3600)]
    [int]$ProfileIntervalSec = 0,

    [switch]$StartLinuxPerfRecorders,

    [ValidateRange(1, 10000)]
    [int]$PerfSampleHz = 99,

    [ValidateRange(1, 3600)]
    [int]$PerfDurationSec = 30,

    [switch]$SelfRecipient,

    [ValidateRange(1, 300)]
    [int]$RootSyncTimeoutSec = 300,

    [int]$BaseP2pPort = 21400,

    [int]$BaseRpcPort = 21401,

    [string]$KeystorePath = 'C:\Users\move-love\.kanari\kanari_config\kanari.keystore',

    [ValidateSet('release', 'debug')]
    [string]$BuildProfile = 'release',

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($env:KANARI_LOAD_PASSWORD)) {
    throw 'KANARI_LOAD_PASSWORD must be set before starting the load run.'
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$loadScript = Join-Path $repoRoot 'scripts\run-local-four-node-parallel-tx-chaos.ps1'

& $loadScript `
    -Senders '0x3ba63b92aac5f2bff87e580e820b61faf1c5fe9ae12f0bc8addd931a340b3146,0x3141a487d7a5382bb435c0ad39a6060067765e60e45b50953a0050bcf24b03a3' `
    -Recipient '0x3141a487d7a5382bb435c0ad39a6060067765e60e45b50953a0050bcf24b03a3' `
    -SelfRecipient:$SelfRecipient `
    -Password $env:KANARI_LOAD_PASSWORD `
    -KeystorePath $KeystorePath `
    -TempWalletCount $TempWalletCount `
    -FundSenders `
    -FaucetAmount 1 `
    -FaucetCoinsPerSender $FaucetCoinsPerSender `
    -AutoCoinFanout:$AutoCoinFanout `
    -P2pChannelCapacity $P2pChannelCapacity `
    -MaxConcurrentSyncMessages $MaxConcurrentSyncMessages `
    -CountPerSender $CountPerSender `
    -Amount $Amount `
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
    -BaseP2pPort $BaseP2pPort `
    -BaseRpcPort $BaseRpcPort `
    -BuildProfile $BuildProfile `
    -SkipBuild:$SkipBuild
