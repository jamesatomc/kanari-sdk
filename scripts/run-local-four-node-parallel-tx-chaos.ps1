param(
    [Parameter(Mandatory = $true)]
    [string[]]$Senders,

    [Parameter(Mandatory = $true)]
    [string]$Recipient,

    [switch]$SelfRecipient,

    [string]$Password = $env:KANARI_LOAD_PASSWORD,

    [ValidateRange(1, 1000000)]
    [int]$CountPerSender = 100,

    [ValidateRange(0.000000001, 1000000000)]
    [double]$Amount = 0.000000001,

    [string]$KeystorePath = $env:KANARI_KEYSTORE_PATH,

    [ValidateRange(0, 1000)]
    [int]$TempWalletCount = 0,

    [ValidateRange(0, 1000000000)]
    [double]$FaucetAmount = 0.0001,

    [ValidateRange(1, 1000000)]
    [int]$FaucetCoinsPerSender = 2,

    [switch]$AutoCoinFanout,

    [ValidateRange(16, 65536)]
    [int]$P2pChannelCapacity = 1024,

    [ValidateRange(1, 4096)]
    [int]$MaxConcurrentSyncMessages = 128,

    [ValidateRange(0, 30000)]
    [int]$P2pPublishDelayMs = 0,

    [ValidateRange(0, 8)]
    [int]$P2pDuplicatePublishes = 0,

    [switch]$FundSenders,

    [ValidateRange(0, 20)]
    [int]$ChaosRounds = 2,

    [ValidateRange(0, 20)]
    [int]$CrashDuringLoadRounds = 0,

    [ValidateSet('follower', 'leader', 'two-node', 'round-robin', 'client-ingress')]
    [string]$CrashDuringLoadPattern = 'follower',

    [int[]]$CrashDuringLoadNodes = @(),

    [ValidateRange(1, 3600)]
    [int]$CrashDuringLoadFirstDelaySec = 10,

    [ValidateRange(1, 3600)]
    [int]$CrashDuringLoadRestartDelaySec = 0,

    [ValidateRange(0, 20)]
    [int]$RecoveryAuditRounds = 0,

    [ValidateRange(0, 300)]
    [int]$RecoveryRestartDelaySec = 3,

    [ValidateRange(0, 3600)]
    [int]$ProfileIntervalSec = 0,

    [switch]$StartLinuxPerfRecorders,

    [ValidateRange(1, 10000)]
    [int]$PerfSampleHz = 99,

    [ValidateRange(1, 3600)]
    [int]$PerfDurationSec = 30,

    [int[]]$ProtectedRpcNodes = @(),

    [ValidateRange(1, 300)]
    [int]$RootSyncTimeoutSec = 120,

    [int]$BaseP2pPort = 19500,

    [int]$BaseRpcPort = 19501,

    [string]$P2pNamespace = '',

    [ValidateSet('release', 'debug')]
    [string]$BuildProfile = 'release',

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$stressLaneScript = Join-Path $repoRoot 'scripts\run-kanari-stress-lane.ps1'
$targetProfileDir = if ($BuildProfile -eq 'release') { 'release' } else { 'debug' }
$nodeExe = Join-Path $repoRoot "target\$targetProfileDir\kanari-node.exe"
$kanariExe = Join-Path $repoRoot "target\$targetProfileDir\kanari.exe"
$runId = Get-Date -Format 'yyyyMMdd-HHmmss'
$runRoot = Join-Path $repoRoot ".codex-runlogs\four-node-parallel-tx-chaos-$runId"
if ([string]::IsNullOrWhiteSpace($P2pNamespace)) {
    $P2pNamespace = "parallel-chaos-$runId"
}

New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

if ([string]::IsNullOrWhiteSpace($Password)) {
    throw "Set KANARI_LOAD_PASSWORD for the temporary load-test keystore."
}

if ([string]::IsNullOrWhiteSpace($KeystorePath)) {
    throw "KeystorePath is required. Pass -KeystorePath or set KANARI_KEYSTORE_PATH."
}
if (-not (Test-Path -LiteralPath $KeystorePath)) {
    throw "Keystore not found: $KeystorePath"
}

$localKeystorePath = Join-Path $runRoot 'kanari.keystore'
Copy-Item -LiteralPath $KeystorePath -Destination $localKeystorePath -Force
if (-not (Test-Path -LiteralPath $localKeystorePath)) {
    throw "Failed to copy keystore into run directory."
}
$KeystorePath = $localKeystorePath

$Senders = @(
    foreach ($sender in $Senders) {
        foreach ($part in ($sender -split ',')) {
            $trimmed = $part.Trim()
            if (-not [string]::IsNullOrWhiteSpace($trimmed)) {
                $trimmed
            }
        }
    }
)
if ($Senders.Count -eq 0) {
    throw "At least one sender is required."
}
if ($FundSenders -and $CountPerSender -gt 1) {
    $requiredCoinFanout = $CountPerSender + 1
    if ($AutoCoinFanout -and $FaucetCoinsPerSender -lt $requiredCoinFanout) {
        Write-Host "AutoCoinFanout adjusted FaucetCoinsPerSender from $FaucetCoinsPerSender to $requiredCoinFanout for CountPerSender=$CountPerSender"
        $FaucetCoinsPerSender = $requiredCoinFanout
    }
    if ($FaucetCoinsPerSender -lt $requiredCoinFanout) {
        throw "FaucetCoinsPerSender=$FaucetCoinsPerSender is too low for CountPerSender=$CountPerSender. Native transfers need one mutable transfer coin plus a separate gas coin; use -FaucetCoinsPerSender $requiredCoinFanout or higher to avoid object-ref waits and false node-stall failures."
    }
}

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        if ($BuildProfile -eq 'release') {
            cargo build -p kanari-node -p kanari --release
        } else {
            cargo build -p kanari-node -p kanari
        }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path $nodeExe)) { throw "$BuildProfile kanari-node not found at $nodeExe" }
if (-not (Test-Path $kanariExe)) { throw "$BuildProfile kanari not found at $kanariExe" }
if (-not (Test-Path $stressLaneScript)) { throw "stress lane wrapper not found at $stressLaneScript" }

if ($TempWalletCount -gt 0) {
    Write-Host "Creating $TempWalletCount temporary sender wallet(s) in copied run keystore"
    $previousKeystorePath = $env:KANARI_KEYSTORE_PATH
    $env:KANARI_KEYSTORE_PATH = $KeystorePath
    try {
        for ($i = 1; $i -le $TempWalletCount; $i++) {
            $createLog = Join-Path $runRoot "temp-wallet-$i.create.out.log"
            $createErr = Join-Path $runRoot "temp-wallet-$i.create.err.log"
            $createProcess = Start-Process `
                -FilePath $kanariExe `
                -ArgumentList @('keytool', 'create', '--password', $Password, '--curve', 'ed25519') `
                -WorkingDirectory $repoRoot `
                -RedirectStandardOutput $createLog `
                -RedirectStandardError $createErr `
                -WindowStyle Hidden `
                -Wait `
                -PassThru
            if ($createProcess.ExitCode -ne 0) {
                Get-Content $createLog -Tail 80
                Get-Content $createErr -Tail 80
                throw "temporary wallet creation failed for index $i; logs: $createLog $createErr"
            }

            $createText = ''
            if (Test-Path -LiteralPath $createLog) { $createText += (Get-Content -Raw -LiteralPath $createLog) }
            if (Test-Path -LiteralPath $createErr) { $createText += "`n" + (Get-Content -Raw -LiteralPath $createErr) }
            $match = [regex]::Match($createText, 'Created wallet:\s*(0x[0-9a-fA-F]{64})')
            if (-not $match.Success) {
                throw "unable to parse temporary wallet address for index $i; logs: $createLog $createErr"
            }
            $Senders += $match.Groups[1].Value.ToLowerInvariant()
        }
    } finally {
        if ($null -eq $previousKeystorePath) {
            Remove-Item Env:\KANARI_KEYSTORE_PATH -ErrorAction SilentlyContinue
        } else {
            $env:KANARI_KEYSTORE_PATH = $previousKeystorePath
        }
    }
    $Senders = @($Senders | Select-Object -Unique)
    Write-Host "Total sender lanes after temp wallet creation: $($Senders.Count)"
}

function Invoke-KanariRpc {
    param(
        [string]$Url,
        [string]$Method,
        [object]$Params = @{},
        [int]$Id = 1,
        [int]$TimeoutSec = 5
    )

    $body = @{
        jsonrpc = '2.0'
        id = $Id
        method = $Method
        params = $Params
    } | ConvertTo-Json -Depth 32 -Compress

    return Invoke-RestMethod -Uri $Url -Method Post -ContentType 'application/json' -Body $body -TimeoutSec $TimeoutSec
}

function Start-KanariNode {
    param([int]$Index, [string]$OutSuffix = '')

    $offset = ($Index - 1) * 10
    $p2pPort = $BaseP2pPort + $offset
    $rpcPort = $BaseRpcPort + $offset
    $dataDir = Join-Path $runRoot "node$Index-db"
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null

    $args = @(
        'start',
        '--network', 'devnet',
        '--p2p-port', $p2pPort,
        '--rpc-port', $rpcPort,
        '--rpc-host', '127.0.0.1',
        '--data-dir', $dataDir,
        '--authority-id', "0x$Index",
        '--authorities', '0x1,0x2,0x3,0x4',
        '--genesis', $genesis,
        '--consensus-private-key-file', (Join-Path $keysDir "node$Index-consensus-private-key.key"),
        '--consensus-public-keys', (Join-Path $keysDir 'consensus-public-keys.json')
    )
    if ($Index -ne 1) {
        $args += @('--bootstrap', "/ip4/127.0.0.1/tcp/$BaseP2pPort")
    }

    $suffix = if ($OutSuffix) { ".$OutSuffix" } else { '' }
    $previousNamespace = $env:KANARI_P2P_NAMESPACE
    $previousP2pCapacity = $env:KANARI_P2P_CHANNEL_CAPACITY
    $previousSyncConcurrency = $env:KANARI_MAX_CONCURRENT_SYNC_MESSAGES
    $previousChaosDelay = $env:KANARI_CHAOS_P2P_PUBLISH_DELAY_MS
    $previousChaosDuplicates = $env:KANARI_CHAOS_P2P_DUPLICATE_PUBLISHES
    $env:KANARI_P2P_NAMESPACE = $P2pNamespace
    $env:KANARI_P2P_CHANNEL_CAPACITY = [string]$P2pChannelCapacity
    $env:KANARI_MAX_CONCURRENT_SYNC_MESSAGES = [string]$MaxConcurrentSyncMessages
    $env:KANARI_CHAOS_P2P_PUBLISH_DELAY_MS = [string]$P2pPublishDelayMs
    $env:KANARI_CHAOS_P2P_DUPLICATE_PUBLISHES = [string]$P2pDuplicatePublishes
    try {
        return Start-Process `
            -FilePath $nodeExe `
            -ArgumentList $args `
            -WorkingDirectory $repoRoot `
            -RedirectStandardOutput (Join-Path $runRoot "node$Index$suffix.out.log") `
            -RedirectStandardError (Join-Path $runRoot "node$Index$suffix.err.log") `
            -WindowStyle Hidden `
            -PassThru
    } finally {
        if ($null -eq $previousNamespace) {
            Remove-Item Env:\KANARI_P2P_NAMESPACE -ErrorAction SilentlyContinue
        } else {
            $env:KANARI_P2P_NAMESPACE = $previousNamespace
        }
        if ($null -eq $previousP2pCapacity) {
            Remove-Item Env:\KANARI_P2P_CHANNEL_CAPACITY -ErrorAction SilentlyContinue
        } else {
            $env:KANARI_P2P_CHANNEL_CAPACITY = $previousP2pCapacity
        }
        if ($null -eq $previousSyncConcurrency) {
            Remove-Item Env:\KANARI_MAX_CONCURRENT_SYNC_MESSAGES -ErrorAction SilentlyContinue
        } else {
            $env:KANARI_MAX_CONCURRENT_SYNC_MESSAGES = $previousSyncConcurrency
        }
        if ($null -eq $previousChaosDelay) {
            Remove-Item Env:\KANARI_CHAOS_P2P_PUBLISH_DELAY_MS -ErrorAction SilentlyContinue
        } else {
            $env:KANARI_CHAOS_P2P_PUBLISH_DELAY_MS = $previousChaosDelay
        }
        if ($null -eq $previousChaosDuplicates) {
            Remove-Item Env:\KANARI_CHAOS_P2P_DUPLICATE_PUBLISHES -ErrorAction SilentlyContinue
        } else {
            $env:KANARI_CHAOS_P2P_DUPLICATE_PUBLISHES = $previousChaosDuplicates
        }
    }
}

function Wait-RpcReady {
    param([string[]]$Urls, [int]$Attempts = 120)

    $ready = @{}
    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        foreach ($url in $Urls) {
            if ($ready[$url]) { continue }
            try {
                $health = Invoke-KanariRpc -Url $url -Method 'kanari_health' -Id 1 -TimeoutSec 2
                if ($health.result) {
                    $ready[$url] = $true
                    Write-Host "READY $url"
                }
            } catch {
            }
        }
        if ($ready.Count -eq $Urls.Count) { return }
        Start-Sleep -Seconds 1
    }
    throw "only $($ready.Count)/$($Urls.Count) RPC endpoints became ready; logs: $runRoot"
}

function Wait-StatsConverged {
    param([string[]]$Urls, [int]$TimeoutSec = $RootSyncTimeoutSec)

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $roots = New-Object System.Collections.Generic.HashSet[string]
        $heights = New-Object System.Collections.Generic.HashSet[string]
        $txs = New-Object System.Collections.Generic.HashSet[string]
        $latest = @{}
        $readable = 0

        foreach ($url in $Urls) {
            try {
                $stats = (Invoke-KanariRpc -Url $url -Method 'kanari_getStats' -Id 2 -TimeoutSec 3).result
            } catch {
                continue
            }
            if (-not $stats) { continue }
            $readable += 1
            $latest[$url] = $stats
            [void]$roots.Add([string]$stats.state_root)
            [void]$heights.Add([string]$stats.height)
            [void]$txs.Add([string]$stats.total_transactions)
        }

        if ($readable -eq $Urls.Count -and $roots.Count -eq 1 -and $heights.Count -eq 1 -and $txs.Count -eq 1) {
            foreach ($url in $Urls) {
                $stats = $latest[$url]
                Write-Host "SYNCED $url height=$($stats.height) txs=$($stats.total_transactions) root=$($stats.state_root)"
            }
            return
        }

        Start-Sleep -Seconds 1
    }

    throw "node stats did not converge within ${TimeoutSec}s; logs: $runRoot"
}

function Show-Stats {
    param([string[]]$Urls)

    foreach ($url in $Urls) {
        try {
            $stats = (Invoke-KanariRpc -Url $url -Method 'kanari_getStats' -Id 3 -TimeoutSec 3).result
            Write-Host "STATS $url height=$($stats.height) txs=$($stats.total_transactions) pending=$($stats.pending_transactions) root=$($stats.state_root)"
        } catch {
            Write-Host "STATS $url OFFLINE $($_.Exception.Message)"
        }
    }
}

$keysDir = Join-Path $runRoot 'consensus-keys'
$genesis = Join-Path $runRoot 'devnet-genesis.json'
$sourceData = Join-Path $runRoot 'node1-db'
New-Item -ItemType Directory -Force -Path $sourceData | Out-Null

Write-Host "Preparing temporary four-node parallel tx chaos devnet under $runRoot"
Write-Host "Using isolated P2P namespace: $P2pNamespace"
Write-Host "P2P channel capacity: $P2pChannelCapacity"
Write-Host "Max concurrent sync messages: $MaxConcurrentSyncMessages"
Write-Host "Chaos P2P publish delay: ${P2pPublishDelayMs}ms"
Write-Host "Chaos duplicate P2P publishes per message: $P2pDuplicatePublishes"
$profilePath = Join-Path $runRoot 'profile-samples.csv'
$profileSummaryPath = Join-Path $runRoot 'profile-summary.json'
$flamegraphTargetsPath = Join-Path $runRoot 'flamegraph-targets.csv'
$laneMetricsPath = Join-Path $runRoot 'tx-lane-metrics.csv'
$prepareLog = Join-Path $runRoot 'prepare.out.log'
$previousErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    & $nodeExe consensus-keygen --node-count 4 --output-dir $keysDir --force *> $prepareLog
    $exitCode = $LASTEXITCODE
    if ($exitCode -eq 0) {
        & $nodeExe genesis-export --network devnet --data-dir $sourceData --output $genesis *>> $prepareLog
        $exitCode = $LASTEXITCODE
    }
} finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($exitCode -ne 0) {
    Get-Content $prepareLog -Tail 80
    throw "devnet preparation failed with exit code $exitCode; log: $prepareLog"
}

$processes = @{}
$txProcesses = @()
$profileProcesses = @()
$laneStartedAt = @{}
$laneRpcByIndex = @{}
$laneSenderByIndex = @{}
$urls = @(1..4 | ForEach-Object { "http://127.0.0.1:$($BaseRpcPort + (($_ - 1) * 10))" })
if ($ProtectedRpcNodes.Count -eq 0) {
    if ($CrashDuringLoadRounds -gt 0 -and $CrashDuringLoadPattern -ne 'client-ingress') {
        # Keep client transport stable by default: do not send client lanes to
        # nodes selected by the built-in crash pattern. Users can still override
        # -ProtectedRpcNodes for client-disruption testing.
        $ProtectedRpcNodes = switch ($CrashDuringLoadPattern) {
            'leader' { @(2, 3, 4) }
            'two-node' { @(1) }
            default { @(1, 2, 3) }
        }
    } elseif ($ChaosRounds -gt 0) {
        # Keep client transport stable while the remaining authority is repeatedly
        # killed and restarted. Lanes are distributed only over this RPC pool.
        $ProtectedRpcNodes = @(1, 2, 3)
    } else {
        # No chaos target will be stopped, so use every authority RPC as a
        # client ingress to exercise the full local devnet under load.
        $ProtectedRpcNodes = @(1, 2, 3, 4)
    }
}

function Get-DirectorySizeBytes {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) { return 0 }
    $total = 0L
    Get-ChildItem -LiteralPath $Path -Recurse -File -ErrorAction SilentlyContinue |
        ForEach-Object { $total += $_.Length }
    return $total
}

function Write-ProfileSample {
    param(
        [string]$Phase,
        [hashtable]$ProcessesByNode,
        [string[]]$Urls,
        [string]$Path
    )

    if ($ProfileIntervalSec -le 0) { return }
    if (-not (Test-Path -LiteralPath $Path)) {
        'timestamp,phase,node,pid,exited,cpu_sec,working_set_mb,private_mb,handles,height,txs,pending,root,db_mb' |
            Set-Content -LiteralPath $Path
    }
    $timestamp = Get-Date -Format o
    for ($node = 1; $node -le $Urls.Count; $node++) {
        $process = $ProcessesByNode[$node]
        $pidText = ''
        $exited = $true
        $cpu = ''
        $ws = ''
        $private = ''
        $handles = ''
        if ($process) {
            try {
                $process.Refresh()
                $pidText = $process.Id
                $exited = $process.HasExited
                if (-not $process.HasExited) {
                    $cpu = '{0:N3}' -f $process.CPU
                    $ws = '{0:N2}' -f ($process.WorkingSet64 / 1MB)
                    $private = '{0:N2}' -f ($process.PrivateMemorySize64 / 1MB)
                    $handles = $process.HandleCount
                }
            } catch {
            }
        }

        $height = ''
        $txs = ''
        $pending = ''
        $root = ''
        try {
            $stats = (Invoke-KanariRpc -Url $Urls[$node - 1] -Method 'kanari_getStats' -Id 30 -TimeoutSec 2).result
            if ($stats) {
                $height = $stats.height
                $txs = $stats.total_transactions
                $pending = $stats.pending_transactions
                $root = $stats.state_root
            }
        } catch {
        }

        $dbPath = Join-Path $runRoot "node$node-db"
        $dbMb = '{0:N2}' -f ((Get-DirectorySizeBytes -Path $dbPath) / 1MB)
        "$timestamp,$Phase,$node,$pidText,$exited,$cpu,$ws,$private,$handles,$height,$txs,$pending,$root,$dbMb" |
            Add-Content -LiteralPath $Path
    }
}

function Get-PercentileValue {
    param(
        [double[]]$Values,
        [ValidateRange(0, 100)]
        [double]$Percentile
    )

    if (-not $Values -or $Values.Count -eq 0) { return 0 }
    $sorted = @($Values | Sort-Object)
    if ($sorted.Count -eq 1) { return [double]$sorted[0] }
    $rank = ($Percentile / 100.0) * ($sorted.Count - 1)
    $lower = [int][Math]::Floor($rank)
    $upper = [int][Math]::Ceiling($rank)
    if ($lower -eq $upper) { return [double]$sorted[$lower] }
    $weight = $rank - $lower
    return ([double]$sorted[$lower] * (1.0 - $weight)) + ([double]$sorted[$upper] * $weight)
}

function Write-LogProfileSummary {
    param([string]$Path)

    $nodeLogs = Get-ChildItem -LiteralPath $runRoot -Filter 'node*.out.log' -File -ErrorAction SilentlyContinue
    $dbLogs = Get-ChildItem -LiteralPath $runRoot -Recurse -Include 'LOG', 'LOG.old.*' -File -ErrorAction SilentlyContinue
    $laneMetrics = if (Test-Path -LiteralPath $laneMetricsPath) {
        @(Import-Csv -LiteralPath $laneMetricsPath)
    } else {
        @()
    }
    $summary = [ordered]@{
        run_root = $runRoot
        node_logs = $nodeLogs.Count
        db_logs = $dbLogs.Count
        lane_metrics = $laneMetrics.Count
        total_lane_txs = 0
        total_lane_duration_ms = 0
        aggregate_lane_tps = 0
        lane_duration_p50_ms = 0
        lane_duration_p95_ms = 0
        lane_duration_p99_ms = 0
        lane_tps_p50 = 0
        lane_tps_p95 = 0
        parallel_execution_lines = 0
        dag_vertex_lines = 0
        p2p_warn_lines = 0
        p2p_outbound_queue_latency_samples = 0
        p2p_outbound_queue_latency_p50_ms = 0
        p2p_outbound_queue_latency_p95_ms = 0
        p2p_outbound_queue_latency_max_ms = 0
        p2p_queue_full_lines = 0
        p2p_publish_failure_lines = 0
        p2p_no_peer_lines = 0
        sync_divergence_warnings = 0
        sync_recovery_lines = 0
        retry_conflict_lines = 0
        rocksdb_compaction_lines = 0
        rocksdb_flush_lines = 0
        rocksdb_stall_lines = 0
    }
    if ($nodeLogs.Count -gt 0) {
        $paths = $nodeLogs.FullName
        $summary.parallel_execution_lines = (Select-String -Path $paths -Pattern '\[parallel execution\]' -ErrorAction SilentlyContinue).Count
        $summary.dag_vertex_lines = (Select-String -Path $paths -Pattern 'DAG Vertex' -ErrorAction SilentlyContinue).Count
        $summary.p2p_warn_lines = (Select-String -Path $paths -Pattern ' WARN .*kanari_node::p2p' -ErrorAction SilentlyContinue).Count
        $summary.p2p_queue_full_lines = (Select-String -Path $paths -Pattern 'queue is full|Failed to queue|outbound queue is full|incoming P2P queue is full|P2P transaction broadcast queue is full' -ErrorAction SilentlyContinue).Count
        $summary.p2p_publish_failure_lines = (Select-String -Path $paths -Pattern 'Failed to publish|Failed to queue .*broadcast' -ErrorAction SilentlyContinue).Count
        $summary.p2p_no_peer_lines = (Select-String -Path $paths -Pattern 'NoPeersSubscribedToTopic|NoPeers' -ErrorAction SilentlyContinue).Count
        $p2pLatencyValues = @(
            Select-String -Path $paths -Pattern 'p2p_outbound_queue_latency_ms' -ErrorAction SilentlyContinue |
                ForEach-Object {
                    $cleanLine = $_.Line -replace "$([char]27)\[[0-9;]*m", ''
                    if ($cleanLine -match 'p2p_outbound_queue_latency_ms=(\d+)') {
                        [double]$Matches[1]
                    }
                }
        )
        if ($p2pLatencyValues.Count -gt 0) {
            $summary.p2p_outbound_queue_latency_samples = $p2pLatencyValues.Count
            $summary.p2p_outbound_queue_latency_p50_ms = [Math]::Round((Get-PercentileValue -Values $p2pLatencyValues -Percentile 50), 3)
            $summary.p2p_outbound_queue_latency_p95_ms = [Math]::Round((Get-PercentileValue -Values $p2pLatencyValues -Percentile 95), 3)
            $summary.p2p_outbound_queue_latency_max_ms = [Math]::Round(($p2pLatencyValues | Measure-Object -Maximum).Maximum, 3)
        }
        $summary.sync_divergence_warnings = (Select-String -Path $paths -Pattern 'Diverged state detected' -ErrorAction SilentlyContinue).Count
        $summary.sync_recovery_lines = (Select-String -Path $paths -Pattern 'now matches local checkpoint/state again' -ErrorAction SilentlyContinue).Count
        $summary.retry_conflict_lines = (Select-String -Path $paths -Pattern 'retry_conflict_txs=[1-9]' -ErrorAction SilentlyContinue).Count
    }
    if ($dbLogs.Count -gt 0) {
        $dbPaths = $dbLogs.FullName
        $summary.rocksdb_compaction_lines = (Select-String -Path $dbPaths -Pattern 'compaction|Compaction' -ErrorAction SilentlyContinue).Count
        $summary.rocksdb_flush_lines = (Select-String -Path $dbPaths -Pattern 'flush|Flush' -ErrorAction SilentlyContinue).Count
        $summary.rocksdb_stall_lines = (Select-String -Path $dbPaths -Pattern 'stall|Stall|Stopped writes|delayed write' -ErrorAction SilentlyContinue).Count
    }
    if ($laneMetrics.Count -gt 0) {
        $durations = @($laneMetrics | ForEach-Object { [double]$_.elapsed_ms })
        $tpsValues = @($laneMetrics | ForEach-Object { [double]$_.tps })
        $totalTxs = ($laneMetrics | Measure-Object -Property requested_txs -Sum).Sum
        $maxDurationMs = ($laneMetrics | Measure-Object -Property elapsed_ms -Maximum).Maximum
        $summary.total_lane_txs = [int]$totalTxs
        $summary.total_lane_duration_ms = [int]$maxDurationMs
        if ($maxDurationMs -gt 0) {
            $summary.aggregate_lane_tps = [Math]::Round(($totalTxs / ($maxDurationMs / 1000.0)), 3)
        }
        $summary.lane_duration_p50_ms = [Math]::Round((Get-PercentileValue -Values $durations -Percentile 50), 3)
        $summary.lane_duration_p95_ms = [Math]::Round((Get-PercentileValue -Values $durations -Percentile 95), 3)
        $summary.lane_duration_p99_ms = [Math]::Round((Get-PercentileValue -Values $durations -Percentile 99), 3)
        $summary.lane_tps_p50 = [Math]::Round((Get-PercentileValue -Values $tpsValues -Percentile 50), 3)
        $summary.lane_tps_p95 = [Math]::Round((Get-PercentileValue -Values $tpsValues -Percentile 95), 3)
    }
    $summary | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Path
}

function Write-FlamegraphTargets {
    param(
        [hashtable]$ProcessesByNode,
        [string]$Path
    )

    'node,pid,role,hint' | Set-Content -LiteralPath $Path
    foreach ($node in ($ProcessesByNode.Keys | Sort-Object)) {
        $process = $ProcessesByNode[$node]
        try {
            $process.Refresh()
            if (-not $process.HasExited) {
                "$node,$($process.Id),kanari-node,attach external sampler/flamegraph profiler to this pid during load or recovery audit" |
                    Add-Content -LiteralPath $Path
            }
        } catch {
        }
    }
}

function Start-LinuxPerfRecorders {
    param(
        [hashtable]$ProcessesByNode,
        [string]$Phase
    )

    $recorders = @()
    if (-not $StartLinuxPerfRecorders) { return $recorders }
    if (-not $IsLinux) {
        Write-Host "Linux perf recorders requested but this host is not Linux; wrote flamegraph-targets.csv for external profiler attach instead"
        return $recorders
    }
    $perfCommand = Get-Command perf -ErrorAction SilentlyContinue
    if (-not $perfCommand) {
        Write-Host "Linux perf recorders requested but 'perf' was not found; install linux-tools/perf and rerun"
        return $recorders
    }

    $perfRoot = Join-Path $runRoot 'perf'
    New-Item -ItemType Directory -Force -Path $perfRoot | Out-Null
    foreach ($node in ($ProcessesByNode.Keys | Sort-Object)) {
        $process = $ProcessesByNode[$node]
        try {
            $process.Refresh()
            if ($process.HasExited) { continue }
            $dataPath = Join-Path $perfRoot "node$node-$Phase.perf.data"
            $outPath = Join-Path $perfRoot "node$node-$Phase.perf.out.log"
            $errPath = Join-Path $perfRoot "node$node-$Phase.perf.err.log"
            $args = @(
                'record',
                '-F', $PerfSampleHz,
                '-g',
                '-p', $process.Id,
                '-o', $dataPath,
                '--',
                'sleep', $PerfDurationSec
            )
            Write-Host "PERF node$node pid=$($process.Id) phase=$Phase data=$dataPath"
            $recorders += Start-Process -FilePath $perfCommand.Source -ArgumentList $args -NoNewWindow -PassThru -RedirectStandardOutput $outPath -RedirectStandardError $errPath
        } catch {
            Write-Host "PERF node$node failed to start: $($_.Exception.Message)"
        }
    }
    return $recorders
}

function Invoke-RecoveryAudit {
    param(
        [int]$Rounds,
        [hashtable]$ProcessesByNode,
        [string[]]$Urls
    )

    if ($Rounds -le 0) { return }
    $auditPath = Join-Path $runRoot 'recovery-audit.csv'
    'timestamp,round,node,action,height,txs,pending,root' | Set-Content -LiteralPath $auditPath
    for ($round = 1; $round -le $Rounds; $round++) {
        for ($node = 1; $node -le $Urls.Count; $node++) {
            Write-Host "RECOVERY audit round=$round restarting node$node"
            if ($ProcessesByNode.ContainsKey($node)) {
                try {
                    if (-not $ProcessesByNode[$node].HasExited) {
                        Stop-Process -Id $ProcessesByNode[$node].Id -Force
                    }
                } catch {
                }
                $ProcessesByNode.Remove($node)
            }
            Start-Sleep -Seconds $RecoveryRestartDelaySec
            $ProcessesByNode[$node] = Start-KanariNode -Index $node -OutSuffix "audit$round"
            Wait-RpcReady -Urls @($Urls[$node - 1]) -Attempts 120
            Wait-StatsConverged -Urls $Urls
            foreach ($urlIndex in 0..($Urls.Count - 1)) {
                $stats = (Invoke-KanariRpc -Url $Urls[$urlIndex] -Method 'kanari_getStats' -Id 40 -TimeoutSec 3).result
                "$(Get-Date -Format o),$round,$($urlIndex + 1),post-restart,$($stats.height),$($stats.total_transactions),$($stats.pending_transactions),$($stats.state_root)" |
                    Add-Content -LiteralPath $auditPath
            }
        }
    }
}

function Get-CrashTargetsForRound {
    param(
        [int]$Round,
        [string]$Pattern,
        [int[]]$ConfiguredNodes,
        [int]$NodeCount
    )

    if ($ConfiguredNodes.Count -gt 0) {
        if ($Pattern -eq 'two-node' -and $ConfiguredNodes.Count -gt 1) {
            $first = ($Round - 1) % $ConfiguredNodes.Count
            $second = ($first + 1) % $ConfiguredNodes.Count
            return @($ConfiguredNodes[$first], $ConfiguredNodes[$second]) | Sort-Object -Unique
        }
        return @($ConfiguredNodes[(($Round - 1) % $ConfiguredNodes.Count)])
    }

    switch ($Pattern) {
        'leader' { return @(1) }
        'two-node' {
            $pairs = @(@(3, 4), @(2, 4), @(2, 3))
            return $pairs[(($Round - 1) % $pairs.Count)]
        }
        'round-robin' { return @(1 + (($Round - 1) % $NodeCount)) }
        'client-ingress' { return @(1 + (($Round - 1) % [Math]::Min(3, $NodeCount))) }
        default { return @(4) }
    }
}

function Restart-CrashTargets {
    param(
        [int]$Round,
        [int[]]$Targets,
        [hashtable]$ProcessesByNode
    )

    foreach ($target in $Targets) {
        if ($ProcessesByNode.ContainsKey($target)) {
            Write-Host "CRASH-DURING-LOAD round=$Round stopping node$target"
            try {
                if (-not $ProcessesByNode[$target].HasExited) {
                    Stop-Process -Id $ProcessesByNode[$target].Id -Force
                }
            } catch {
            }
            $ProcessesByNode.Remove($target)
        }
    }

    $delay = if ($CrashDuringLoadRestartDelaySec -gt 0) {
        $CrashDuringLoadRestartDelaySec
    } else {
        $RecoveryRestartDelaySec
    }
    Start-Sleep -Seconds $delay

    foreach ($target in $Targets) {
        Write-Host "CRASH-DURING-LOAD round=$Round restarting node$target"
        $ProcessesByNode[$target] = Start-KanariNode -Index $target -OutSuffix "loadcrash$Round"
        Wait-RpcReady -Urls @("http://127.0.0.1:$($BaseRpcPort + (($target - 1) * 10))") -Attempts 120
    }
}
$ProtectedRpcNodes = @($ProtectedRpcNodes | Sort-Object -Unique)
if ($ProtectedRpcNodes | Where-Object { $_ -lt 1 -or $_ -gt $urls.Count }) {
    throw "ProtectedRpcNodes must contain node indexes from 1 to $($urls.Count)"
}
$chaosTargets = @(1..4 | Where-Object { $ProtectedRpcNodes -notcontains $_ })
if ($ChaosRounds -gt 0 -and $chaosTargets.Count -eq 0) {
    throw "No chaos targets remain after protecting RPC nodes: $($ProtectedRpcNodes -join ', ')"
}
if ($CrashDuringLoadNodes.Count -eq 0) {
    $CrashDuringLoadNodes = @()
}
$CrashDuringLoadNodes = @($CrashDuringLoadNodes | Sort-Object -Unique)
if ($CrashDuringLoadNodes | Where-Object { $_ -lt 1 -or $_ -gt $urls.Count }) {
    throw "CrashDuringLoadNodes must contain node indexes from 1 to $($urls.Count)"
}
Write-Host "Protected RPC nodes: $($ProtectedRpcNodes -join ', ')"
Write-Host "Chaos target node pool: $($chaosTargets -join ', ')"
Write-Host "Crash-during-load pattern: $CrashDuringLoadPattern nodes=$($CrashDuringLoadNodes -join ', ') restart_delay=${CrashDuringLoadRestartDelaySec}s"
if ($ProfileIntervalSec -gt 0) {
    Write-Host "Profile samples: every ${ProfileIntervalSec}s -> $profilePath"
}
$laneRpcUrls = @($ProtectedRpcNodes | ForEach-Object { $urls[$_ - 1] })

try {
    foreach ($i in 1..4) {
        $processes[$i] = Start-KanariNode -Index $i
        Start-Sleep -Milliseconds 900
    }

    Wait-RpcReady -Urls $urls
    Wait-StatsConverged -Urls $urls
    Write-FlamegraphTargets -ProcessesByNode $processes -Path $flamegraphTargetsPath
    Write-ProfileSample -Phase 'startup' -ProcessesByNode $processes -Urls $urls -Path $profilePath

    if ($FundSenders) {
        $devAddress = '0x3ba63b92aac5f2bff87e580e820b61faf1c5fe9ae12f0bc8addd931a340b3146'
        foreach ($sender in $Senders) {
            if ($sender.Equals($devAddress, [StringComparison]::OrdinalIgnoreCase)) {
                Write-Host "Funding sender $sender skipped; dev genesis wallet already owns native coin objects"
                continue
            }
            Write-Host "Funding sender $sender with $FaucetCoinsPerSender coin object(s) of $FaucetAmount KANARI"
            for ($fundIndex = 1; $fundIndex -le $FaucetCoinsPerSender; $fundIndex++) {
                $fundLog = Join-Path $runRoot "fund-$($sender.Substring(2, [Math]::Min(12, $sender.Length - 2)))-$fundIndex.log"
                $fundErr = Join-Path $runRoot "fund-$($sender.Substring(2, [Math]::Min(12, $sender.Length - 2)))-$fundIndex.err.log"
                $env:KANARI_KEYSTORE_PATH = $KeystorePath
                $fundArgs = @(
                    'client',
                    'faucet',
                    '--to',
                    $sender,
                    '--amount',
                    $FaucetAmount,
                    '--rpc',
                    $urls[0],
                    '--dev-password',
                    $Password
                )
                $fundProcess = Start-Process `
                    -FilePath $kanariExe `
                    -ArgumentList $fundArgs `
                    -WorkingDirectory $repoRoot `
                    -RedirectStandardOutput $fundLog `
                    -RedirectStandardError $fundErr `
                    -WindowStyle Hidden `
                    -Wait `
                    -PassThru
                if ($fundProcess.ExitCode -ne 0) {
                    Get-Content $fundLog -Tail 80
                    Get-Content $fundErr -Tail 80
                    throw "fund sender failed for $sender coin $fundIndex; logs: $fundLog $fundErr"
                }
            }
        }
        Wait-StatsConverged -Urls $urls
        Write-ProfileSample -Phase 'post-funding' -ProcessesByNode $processes -Urls $urls -Path $profilePath
    }

    Write-Host "Starting $($Senders.Count) parallel tx lanes, $CountPerSender tx each"
    'lane,pid,sender,rpc,requested_txs,started_at,ended_at,elapsed_ms,tps,exit_code,success' |
        Set-Content -LiteralPath $laneMetricsPath
    for ($i = 0; $i -lt $Senders.Count; $i++) {
        $sender = $Senders[$i]
        $laneRecipient = if ($SelfRecipient) { $sender } else { $Recipient }
        $rpc = $laneRpcUrls[$i % $laneRpcUrls.Count]
        $lane = $i + 1
        $txLog = Join-Path $runRoot "lane$lane.tx.out.log"
        $txErr = Join-Path $runRoot "lane$lane.tx.err.log"
        $laneKeystorePath = Join-Path $runRoot "lane$lane.kanari.keystore"
        Copy-Item -LiteralPath $KeystorePath -Destination $laneKeystorePath -Force
        $txArgs = @(
            '-NoProfile',
            '-ExecutionPolicy',
            'Bypass',
            '-File',
            $stressLaneScript,
            '-KanariExe', $kanariExe,
            '-KeystorePath', $laneKeystorePath,
            '-Sender', $sender,
            '-Recipient', $laneRecipient,
            '-Amount', $Amount,
            '-Count', $CountPerSender,
            '-Rpc', $rpc
        )
        $env:KANARI_LOAD_PASSWORD = $Password
        $startedAt = Get-Date
        $laneProcess = Start-Process `
            -FilePath powershell `
            -ArgumentList $txArgs `
            -WorkingDirectory $repoRoot `
            -RedirectStandardOutput $txLog `
            -RedirectStandardError $txErr `
            -WindowStyle Hidden `
            -PassThru
        $txProcesses += $laneProcess
        $laneStartedAt[$lane] = $startedAt
        $laneRpcByIndex[$lane] = $rpc
        $laneSenderByIndex[$lane] = $sender
        Write-Host "LANE $lane sender=$sender to=$laneRecipient rpc=$rpc pid=$($laneProcess.Id)"
    }

    $profileProcesses += Start-LinuxPerfRecorders -ProcessesByNode $processes -Phase 'load'

    for ($round = 1; $round -le $ChaosRounds; $round++) {
        Start-Sleep -Seconds 10
        $target = $chaosTargets[(($round - 1) % $chaosTargets.Count)]
        if ($processes.ContainsKey($target)) {
            Write-Host "CHAOS round=$round stopping node$target"
            Stop-Process -Id $processes[$target].Id -Force
            $processes.Remove($target)
        }
        Start-Sleep -Seconds 10
        Write-Host "CHAOS round=$round survivor stats"
        Show-Stats -Urls ($urls | Where-Object { $_ -ne "http://127.0.0.1:$($BaseRpcPort + (($target - 1) * 10))" })

        Write-Host "CHAOS round=$round restarting node$target"
        $processes[$target] = Start-KanariNode -Index $target -OutSuffix "restart$round"
        Wait-RpcReady -Urls @("http://127.0.0.1:$($BaseRpcPort + (($target - 1) * 10))") -Attempts 120
    }

    Write-Host "Waiting for tx lanes to finish"
    $remainingCrashRounds = $CrashDuringLoadRounds
    $lastProfileSample = Get-Date
    $nextCrashAt = if ($remainingCrashRounds -gt 0) { (Get-Date).AddSeconds($CrashDuringLoadFirstDelaySec) } else { $null }
    while ($txProcesses | Where-Object { -not $_.HasExited }) {
        $now = Get-Date
        if ($ProfileIntervalSec -gt 0 -and (($now - $lastProfileSample).TotalSeconds -ge $ProfileIntervalSec)) {
            Write-ProfileSample -Phase 'load' -ProcessesByNode $processes -Urls $urls -Path $profilePath
            $lastProfileSample = $now
        }
        if ($remainingCrashRounds -gt 0 -and $null -ne $nextCrashAt -and $now -ge $nextCrashAt) {
            $roundNumber = $CrashDuringLoadRounds - $remainingCrashRounds + 1
            $targets = Get-CrashTargetsForRound `
                -Round $roundNumber `
                -Pattern $CrashDuringLoadPattern `
                -ConfiguredNodes $CrashDuringLoadNodes `
                -NodeCount $urls.Count
            Restart-CrashTargets -Round $roundNumber -Targets $targets -ProcessesByNode $processes
            $remainingCrashRounds -= 1
            $nextCrashAt = if ($remainingCrashRounds -gt 0) { (Get-Date).AddSeconds(10) } else { $null }
        }
        Start-Sleep -Seconds 1
    }

    $laneFailures = 0
    for ($processIndex = 0; $processIndex -lt $txProcesses.Count; $processIndex++) {
        $process = $txProcesses[$processIndex]
        $lane = $processIndex + 1
        $process.WaitForExit()
        $process.Refresh()
        $exitOk = $process.ExitCode -eq 0
        if ($null -eq $process.ExitCode) {
            $laneErr = Join-Path $runRoot "lane$lane.tx.err.log"
            $laneText = if (Test-Path -LiteralPath $laneErr) {
                Get-Content -Raw -LiteralPath $laneErr
            } else {
                ''
            }
            $exitOk = $laneText -match 'Failed:\s+0' -and $laneText -notmatch 'Error:'
        }
        if (-not $exitOk) {
            $laneFailures += 1
            Write-Host "LANE FAILED lane=$lane pid=$($process.Id) exit=$($process.ExitCode)"
        }
        $startedAt = $laneStartedAt[$lane]
        $endedAt = Get-Date
        try {
            $candidateExitTime = $process.ExitTime
            if ($candidateExitTime -gt $startedAt) {
                $endedAt = $candidateExitTime
            }
        } catch {
        }
        $elapsedMs = [Math]::Max(1, [int](($endedAt - $startedAt).TotalMilliseconds))
        $laneTps = [Math]::Round(($CountPerSender / ($elapsedMs / 1000.0)), 3)
        "$lane,$($process.Id),$($laneSenderByIndex[$lane]),$($laneRpcByIndex[$lane]),$CountPerSender,$($startedAt.ToString('o')),$($endedAt.ToString('o')),$elapsedMs,$laneTps,$($process.ExitCode),$exitOk" |
            Add-Content -LiteralPath $laneMetricsPath
    }

    Wait-StatsConverged -Urls $urls
    Show-Stats -Urls $urls
    Write-ProfileSample -Phase 'post-load' -ProcessesByNode $processes -Urls $urls -Path $profilePath
    if ($laneFailures -gt 0) {
        throw "$laneFailures tx lane(s) failed; inspect lane*.tx.*.log in $runRoot"
    }
    Invoke-RecoveryAudit -Rounds $RecoveryAuditRounds -ProcessesByNode $processes -Urls $urls
    Write-FlamegraphTargets -ProcessesByNode $processes -Path $flamegraphTargetsPath
    Write-ProfileSample -Phase 'post-recovery-audit' -ProcessesByNode $processes -Urls $urls -Path $profilePath
    Write-LogProfileSummary -Path $profileSummaryPath
    Write-Host "Four-node parallel tx chaos complete"
} finally {
    foreach ($process in $profileProcesses) {
        try {
            if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force }
        } catch {
        }
    }
    foreach ($process in $txProcesses) {
        try {
            if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
        } catch {
        }
    }
    foreach ($process in $processes.Values) {
        try {
            if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
        } catch {
        }
    }
    Remove-Item -LiteralPath $localKeystorePath -Force -ErrorAction SilentlyContinue
    Get-ChildItem -LiteralPath $runRoot -Filter 'lane*.kanari.keystore' -File |
        Remove-Item -Force -ErrorAction SilentlyContinue
    Get-ChildItem -LiteralPath $runRoot -Filter 'temp-wallet-*.create.*.log' -File |
        Remove-Item -Force -ErrorAction SilentlyContinue
    Write-Host "Run artifacts: $runRoot"
}
