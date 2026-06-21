# Verify transaction broadcast indirectly through checkpoint convergence.
param(
    [Parameter(Mandatory=$true)]
    [string]$CommitteeConfig,
    [scriptblock]$SubmitAction,
    [ValidateRange(5, 600)]
    [int]$TimeoutSeconds = 60,
    [ValidateRange(1, 30)]
    [int]$PollSeconds = 2
)

. (Join-Path $PSScriptRoot 'node-script-common.ps1')

try {
    $config = Read-ValidatorCommitteeConfig -Path $CommitteeConfig
    $targets = @($config.authorities)
    if ($targets.Count -lt 2) {
        throw 'P2P convergence testing requires at least two validators.'
    }

    $initial = @()
    for ($i = 0; $i -lt $targets.Count; $i++) {
        $entry = $targets[$i]
        $result = Test-NodeHealth `
            -RpcUrl ([string]$entry.rpc_url) `
            -NodeId ($i + 1) `
            -ExpectedNetwork ([string]$config.network) `
            -ExpectedAuthorityId ([string]$entry.authority_id)
        $initial += $result.Stats
    }

    $initialSourceHeight = [long]$initial[0].height
    if ($SubmitAction) {
        Write-Host 'Executing caller-provided transaction submission action...' -ForegroundColor Cyan
        & $SubmitAction
    } else {
        Write-Host 'No -SubmitAction supplied; checking current checkpoint convergence only.' -ForegroundColor Yellow
    }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $stats = @()
        $allReachable = $true
        foreach ($entry in $targets) {
            try {
                $stats += Get-NodeStats -RpcUrl ([string]$entry.rpc_url)
            } catch {
                $allReachable = $false
                break
            }
        }

        if ($allReachable) {
            $heights = @($stats | ForEach-Object { [long]$_.height })
            $roots = @($stats | ForEach-Object { [string]$_.state_root })
            $supplies = @($stats | ForEach-Object { [decimal]$_.total_supply })
            $heightConverged = @($heights | Sort-Object -Unique).Count -eq 1
            $rootConverged = @($roots | Sort-Object -Unique).Count -eq 1
            $supplyConverged = @($supplies | Sort-Object -Unique).Count -eq 1
            $sourceAdvanced = (-not $SubmitAction) -or ($heights[0] -gt $initialSourceHeight)

            Write-Host "heights=[$($heights -join ',')] roots=$(@($roots | Sort-Object -Unique).Count) supplies=$(@($supplies | Sort-Object -Unique).Count)" -ForegroundColor DarkGray

            if ($heightConverged -and $rootConverged -and $supplyConverged -and $sourceAdvanced) {
                Write-Host 'P2P checkpoint convergence passed.' -ForegroundColor Green
                Write-Host 'Inbound synced checkpoints are accepted only after checkpoint-certificate verification.' -ForegroundColor Green
                exit 0
            }
        }

        Start-Sleep -Seconds $PollSeconds
    } while ((Get-Date) -lt $deadline)

    throw "Cluster did not converge within $TimeoutSeconds seconds. Check P2P logs, bootstrap addresses, committee keys and certificate verification errors."
} catch {
    Write-Host "P2P convergence test failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
