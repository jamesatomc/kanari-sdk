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

function Get-NodePorts {
    param(
        [int]$NodeId,
        [int]$BasePeerPort,
        [int]$BaseRpcPort
    )

    $offset = ($NodeId - 1) * 10
    return @{
        P2pPort = $BasePeerPort + $offset
        RpcPort = $BaseRpcPort + $offset
    }
}

function Get-NodeDataDir {
    param(
        [int]$NodeId,
        [string]$DataDir,
        [string]$BaseDataDir
    )

    if ($DataDir -ne "") {
        return $DataDir
    }

    return (Join-Path $BaseDataDir "node$NodeId")
}

function Get-NodeRpcUrl {
    param(
        [string]$HostIp,
        [int]$RpcPort
    )

    if ($HostIp) {
        return "http://$HostIp`:$RpcPort"
    }

    return "http://127.0.0.1:$RpcPort"
}

function Get-RpcConnectHost {
    param(
        [string]$RpcHost,
        [string]$LanIp
    )

    if ([string]::IsNullOrWhiteSpace($RpcHost)) {
        return "127.0.0.1"
    }

    if ($RpcHost -eq "0.0.0.0" -or $RpcHost -eq "::") {
        if ($LanIp) {
            return $LanIp
        }
        return "127.0.0.1"
    }

    if ($RpcHost -eq "localhost") {
        return "127.0.0.1"
    }

    return $RpcHost
}

function Find-KanariNodeExecutable {
    $localBuilds = @(
        @{
            Path = Join-Path $PSScriptRoot "..\..\target\release\kanari-node.exe"
            Kind = "release"
            Color = "Green"
        },
        @{
            Path = Join-Path $PSScriptRoot "..\..\target\debug\kanari-node.exe"
            Kind = "debug"
            Color = "Yellow"
        }
    ) | Where-Object { Test-Path $_.Path } | ForEach-Object {
        $resolvedPath = (Resolve-Path $_.Path).Path
        @{
            Path = $resolvedPath
            Kind = $_.Kind
            Color = $_.Color
            LastWriteTimeUtc = (Get-Item $resolvedPath).LastWriteTimeUtc
        }
    }

    if ($localBuilds.Count -gt 0) {
        $selected = $localBuilds | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        return @{
            Path = $selected.Path
            Label = "Using newest local $($selected.Kind) build: $($selected.Path) (built $($selected.LastWriteTimeUtc.ToString('u')))"
            Color = $selected.Color
        }
    }

    if (Get-Command kanari-node -ErrorAction SilentlyContinue) {
        return @{
            Path = "kanari-node"
            Label = "Using kanari-node from PATH"
            Color = "Green"
        }
    }

    throw "kanari-node executable not found"
}

function Invoke-KanariJsonRpc {
    param(
        [string]$RpcUrl,
        [string]$Method,
        [hashtable]$Params = @{},
        [int]$RequestId = 1
    )

    $body = @{
        jsonrpc = "2.0"
        method = $Method
        params = $Params
        id = $RequestId
    } | ConvertTo-Json -Depth 8

    return Invoke-RestMethod -Uri $RpcUrl -Method Post -Body $body -ContentType "application/json" -TimeoutSec 5
}

function Get-NodeHealthStatus {
    param(
        [string]$RpcUrl
    )

    $response = Invoke-KanariJsonRpc -RpcUrl $RpcUrl -Method "kanari_health" -RequestId 1
    return $response.result
}

function Get-NodeStats {
    param(
        [string]$RpcUrl
    )

    $response = Invoke-KanariJsonRpc -RpcUrl $RpcUrl -Method "kanari_getStats" -RequestId 2
    return $response.result
}

function Test-NodeHealth {
    param(
        [string]$RpcUrl,
        [int]$NodeId,
        [switch]$RequireBootstrappedState
    )

    try {
        $health = Get-NodeHealthStatus -RpcUrl $RpcUrl

        if (-not $health) {
            Write-Host "Node $NodeId health check returned no result." -ForegroundColor Yellow
            return
        }

        $status = $health.status
        $network = $health.network
        $supplyOk = $health.supply_invariants_ok
        $failFast = $health.fail_fast_enabled
        $strictPersistence = $health.strict_persistence_required
        $strictRoots = $health.strict_checkpoint_roots
        $persistentStorage = $health.persistent_storage_available

        if ($status -eq "ok" -and $supplyOk) {
            Write-Host "Node $NodeId health OK | network=$network | fail-fast=$failFast | strict-persistence=$strictPersistence | strict-roots=$strictRoots | persisted=$persistentStorage | $RpcUrl" -ForegroundColor Green
        } else {
            $detail = $health.supply_invariant_error
            Write-Host "Node $NodeId health DEGRADED | network=$network | fail-fast=$failFast | strict-persistence=$strictPersistence | strict-roots=$strictRoots | persisted=$persistentStorage | $RpcUrl" -ForegroundColor Yellow
            if ($detail) {
                Write-Host "  Supply invariant error: $detail" -ForegroundColor Yellow
            }
        }

        if ($RequireBootstrappedState) {
            $stats = Get-NodeStats -RpcUrl $RpcUrl

            if (-not $stats) {
                throw "Node $NodeId stats request returned no result."
            }

            if (($stats.total_supply -as [long]) -le 0) {
                throw "Node $NodeId reports total_supply=0 after startup."
            }

            Write-Host "Node $NodeId stats OK | total_supply=$($stats.total_supply) | accounts=$($stats.total_accounts)" -ForegroundColor Green
        }
    } catch {
        Write-Host "Node $NodeId health check failed at $RpcUrl : $($_.Exception.Message)" -ForegroundColor Yellow
    }
}
