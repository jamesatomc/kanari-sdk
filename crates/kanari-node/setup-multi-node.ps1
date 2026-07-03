# Kanari Multi-Node Launcher
param(
    [int]$NodeCount = 3,
    [ValidateSet("mainnet", "testnet", "devnet")]
    [string]$Network = "testnet",
    [string]$SourceNodeDataDir = "$env:USERPROFILE\.kanari\kanari-db",
    [string]$ReplicaBaseDataDir = "$env:USERPROFILE\.kanari\node-db",
    [int]$BasePeerPort = 19000,
    [int]$BaseRpcPort = 19001,
    [string]$RpcHost = "0.0.0.0",
    [switch]$ResetReplicaData,
    [switch]$ResetSourceData,
    [switch]$AllowUnsafeResetPath,
    [switch]$AllowReuseData,
    [switch]$DisableFailFast,
    [switch]$SkipHealthCheck,
    [string]$ConsensusKeyDir = "$env:USERPROFILE\.kanari\consensus-keys",
    [switch]$ResetConsensusKeys
)

. (Join-Path $PSScriptRoot 'node-script-common.ps1')

function Test-DirectoryHasEntries {
    param(
        [string]$Path
    )

    if (-not (Test-Path $Path)) {
        return $false
    }

    return $null -ne (Get-ChildItem -LiteralPath $Path -Force -ErrorAction SilentlyContinue | Select-Object -First 1)
}

function Assert-SafeResetPath {
    param(
        [string]$Path,
        [string]$Label
    )

    if ($AllowUnsafeResetPath) {
        return
    }

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $fullTrim = $fullPath.TrimEnd([char[]]@('\', '/'))
    $kanariHome = [System.IO.Path]::GetFullPath((Join-Path $env:USERPROFILE ".kanari"))
    $kanariTrim = $kanariHome.TrimEnd([char[]]@('\', '/'))
    $homeTrim = ([System.IO.Path]::GetFullPath($env:USERPROFILE)).TrimEnd([char[]]@('\', '/'))
    $rootTrim = ([System.IO.Path]::GetPathRoot($fullPath)).TrimEnd([char[]]@('\', '/'))
    $kanariPrefix = $kanariTrim + [System.IO.Path]::DirectorySeparatorChar

    if ($fullTrim -eq $homeTrim -or $fullTrim -eq $kanariTrim -or $fullTrim -eq $rootTrim) {
        throw "$Label reset path is too broad: $fullPath"
    }

    if (-not $fullTrim.StartsWith($kanariPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label reset path must be under $kanariTrim unless -AllowUnsafeResetPath is passed: $fullPath"
    }
}

Write-Host "Starting Kanari Multi-Node Setup..." -ForegroundColor Green
Write-Host ""

if ($NodeCount -lt 1) {
    Write-Host "NodeCount must be at least 1." -ForegroundColor Red
    exit 1
}

$failFastEnabled = -not $DisableFailFast
$env:KANARI_FAIL_FAST_ON_SUPPLY_MISMATCH = if ($failFastEnabled) { "true" } else { "false" }

Write-Host "Node count: $NodeCount" -ForegroundColor Cyan
Write-Host "Network: $Network" -ForegroundColor Cyan
Write-Host "Source node data dir (node1): $SourceNodeDataDir" -ForegroundColor Cyan
Write-Host "Replica base data dir (node2..N): $ReplicaBaseDataDir" -ForegroundColor Cyan
Write-Host "RPC bind host: $RpcHost" -ForegroundColor Cyan
Write-Host "Supply fail-fast: $($env:KANARI_FAIL_FAST_ON_SUPPLY_MISMATCH)" -ForegroundColor Cyan
Write-Host "Allow reuse existing data: $AllowReuseData" -ForegroundColor Cyan
Write-Host "Consensus key dir: $ConsensusKeyDir" -ForegroundColor Cyan
Write-Host ""

if (-not (Test-Path $SourceNodeDataDir)) {
    New-Item -ItemType Directory -Path $SourceNodeDataDir -Force | Out-Null
}
if (-not (Test-Path $ReplicaBaseDataDir)) {
    New-Item -ItemType Directory -Path $ReplicaBaseDataDir -Force | Out-Null
}

$existingReplicaDirs = @()
for ($i = 2; $i -le $NodeCount; $i++) {
    $nodeDir = Join-Path $ReplicaBaseDataDir "node$i"
    if (Test-DirectoryHasEntries -Path $nodeDir) {
        $existingReplicaDirs += $nodeDir
    }
}

$sourceHasExistingData = Test-DirectoryHasEntries -Path $SourceNodeDataDir
$hasReusableData = $sourceHasExistingData -or $existingReplicaDirs.Count -gt 0

if ($hasReusableData -and -not $AllowReuseData -and (-not $ResetSourceData -or -not $ResetReplicaData)) {
    Write-Host ""
    Write-Host "Refusing to launch multi-node cluster with reused data by default." -ForegroundColor Red
    Write-Host "This is a common cause of checkpoint/state-root divergence between nodes." -ForegroundColor Red
    if ($sourceHasExistingData) {
        Write-Host "Existing source data detected: $SourceNodeDataDir" -ForegroundColor Yellow
    }
    foreach ($nodeDir in $existingReplicaDirs) {
        Write-Host "Existing replica data detected: $nodeDir" -ForegroundColor Yellow
    }
    Write-Host ""
    Write-Host "Use one of these options:" -ForegroundColor Cyan
    Write-Host "  1. Fresh start: .\setup-multi-node.ps1 -NodeCount $NodeCount -ResetSourceData -ResetReplicaData" -ForegroundColor Cyan
    Write-Host "  2. Reuse intentionally: add -AllowReuseData" -ForegroundColor Cyan
    exit 1
}

if ($ResetSourceData) {
    Write-Host "ResetSourceData enabled: clearing source node database..." -ForegroundColor Yellow
    Assert-SafeResetPath -Path $SourceNodeDataDir -Label "SourceNodeDataDir"
    Get-ChildItem -LiteralPath $SourceNodeDataDir -Force -ErrorAction SilentlyContinue |
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
} else {
    Write-Host "Preserving source node database for node1." -ForegroundColor Cyan
}

if ($ResetReplicaData) {
    Write-Host "ResetReplicaData enabled: clearing replica node databases..." -ForegroundColor Yellow
    for ($i = 2; $i -le $NodeCount; $i++) {
        $nodeDir = Join-Path $ReplicaBaseDataDir "node$i"
        if (Test-Path $nodeDir) {
            Assert-SafeResetPath -Path $nodeDir -Label "Replica node$i"
            Remove-Item -LiteralPath $nodeDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
} else {
    Write-Host "Preserving replica node databases. Use -ResetReplicaData for a fresh sync set." -ForegroundColor Cyan
}

$authorities = @()
for ($i = 1; $i -le $NodeCount; $i++) {
    $authorities += "0x$i"
}
$authoritiesStr = $authorities -join ","

Write-Host "Authority committee: $authoritiesStr" -ForegroundColor Cyan

try {
    $exeInfo = Find-KanariNodeExecutable
    $exePath = $exeInfo.Path
    Write-Host $exeInfo.Label -ForegroundColor $exeInfo.Color
} catch {
    Write-Host 'Error: kanari-node executable not found! Build it first with: cargo build -p kanari-node' -ForegroundColor Red
    exit 1
}

if ($ResetConsensusKeys -and (Test-Path $ConsensusKeyDir)) {
    Write-Host "ResetConsensusKeys enabled: clearing consensus keys..." -ForegroundColor Yellow
    Assert-SafeResetPath -Path $ConsensusKeyDir -Label "ConsensusKeyDir"
    Remove-Item -LiteralPath $ConsensusKeyDir -Recurse -Force -ErrorAction SilentlyContinue
}

$publicKeysPath = Join-Path $ConsensusKeyDir "consensus-public-keys.json"
$missingConsensusKeys = -not (Test-Path $publicKeysPath)
for ($i = 1; $i -le $NodeCount; $i++) {
    $privateKeyPath = Join-Path $ConsensusKeyDir "node$i-consensus-private-key.hex"
    if (-not (Test-Path $privateKeyPath)) {
        $missingConsensusKeys = $true
        break
    }
}

if ($missingConsensusKeys) {
    Write-Host "Generating consensus keys for $NodeCount node(s)..." -ForegroundColor Cyan
    & $exePath consensus-keygen --node-count $NodeCount --output-dir $ConsensusKeyDir --force
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Failed to generate consensus keys." -ForegroundColor Red
        exit $LASTEXITCODE
    }
} else {
    Write-Host "Using existing consensus keys in $ConsensusKeyDir" -ForegroundColor Cyan
}

$localIp = Get-LanIpAddress
if (-not $localIp) {
    Write-Host "Warning: no LAN IPv4 detected. Using 127.0.0.1 for local bootstrap and RPC health checks." -ForegroundColor Yellow
}
$bootstrapHost = if ($localIp) { $localIp } else { "127.0.0.1" }
$rpcConnectHost = Get-RpcConnectHost -RpcHost $RpcHost -LanIp $localIp

Write-Host ""
$startNow = Read-Host "Start all $NodeCount nodes now with node1 as source? (Y/N)"
if ($startNow -notmatch '^[Yy]') {
    Write-Host "Aborted." -ForegroundColor Yellow
    exit 0
}

$scriptPath = Join-Path $PSScriptRoot 'start-node.ps1'
$currentPS = (Get-Process -Id $PID).Path

for ($i = 1; $i -le $NodeCount; $i++) {
    $ports = Get-NodePorts -NodeId $i -BasePeerPort $BasePeerPort -BaseRpcPort $BaseRpcPort
    $nodeP2pPort = $ports.P2pPort
    $nodeRpcPort = $ports.RpcPort

    if ($i -eq 1) {
        $dataDir = $SourceNodeDataDir
        $argString = "-NoExit -ExecutionPolicy Bypass -File `"$scriptPath`" -NodeId $i -Network $Network -DataDir `"$dataDir`" -BasePeerPort $BasePeerPort -BaseRpcPort $BaseRpcPort -RpcHost `"$RpcHost`" -Authorities `"$authoritiesStr`" -ConsensusKeyDir `"$ConsensusKeyDir`""
    } else {
        $dataDir = Get-NodeDataDir -NodeId $i -DataDir "" -BaseDataDir $ReplicaBaseDataDir
        $bootstrapAddr = "/ip4/$bootstrapHost/tcp/$BasePeerPort"
        $argString = "-NoExit -ExecutionPolicy Bypass -File `"$scriptPath`" -NodeId $i -Network $Network -DataDir `"$dataDir`" -BasePeerPort $BasePeerPort -BaseRpcPort $BaseRpcPort -RpcHost `"$RpcHost`" -Authorities `"$authoritiesStr`" -Bootstrap `"$bootstrapAddr`" -ConsensusKeyDir `"$ConsensusKeyDir`""
    }

    Write-Host "Launching node $i | Authority 0x$i | P2P $nodeP2pPort | RPC $RpcHost`:$nodeRpcPort | DataDir $dataDir" -ForegroundColor Cyan
    Start-Process -FilePath $currentPS -ArgumentList $argString -WindowStyle Normal

    if ($i -eq 1) {
        Write-Host "Waiting 5 seconds for node1 source to initialize on ${bootstrapHost}:$BasePeerPort..." -ForegroundColor Cyan
        Start-Sleep -Seconds 5
    } else {
        Start-Sleep -Milliseconds 600
    }
}

Write-Host "Launched $NodeCount terminals successfully." -ForegroundColor Green
Write-Host "Node1 is the source node. Node2..N bootstrap from node1." -ForegroundColor Green

if (-not $SkipHealthCheck) {
    Write-Host ""
    Write-Host "Checking RPC health endpoints..." -ForegroundColor Cyan
    Start-Sleep -Seconds 3

    for ($i = 1; $i -le $NodeCount; $i++) {
        $ports = Get-NodePorts -NodeId $i -BasePeerPort $BasePeerPort -BaseRpcPort $BaseRpcPort
        $rpcUrl = Get-NodeRpcUrl -HostIp $rpcConnectHost -RpcPort $ports.RpcPort
        if ($i -eq 1) {
            Test-NodeHealth -RpcUrl $rpcUrl -NodeId $i -RequireBootstrappedState
        } else {
            Test-NodeHealth -RpcUrl $rpcUrl -NodeId $i
        }
    }
}
