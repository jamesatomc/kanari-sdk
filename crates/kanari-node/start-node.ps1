# Start one Kanari validator from an operator committee manifest.
param(
    [Parameter(Mandatory=$true)]
    [string]$CommitteeConfig,

    [string]$AuthorityId = '',
    [int]$NodeId = 0,
    [string]$DataDir = '',
    [int]$P2pPort = 0,
    [int]$RpcPort = 0,
    [string]$RpcHost = '',
    [string]$Bootstrap = '',
    [string]$ConsensusPrivateKeyFile = '',
    [string]$ConsensusPublicKeys = '',
    [switch]$RelayServer
)

. (Join-Path $PSScriptRoot 'node-script-common.ps1')

try {
    $config = Read-ValidatorCommitteeConfig -Path $CommitteeConfig
    if ([string]::IsNullOrWhiteSpace($AuthorityId)) {
        if ($NodeId -lt 1) {
            throw 'Pass -AuthorityId (for example 0x1) or -NodeId.'
        }
        $AuthorityId = "0x$NodeId"
    }

    $entry = Get-CommitteeAuthority -Config $config -AuthorityId $AuthorityId
    $authorityIds = Get-CommitteeAuthorityIds -Config $config
    $authorities = $authorityIds -join ','

    if ([string]$config.chain_id -ne 'kanari-v2-mysticeti') {
        throw "Unsupported chain_id '$($config.chain_id)'; this binary expects kanari-v2-mysticeti."
    }
    if ([long]$config.protocol_version -ne 1) {
        throw "Unsupported protocol_version '$($config.protocol_version)'; this binary expects 1."
    }

    if ([string]::IsNullOrWhiteSpace($DataDir)) {
        $DataDir = Resolve-OperatorPath -Path ([string]$entry.data_dir) -ConfigPath $config._config_path
    }
    if ($P2pPort -le 0) {
        $P2pPort = [int]$entry.p2p_port
    }
    if ($RpcPort -le 0) {
        $RpcPort = [int]$entry.rpc_port
    }
    if ([string]::IsNullOrWhiteSpace($RpcHost)) {
        $RpcHost = if ($entry.rpc_bind_host) { [string]$entry.rpc_bind_host } else { '127.0.0.1' }
    }
    if ([string]::IsNullOrWhiteSpace($Bootstrap) -and $entry.bootstrap_multiaddr) {
        $Bootstrap = [string]$entry.bootstrap_multiaddr
    }
    if ([string]::IsNullOrWhiteSpace($ConsensusPrivateKeyFile)) {
        $ConsensusPrivateKeyFile = Resolve-OperatorPath -Path ([string]$entry.consensus_private_key_file) -ConfigPath $config._config_path
    }
    if ([string]::IsNullOrWhiteSpace($ConsensusPublicKeys)) {
        $ConsensusPublicKeys = Resolve-OperatorPath -Path ([string]$config.committee.consensus_public_keys_file) -ConfigPath $config._config_path
    }

    Test-ConsensusPrivateKeyFile -Path $ConsensusPrivateKeyFile
    Test-ConsensusPublicKeysFile -Path $ConsensusPublicKeys -AuthorityIds $authorityIds

    if (-not (Test-Path -LiteralPath $DataDir)) {
        New-Item -ItemType Directory -Path $DataDir -Force | Out-Null
    }

    $exeInfo = Find-KanariNodeExecutable
    Write-Host $exeInfo.Label -ForegroundColor $exeInfo.Color
    Write-Host '========================================' -ForegroundColor Cyan
    Write-Host "Starting Kanari validator $AuthorityId" -ForegroundColor Green
    Write-Host "Network:          $($config.network)" -ForegroundColor Yellow
    Write-Host "Chain ID:         $($config.chain_id)" -ForegroundColor Yellow
    Write-Host "Epoch:            $($config.epoch)" -ForegroundColor Yellow
    Write-Host "Protocol version: $($config.protocol_version)" -ForegroundColor Yellow
    Write-Host "P2P:              $P2pPort" -ForegroundColor Yellow
    Write-Host "RPC bind:         $RpcHost`:$RpcPort" -ForegroundColor Yellow
    Write-Host "Data dir:         $DataDir" -ForegroundColor Yellow
    Write-Host "Private key file: $ConsensusPrivateKeyFile" -ForegroundColor DarkYellow
    Write-Host "Public-key map:   $ConsensusPublicKeys" -ForegroundColor DarkYellow
    Write-Host '========================================' -ForegroundColor Cyan

    if ($RpcHost -eq '0.0.0.0') {
        Write-Warning 'RPC is exposed on all interfaces. Use a firewall and authenticated reverse proxy.'
    }

    $nodeArgs = @(
        'start',
        '--network', [string]$config.network,
        '--p2p-port', $P2pPort,
        '--rpc-port', $RpcPort,
        '--rpc-host', $RpcHost,
        '--data-dir', $DataDir,
        '--authority-id', $AuthorityId,
        '--authorities', $authorities,
        '--consensus-private-key-file', $ConsensusPrivateKeyFile,
        '--consensus-public-keys', $ConsensusPublicKeys
    )

    if (-not [string]::IsNullOrWhiteSpace($Bootstrap)) {
        $nodeArgs += @('--bootstrap', $Bootstrap)
    }
    if ($RelayServer) {
        $nodeArgs += '--relay-server'
    }

    & $exeInfo.Path @nodeArgs
    exit $LASTEXITCODE
} catch {
    Write-Host "Failed to start validator: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
