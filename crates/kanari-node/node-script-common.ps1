Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-LanIpAddress {
    $adapters = Get-NetAdapter -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Status -eq 'Up' -and
            $_.InterfaceDescription -notmatch 'Virtual|vEthernet|Hyper-V|Docker|VMware|Loopback'
        }

    foreach ($adapter in $adapters) {
        $ip = Get-NetIPAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object {
                $_.IPAddress -notmatch '^127\.' -and
                $_.IPAddress -notmatch '^169\.254\.'
            } |
            Select-Object -First 1
        if ($ip) {
            return $ip.IPAddress
        }
    }

    return (
        Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object {
                $_.IPAddress -notmatch '^127\.' -and
                $_.IPAddress -notmatch '^169\.254\.'
            } |
            Select-Object -First 1
    ).IPAddress
}

function Resolve-OperatorPath {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Path,
        [Parameter(Mandatory=$true)]
        [string]$ConfigPath
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }

    $configDirectory = Split-Path -Parent ([System.IO.Path]::GetFullPath($ConfigPath))
    return [System.IO.Path]::GetFullPath((Join-Path $configDirectory $Path))
}

function Read-ValidatorCommitteeConfig {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Validator committee config not found: $Path"
    }

    $resolvedPath = (Resolve-Path -LiteralPath $Path).Path
    $config = Get-Content -LiteralPath $resolvedPath -Raw | ConvertFrom-Json

    if (($config.schema_version -as [int]) -lt 2) {
        throw 'Committee config schema_version must be 2 or newer.'
    }
    if ([string]::IsNullOrWhiteSpace([string]$config.network)) {
        throw 'Committee config is missing network.'
    }
    if ([string]::IsNullOrWhiteSpace([string]$config.chain_id)) {
        throw 'Committee config is missing chain_id.'
    }
    if (($config.protocol_version -as [long]) -lt 1) {
        throw 'Committee config protocol_version must be at least 1.'
    }
    if (-not $config.committee -or -not $config.committee.authority_ids) {
        throw 'Committee config is missing committee.authority_ids.'
    }
    if (-not $config.authorities -or $config.authorities.Count -eq 0) {
        throw 'Committee config must contain at least one authority.'
    }

    $authorityIds = @($config.committee.authority_ids | ForEach-Object { [string]$_ })
    $uniqueAuthorityIds = @($authorityIds | Sort-Object -Unique)
    if ($uniqueAuthorityIds.Count -ne $authorityIds.Count) {
        throw 'Committee config contains duplicate authority IDs.'
    }

    $entries = @($config.authorities)
    $entryIds = @($entries | ForEach-Object { [string]$_.authority_id })
    if (@($entryIds | Sort-Object -Unique).Count -ne $entries.Count) {
        throw 'Committee authority entries contain duplicate authority IDs.'
    }

    foreach ($authorityId in $authorityIds) {
        if ($entryIds -notcontains $authorityId) {
            throw "Committee authority $authorityId has no matching authority entry."
        }
    }

    $requiredSignatures = [int]$config.committee.minimum_signatures
    if ($requiredSignatures -lt 1 -or $requiredSignatures -gt $authorityIds.Count) {
        throw 'committee.minimum_signatures is outside the authority set size.'
    }

    $config | Add-Member -NotePropertyName '_config_path' -NotePropertyValue $resolvedPath -Force
    return $config
}

function Get-CommitteeAuthorityIds {
    param([Parameter(Mandatory=$true)]$Config)
    return @($Config.committee.authority_ids | ForEach-Object { [string]$_ })
}

function Get-CommitteeAuthority {
    param(
        [Parameter(Mandatory=$true)]$Config,
        [Parameter(Mandatory=$true)][string]$AuthorityId
    )

    $entry = @($Config.authorities | Where-Object { [string]$_.authority_id -eq $AuthorityId })
    if ($entry.Count -ne 1) {
        throw "Expected exactly one committee entry for authority $AuthorityId."
    }
    return $entry[0]
}

function Test-ConsensusPrivateKeyFile {
    param([Parameter(Mandatory=$true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Consensus private key file not found: $Path"
    }
    $value = (Get-Content -LiteralPath $Path -Raw).Trim()
    if ($value -notmatch '^[0-9a-fA-F]{64}$') {
        throw "Consensus private key file must contain exactly 32 bytes (64 hexadecimal characters): $Path"
    }
}

function Test-ConsensusPublicKeysFile {
    param(
        [Parameter(Mandatory=$true)][string]$Path,
        [Parameter(Mandatory=$true)][string[]]$AuthorityIds
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Consensus public-key map not found: $Path"
    }

    $map = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    $properties = @($map.PSObject.Properties)
    foreach ($authorityId in $AuthorityIds) {
        $property = $map.PSObject.Properties[$authorityId]
        if ($null -eq $property) {
            throw "Consensus public-key map is missing authority $authorityId."
        }
        if ([string]$property.Value -notmatch '^[0-9a-fA-F]{64}$') {
            throw "Consensus public key for $authorityId must be 32 bytes of hexadecimal data."
        }
    }
    if ($properties.Count -ne $AuthorityIds.Count) {
        throw 'Consensus public-key map contains an authority set different from the committee config.'
    }
}

function Get-NodePorts {
    param(
        [int]$NodeId,
        [int]$BasePeerPort,
        [int]$BaseRpcPort
    )

    $offset = ($NodeId - 1) * 10
    return @{ P2pPort = $BasePeerPort + $offset; RpcPort = $BaseRpcPort + $offset }
}

function Get-NodeDataDir {
    param([int]$NodeId, [string]$DataDir, [string]$BaseDataDir)
    if (-not [string]::IsNullOrWhiteSpace($DataDir)) { return $DataDir }
    return (Join-Path $BaseDataDir "node$NodeId")
}

function Get-NodeRpcUrl {
    param([string]$HostIp, [int]$RpcPort)
    if (-not [string]::IsNullOrWhiteSpace($HostIp) -and $HostIp -ne '0.0.0.0') {
        return "http://$HostIp`:$RpcPort"
    }
    return "http://127.0.0.1:$RpcPort"
}

function Find-KanariNodeExecutable {
    $localBuilds = @(
        @{ Path = Join-Path $PSScriptRoot '..\..\target\release\kanari-node.exe'; Kind = 'release'; Color = 'Green' },
        @{ Path = Join-Path $PSScriptRoot '..\..\target\debug\kanari-node.exe'; Kind = 'debug'; Color = 'Yellow' }
    ) | Where-Object { Test-Path $_.Path } | ForEach-Object {
        $resolvedPath = (Resolve-Path $_.Path).Path
        @{ Path = $resolvedPath; Kind = $_.Kind; Color = $_.Color; LastWriteTimeUtc = (Get-Item $resolvedPath).LastWriteTimeUtc }
    }

    if ($localBuilds.Count -gt 0) {
        $selected = $localBuilds | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        return @{ Path = $selected.Path; Label = "Using newest local $($selected.Kind) build: $($selected.Path)"; Color = $selected.Color }
    }
    if (Get-Command kanari-node -ErrorAction SilentlyContinue) {
        return @{ Path = 'kanari-node'; Label = 'Using kanari-node from PATH'; Color = 'Green' }
    }
    throw 'kanari-node executable not found. Build it with cargo build -p kanari-node --release.'
}

function Invoke-KanariJsonRpc {
    param(
        [Parameter(Mandatory=$true)][string]$RpcUrl,
        [Parameter(Mandatory=$true)][string]$Method,
        [object]$Params = @{},
        [int]$RequestId = 1
    )

    $body = @{ jsonrpc = '2.0'; method = $Method; params = $Params; id = $RequestId } | ConvertTo-Json -Depth 12
    $response = Invoke-RestMethod -Uri $RpcUrl -Method Post -Body $body -ContentType 'application/json' -TimeoutSec 10
    if ($response.error) { throw "JSON-RPC $Method failed: $($response.error.message)" }
    return $response.result
}

function Get-NodeHealthStatus { param([string]$RpcUrl) return Invoke-KanariJsonRpc -RpcUrl $RpcUrl -Method 'kanari_health' -RequestId 1 }
function Get-NodeStats { param([string]$RpcUrl) return Invoke-KanariJsonRpc -RpcUrl $RpcUrl -Method 'kanari_getStats' -RequestId 2 }
function Get-NodeNetworkStatus { param([string]$RpcUrl) return Invoke-KanariJsonRpc -RpcUrl $RpcUrl -Method 'kanari_getNetworkStatus' -RequestId 3 }

function Test-NodeHealth {
    param(
        [Parameter(Mandatory=$true)][string]$RpcUrl,
        [Parameter(Mandatory=$true)][int]$NodeId,
        [string]$ExpectedNetwork = '',
        [string]$ExpectedAuthorityId = '',
        [switch]$RequireBootstrappedState
    )

    $health = Get-NodeHealthStatus -RpcUrl $RpcUrl
    $stats = Get-NodeStats -RpcUrl $RpcUrl
    $networkStatus = Get-NodeNetworkStatus -RpcUrl $RpcUrl

    if ($health.status -ne 'ok' -or -not $health.supply_invariants_ok) {
        throw "Node $NodeId is unhealthy: $($health.supply_invariant_error)"
    }
    if ($ExpectedNetwork -and $health.network -ne $ExpectedNetwork) {
        throw "Node $NodeId reports network $($health.network), expected $ExpectedNetwork."
    }
    if ($ExpectedAuthorityId -and $networkStatus.local_authority_id -ne $ExpectedAuthorityId) {
        throw "Node $NodeId reports authority $($networkStatus.local_authority_id), expected $ExpectedAuthorityId."
    }
    if ($RequireBootstrappedState -and ([long]$stats.total_supply -le 0)) {
        throw "Node $NodeId reports total_supply=0 after startup."
    }

    Write-Host "Node $NodeId OK | authority=$($networkStatus.local_authority_id) | network=$($health.network) | height=$($stats.height) | supply=$($stats.total_supply) | root=$($stats.state_root) | $RpcUrl" -ForegroundColor Green
    return @{ Health = $health; Stats = $stats; Network = $networkStatus }
}
