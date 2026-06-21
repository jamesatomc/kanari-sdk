# Create and optionally start a local Kanari validator committee.
# This script never deletes existing node data or private keys.
param(
    [ValidateRange(1, 200)]
    [int]$NodeCount = 4,
    [ValidateSet('mainnet', 'testnet', 'devnet')]
    [string]$Network = 'devnet',
    [string]$ChainId = 'kanari-v2-mysticeti',
    [long]$Epoch = 0,
    [long]$ProtocolVersion = 1,
    [string]$DataRoot = "$env:USERPROFILE\.kanari\clusters\devnet-v2",
    [string]$ConsensusKeyDir = "$env:USERPROFILE\.kanari\consensus-keys\devnet-v2",
    [string]$CommitteeConfig = '',
    [int]$BasePeerPort = 19000,
    [int]$BaseRpcPort = 19001,
    [string]$RpcHost = '127.0.0.1',
    [switch]$StartNodes,
    [switch]$SkipHealthCheck
)

. (Join-Path $PSScriptRoot 'node-script-common.ps1')

function Test-DirectoryHasEntries([string]$Path) {
    return (Test-Path -LiteralPath $Path) -and
        ($null -ne (Get-ChildItem -LiteralPath $Path -Force -ErrorAction SilentlyContinue | Select-Object -First 1))
}

try {
    if ($ChainId -ne 'kanari-v2-mysticeti') {
        throw 'Current node runtime requires chain_id kanari-v2-mysticeti.'
    }
    if ($ProtocolVersion -ne 1) {
        throw 'Current checkpoint certificate format requires protocol_version 1.'
    }
    if ($RpcHost -eq '0.0.0.0') {
        Write-Warning 'RPC will be exposed on all interfaces. Use firewall and gateway controls.'
    }

    if ([string]::IsNullOrWhiteSpace($CommitteeConfig)) {
        $CommitteeConfig = Join-Path $DataRoot 'validator-committee.local.json'
    }

    for ($i = 1; $i -le $NodeCount; $i++) {
        $nodeDir = Join-Path $DataRoot "node$i"
        if (Test-DirectoryHasEntries $nodeDir) {
            throw "Node data already exists at $nodeDir. Choose a fresh -DataRoot or restore intentionally with restore-node-data.ps1."
        }
    }

    foreach ($directory in @($DataRoot, $ConsensusKeyDir)) {
        if (-not (Test-Path -LiteralPath $directory)) {
            New-Item -ItemType Directory -Path $directory -Force | Out-Null
        }
    }

    $exeInfo = Find-KanariNodeExecutable
    $publicKeysPath = Join-Path $ConsensusKeyDir 'consensus-public-keys.json'
    $missingKeys = -not (Test-Path -LiteralPath $publicKeysPath)
    for ($i = 1; $i -le $NodeCount; $i++) {
        if (-not (Test-Path -LiteralPath (Join-Path $ConsensusKeyDir "node$i-consensus-private-key.hex"))) {
            $missingKeys = $true
        }
    }

    if ($missingKeys) {
        if (Test-DirectoryHasEntries $ConsensusKeyDir) {
            throw "Consensus key directory is incomplete: $ConsensusKeyDir. Use a new directory instead of overwriting keys."
        }
        & $exeInfo.Path consensus-keygen --node-count $NodeCount --output-dir $ConsensusKeyDir
        if ($LASTEXITCODE -ne 0) { throw 'Consensus key generation failed.' }
    }

    $authorityIds = @(1..$NodeCount | ForEach-Object { "0x$_" })
    Test-ConsensusPublicKeysFile -Path $publicKeysPath -AuthorityIds $authorityIds
    $quorum = [math]::Floor((2 * $NodeCount) / 3) + 1
    $lanIp = Get-LanIpAddress
    if ([string]::IsNullOrWhiteSpace($lanIp)) { $lanIp = '127.0.0.1' }

    $authorities = @()
    for ($i = 1; $i -le $NodeCount; $i++) {
        $ports = Get-NodePorts -NodeId $i -BasePeerPort $BasePeerPort -BaseRpcPort $BaseRpcPort
        $dataDir = Join-Path $DataRoot "node$i"
        $bootstrap = if ($i -eq 1) { $null } else { "/ip4/$lanIp/tcp/$BasePeerPort" }
        $authorities += [ordered]@{
            authority_id = "0x$i"
            name = "validator-$i"
            p2p_host = $lanIp
            p2p_port = $ports.P2pPort
            rpc_bind_host = $RpcHost
            rpc_port = $ports.RpcPort
            rpc_url = Get-NodeRpcUrl -HostIp $RpcHost -RpcPort $ports.RpcPort
            bootstrap_multiaddr = $bootstrap
            data_dir = [System.IO.Path]::GetFullPath($dataDir)
            consensus_private_key_file = [System.IO.Path]::GetFullPath((Join-Path $ConsensusKeyDir "node$i-consensus-private-key.hex"))
        }
    }

    $manifest = [ordered]@{
        schema_version = 2
        network = $Network
        chain_id = $ChainId
        epoch = $Epoch
        protocol_version = $ProtocolVersion
        committee = [ordered]@{
            authority_ids = $authorityIds
            quorum_model = '2f+1-authority-count'
            minimum_signatures = $quorum
            consensus_public_keys_file = [System.IO.Path]::GetFullPath($publicKeysPath)
        }
        checkpoint_certificates = [ordered]@{
            domain = 'kanari:checkpoint-certificate:v1'
            required_for_synced_checkpoints = $true
        }
        authorities = $authorities
        rollout = [ordered]@{
            source_validator = '0x1'
            minimum_cluster_size = $NodeCount
            require_equal_supply = $true
            require_equal_height = $true
            require_equal_state_root = $true
        }
    }

    $manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $CommitteeConfig -Encoding UTF8
    $config = Read-ValidatorCommitteeConfig -Path $CommitteeConfig
    Write-Host "Committee manifest: $($config._config_path)" -ForegroundColor Green
    Write-Host "Authorities: $($authorityIds -join ',') | quorum=$quorum | chain=$ChainId | epoch=$Epoch" -ForegroundColor Cyan

    if (-not $StartNodes) {
        Write-Host 'Configuration complete. Re-run with -StartNodes to launch the committee.' -ForegroundColor Yellow
        exit 0
    }

    $startScript = Join-Path $PSScriptRoot 'start-node.ps1'
    $powerShell = (Get-Process -Id $PID).Path
    for ($i = 1; $i -le $NodeCount; $i++) {
        $authorityId = "0x$i"
        $arguments = "-NoExit -ExecutionPolicy Bypass -File `"$startScript`" -CommitteeConfig `"$CommitteeConfig`" -AuthorityId `"$authorityId`""
        Start-Process -FilePath $powerShell -ArgumentList $arguments -WindowStyle Normal
        Start-Sleep -Seconds $(if ($i -eq 1) { 5 } else { 1 })
    }

    if (-not $SkipHealthCheck) {
        Start-Sleep -Seconds 3
        $index = 0
        foreach ($entry in $config.authorities) {
            $index++
            Test-NodeHealth -RpcUrl ([string]$entry.rpc_url) -NodeId $index -ExpectedNetwork $Network -ExpectedAuthorityId ([string]$entry.authority_id)
        }
    }
} catch {
    Write-Host "Multi-node setup failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
