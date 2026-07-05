# Start a specific Kanari node
param(
    [Parameter(Mandatory=$true)]
    [int]$NodeId = 1,

    [ValidateSet("mainnet", "testnet", "devnet")]
    [string]$Network = "testnet",
    [string]$DataDir = "",
    [string]$BaseDataDir = "$env:USERPROFILE\.kanari\node-db",
    [int]$BasePeerPort = 19000,
    [int]$BaseRpcPort = 19001,
    [string]$RpcHost = "0.0.0.0",
    [string]$Authorities = "",
    [string]$Bootstrap = "",
    [string]$ConsensusPrivateKeyHex = "",
    [string]$ConsensusPublicKeys = "",
    [string]$ConsensusKeyDir = "$env:USERPROFILE\.kanari\consensus-keys"
)

. (Join-Path $PSScriptRoot 'node-script-common.ps1')

$ports = Get-NodePorts -NodeId $NodeId -BasePeerPort $BasePeerPort -BaseRpcPort $BaseRpcPort
$p2pPort = $ports.P2pPort
$rpcPort = $ports.RpcPort
$dataDir = Get-NodeDataDir -NodeId $NodeId -DataDir $DataDir -BaseDataDir $BaseDataDir
$authId = "0x$NodeId"
$localIp = Get-LanIpAddress
$rpcConnectHost = Get-RpcConnectHost -RpcHost $RpcHost -LanIp $localIp
$rpcUrl = Get-NodeRpcUrl -HostIp $rpcConnectHost -RpcPort $rpcPort

if (-not (Test-Path $dataDir)) {
    New-Item -ItemType Directory -Path $dataDir -Force | Out-Null
}

Write-Host '========================================' -ForegroundColor Cyan
Write-Host 'Starting Kanari Node' $NodeId -ForegroundColor Green
Write-Host '========================================' -ForegroundColor Cyan
Write-Host 'P2P Port:' $p2pPort -ForegroundColor Yellow
Write-Host 'RPC Port:' $rpcPort -ForegroundColor Yellow
Write-Host 'RPC Bind Host:' $RpcHost -ForegroundColor Yellow
Write-Host 'Network:' $Network -ForegroundColor Yellow
Write-Host 'Data Dir:' $dataDir -ForegroundColor Yellow
Write-Host "RPC URL:  $rpcUrl" -ForegroundColor Yellow
Write-Host '========================================' -ForegroundColor Cyan
Write-Host ''

try {
    $exeInfo = Find-KanariNodeExecutable
    $exePath = $exeInfo.Path
    Write-Host $exeInfo.Label -ForegroundColor $exeInfo.Color
} catch {
    Write-Host 'Error: kanari-node executable not found!' -ForegroundColor Red
    exit 1
}

Write-Host ''

if ([string]::IsNullOrWhiteSpace($Authorities)) {
    Write-Host 'Error: multi-node DAG startup requires -Authorities so kanari-node can pass --authority-id and --authorities explicitly.' -ForegroundColor Red
    exit 1
}

if ([string]::IsNullOrWhiteSpace($ConsensusPrivateKeyHex)) {
    $privateKeyPath = Join-Path $ConsensusKeyDir "node$NodeId-consensus-private-key.hex"
    if (-not (Test-Path $privateKeyPath)) {
        Write-Host "Error: consensus private key not found: $privateKeyPath" -ForegroundColor Red
        Write-Host "Run setup-multi-node.ps1 first, or pass -ConsensusPrivateKeyHex explicitly." -ForegroundColor Yellow
        exit 1
    }
    $ConsensusPrivateKeyHex = (Get-Content -LiteralPath $privateKeyPath -Raw).Trim()
}

if ([string]::IsNullOrWhiteSpace($ConsensusPublicKeys)) {
    $ConsensusPublicKeys = Join-Path $ConsensusKeyDir "consensus-public-keys.json"
}

if (-not (Test-Path $ConsensusPublicKeys)) {
    Write-Host "Error: consensus public keys file not found: $ConsensusPublicKeys" -ForegroundColor Red
    Write-Host "Run setup-multi-node.ps1 first, or pass -ConsensusPublicKeys explicitly." -ForegroundColor Yellow
    exit 1
}

$nodeArgs = @(
    "start",
    "--network", $Network,
    "--p2p-port", $p2pPort,
    "--rpc-port", $rpcPort,
    "--rpc-host", $RpcHost,
    "--data-dir", $dataDir,
    "--authority-id", $authId,
    "--authorities", $Authorities,
    "--consensus-private-key-hex", $ConsensusPrivateKeyHex,
    "--consensus-public-keys", $ConsensusPublicKeys
)

if ($Bootstrap -ne "") {
    $nodeArgs += @("--bootstrap", $Bootstrap)
}

& $exePath @nodeArgs
