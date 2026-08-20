param(
    [ValidateRange(1, 168)]
    [int]$Hours = 4,

    # Optional finer-grained duration for CI and short validation campaigns.
    # When set, this takes precedence over Hours.
    [ValidateRange(0, 10080)]
    [int]$Minutes = 0,

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

    # Zero selects the exact safe minimum for the selected transaction count:
    # one mutable transfer coin and one distinct gas coin per transaction,
    # plus the reserve retained by the runner.
    [ValidateRange(0, 1000000)]
    [int]$FaucetCoinsPerSender = 0,

    [ValidateRange(1, 64)]
    [int]$FanoutBatchSize = 64,

    [ValidateRange(2, 1024)]
    [int]$CoinReserveBuffer = 32,

    [ValidateRange(30, 7200)]
    [int]$LaneTimeoutSec = 900,

    [ValidateRange(20, 7200)]
    [int]$FundingCommitTimeoutSec = 180,

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
    [int]$CrashDuringLoadRestartDelaySec = 1,

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

    [switch]$P2pReorderBestEffort,

    [ValidateRange(16, 65536)]
    [int]$P2pChannelCapacity = 8192,

    [ValidateRange(1, 4096)]
    [int]$MaxConcurrentSyncMessages = 512,

    [ValidateRange(1, 300)]
    [int]$RootSyncTimeoutSec = 300,

    # Each campaign iteration offsets the base by 100 and each run uses four
    # nodes spaced by ten. Keep callers in the valid TCP port range.
    [ValidateRange(1024, 65405)]
    [int]$BaseP2pPort = 23000,

    [ValidateRange(1024, 65405)]
    [int]$BaseRpcPort = 23001,

    [ValidateRange(0, 100000)]
    [int]$RpcAdversarialRequestsPerNode = 0,

    [ValidateRange(1, 512)]
    [int]$RpcAdversarialConcurrency = 32,

    [switch]$IncludeOversizedRpcAdversarial,

    [ValidateSet('release', 'debug')]
    [string]$BuildProfile = 'release',

    # JSON heartbeat written before and after every child run. A watchdog can
    # distinguish a normal completion from a terminated parent process.
    [string]$StatusPath,

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$minimumFaucetCoinsPerSender = (2 * $CountPerSender) + $CoinReserveBuffer
if ($FaucetCoinsPerSender -eq 0) {
    $FaucetCoinsPerSender = $minimumFaucetCoinsPerSender
} elseif ($FaucetCoinsPerSender -lt $minimumFaucetCoinsPerSender) {
    throw "FaucetCoinsPerSender=$FaucetCoinsPerSender is too low for CountPerSender=$CountPerSender and CoinReserveBuffer=$CoinReserveBuffer. Use $minimumFaucetCoinsPerSender or omit it to select the safe minimum."
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$chaosScript = Join-Path $repoRoot 'scripts\run-local-four-node-parallel-tx-chaos.ps1'
if (-not (Test-Path -LiteralPath $chaosScript)) {
    throw "Missing chaos runner: $chaosScript"
}

$duration = if ($Minutes -gt 0) {
    [TimeSpan]::FromMinutes($Minutes)
} else {
    [TimeSpan]::FromHours($Hours)
}
$deadline = (Get-Date).Add($duration)
$iteration = 0
$campaignStartedAt = Get-Date
$campaignState = 'starting'

# Each child run consumes four ports spaced by ten. Keep the per-iteration
# offset, but wrap it inside the range accepted by the child runner before a
# long campaign reaches the end of the TCP port space. The runner tears down
# its nodes before the next iteration, so reusing a port only happens after a
# clean child exit.
function Get-CampaignBasePort {
    param(
        [int]$InitialPort,
        [int]$Iteration
    )

    $minimumBasePort = 1024
    $maximumBasePort = 65405
    $span = $maximumBasePort - $minimumBasePort + 1
    $offset = $InitialPort - $minimumBasePort + (100 * ($Iteration - 1))
    return $minimumBasePort + ($offset % $span)
}

# Windows can reserve TCP ranges for Hyper-V, containers, or system services.
# A bind to such a port fails with WSAEACCES (10013), which is an environment
# failure rather than a useful blockchain recovery result.  Keep the campaign
# portable: on Windows skip excluded or already-listening port sets; on Unix
# only the listener check applies when the cmdlet is available.
function Get-ExcludedTcpPortRanges {
    if ($env:OS -ne 'Windows_NT') { return @() }

    try {
        $ranges = @()
        foreach ($line in (& netsh interface ipv4 show excludedportrange protocol=tcp 2>$null)) {
            if ($line -match '^\s*(\d+)\s+(\d+)\s+(?:\*)?\s*$') {
                $ranges += [pscustomobject]@{
                    Start = [int]$matches[1]
                    End = [int]$matches[2]
                }
            }
        }
        return $ranges
    } catch {
        Write-Warning "Unable to read Windows excluded TCP port ranges: $($_.Exception.Message)"
        return @()
    }
}

$excludedTcpPortRanges = Get-ExcludedTcpPortRanges

function Test-CampaignPortSetAvailable {
    param(
        [int]$P2pBasePort,
        [int]$RpcBasePort
    )

    $ports = @(0, 10, 20, 30 | ForEach-Object { $P2pBasePort + $_ }) +
        @(0, 10, 20, 30 | ForEach-Object { $RpcBasePort + $_ })
    if (($ports | Select-Object -Unique).Count -ne $ports.Count) { return $false }

    $getNetTcpConnection = Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue
    foreach ($port in $ports) {
        if ($port -lt 1024 -or $port -gt 65535) { return $false }
        if ($excludedTcpPortRanges | Where-Object { $port -ge $_.Start -and $port -le $_.End }) {
            return $false
        }
        if ($null -ne $getNetTcpConnection) {
            $listener = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
            if ($null -ne $listener) { return $false }
        }
    }
    return $true
}

function Get-CampaignPortPair {
    param([int]$Iteration)

    # Advance by the normal 100-port iteration stride until all eight node
    # listener ports are usable.  The finite bound avoids an accidental loop
    # forever on a host with unusual networking policy.
    for ($probe = 0; $probe -lt 640; $probe++) {
        $candidateIteration = $Iteration + $probe
        $p2p = Get-CampaignBasePort -InitialPort $BaseP2pPort -Iteration $candidateIteration
        $rpc = Get-CampaignBasePort -InitialPort $BaseRpcPort -Iteration $candidateIteration
        if (Test-CampaignPortSetAvailable -P2pBasePort $p2p -RpcBasePort $rpc) {
            if ($probe -gt 0) {
                Write-Host "Skipping unavailable campaign port set; using p2p=$p2p rpc=$rpc instead."
            }
            return [pscustomobject]@{ P2p = $p2p; Rpc = $rpc }
        }
    }
    throw 'Unable to find a usable four-node TCP port set after 640 attempts.'
}
$lastExitCode = $null
$terminalError = $null

if ([string]::IsNullOrWhiteSpace($StatusPath)) {
    $StatusPath = Join-Path $repoRoot '.codex-runlogs\chaos-campaign-status.json'
}
$statusDirectory = Split-Path -Parent $StatusPath
if (-not [string]::IsNullOrWhiteSpace($statusDirectory)) {
    New-Item -ItemType Directory -Force -Path $statusDirectory | Out-Null
}

function Write-CampaignStatus {
    param([Parameter(Mandatory = $true)][string]$Phase)

    $status = [ordered]@{
        version = 1
        pid = $PID
        state = $campaignState
        phase = $Phase
        started_at = $campaignStartedAt.ToString('o')
        deadline = $deadline.ToString('o')
        heartbeat_at = (Get-Date).ToString('o')
        iteration = $iteration
        last_exit_code = $lastExitCode
        terminal_error = $terminalError
    }
    $temporaryPath = "$StatusPath.tmp"
    $status | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $temporaryPath -Encoding utf8
    Move-Item -LiteralPath $temporaryPath -Destination $StatusPath -Force
}

function Invoke-ChaosRunner {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    # A script invoked with `&` does not reliably set $LASTEXITCODE.  Run it
    # through a child PowerShell process instead, so campaign success/failure
    # is derived from an explicit process exit code.
    $commandArguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $chaosScript) + $Arguments
    $process = Start-Process `
        -FilePath 'powershell.exe' `
        -ArgumentList $commandArguments `
        -WorkingDirectory $repoRoot `
        -NoNewWindow `
        -Wait `
        -PassThru
    return [int]$process.ExitCode
}

Write-Host "Kanari network chaos campaign"
Write-Host "  duration=$duration deadline=$deadline"
Write-Host "  senders=$($Senders -join ', ')"
Write-Host "  count_per_sender=$CountPerSender chaos_rounds=$ChaosRoundsPerIteration"
Write-Host "  crash_during_load_rounds=$CrashDuringLoadRoundsPerIteration pattern=$CrashDuringLoadPattern recovery_audit_rounds=$RecoveryAuditRoundsPerIteration"
Write-Host "  p2p_delay_ms=$P2pPublishDelayMs duplicate_publishes=$P2pDuplicatePublishes"
Write-Host "  p2p_best_effort_reorder=$P2pReorderBestEffort"
Write-Host "  rpc_adversarial_requests_per_node=$RpcAdversarialRequestsPerNode concurrency=$RpcAdversarialConcurrency oversized=$IncludeOversizedRpcAdversarial"
Write-Host "  status_path=$StatusPath"

try {
    $campaignState = 'running'
    Write-CampaignStatus -Phase 'campaign-started'

    while ((Get-Date) -lt $deadline) {
        $iteration += 1
        $portPair = Get-CampaignPortPair -Iteration $iteration
        $baseP2pPort = $portPair.P2p
        $baseRpcPort = $portPair.Rpc
        Write-Host "[$(Get-Date -Format o)] chaos iteration $iteration base_p2p=$baseP2pPort base_rpc=$baseRpcPort"
        Write-CampaignStatus -Phase 'starting-chaos-runner'

        $runnerArguments = @(
            '-Senders', $Senders,
            '-Recipient', $Recipient,
            '-Password', $Password,
            '-KeystorePath', $KeystorePath,
            '-FundSenders',
            '-FaucetAmount', '1',
            '-FaucetCoinsPerSender', $FaucetCoinsPerSender,
            '-FanoutBatchSize', $FanoutBatchSize,
            '-CoinReserveBuffer', $CoinReserveBuffer,
            '-LaneTimeoutSec', $LaneTimeoutSec,
            '-FundingCommitTimeoutSec', $FundingCommitTimeoutSec,
            '-P2pChannelCapacity', $P2pChannelCapacity,
            '-MaxConcurrentSyncMessages', $MaxConcurrentSyncMessages,
            '-P2pPublishDelayMs', $P2pPublishDelayMs,
            '-P2pDuplicatePublishes', $P2pDuplicatePublishes,
            '-CountPerSender', $CountPerSender,
            '-Amount', $Amount,
            '-ChaosRounds', $ChaosRoundsPerIteration,
            '-CrashDuringLoadRounds', $CrashDuringLoadRoundsPerIteration,
            '-CrashDuringLoadPattern', $CrashDuringLoadPattern,
            '-CrashDuringLoadFirstDelaySec', $CrashDuringLoadFirstDelaySec,
            '-CrashDuringLoadRestartDelaySec', $CrashDuringLoadRestartDelaySec,
            '-RecoveryAuditRounds', $RecoveryAuditRoundsPerIteration,
            '-ProfileIntervalSec', $ProfileIntervalSec,
            '-PerfSampleHz', $PerfSampleHz,
            '-PerfDurationSec', $PerfDurationSec,
            '-RootSyncTimeoutSec', $RootSyncTimeoutSec,
            '-RpcAdversarialRequests', $RpcAdversarialRequestsPerNode,
            '-RpcAdversarialConcurrency', $RpcAdversarialConcurrency,
            '-BaseP2pPort', $baseP2pPort,
            '-BaseRpcPort', $baseRpcPort,
            '-BuildProfile', $BuildProfile
        )
        if ($P2pReorderBestEffort) { $runnerArguments += '-P2pReorderBestEffort' }
        if ($AutoCoinFanout) { $runnerArguments += '-AutoCoinFanout' }
        if ($StartLinuxPerfRecorders) { $runnerArguments += '-StartLinuxPerfRecorders' }
        if ($IncludeOversizedRpcAdversarial) { $runnerArguments += '-IncludeOversizedRpcAdversarial' }
        if ($SkipBuild -or $iteration -gt 1) { $runnerArguments += '-SkipBuild' }
        $lastExitCode = Invoke-ChaosRunner -Arguments $runnerArguments
        if ($lastExitCode -ne 0) {
            throw "Chaos runner exited with code $lastExitCode in iteration $iteration"
        }
        Write-CampaignStatus -Phase 'iteration-complete'
    }

    $campaignState = 'completed'
    Write-CampaignStatus -Phase 'campaign-complete'
    Write-Host "Kanari network chaos campaign completed: $iteration iteration(s) without failure."
} catch {
    $campaignState = 'failed'
    $terminalError = $_.Exception.Message
    Write-CampaignStatus -Phase 'campaign-failed'
    Write-Error "Kanari network chaos campaign failed in iteration ${iteration}: $terminalError"
    exit 1
}
