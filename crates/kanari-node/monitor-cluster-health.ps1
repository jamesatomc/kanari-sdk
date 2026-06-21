param(
    [string]$CommitteeConfig = '',
    [string[]]$RpcUrls = @(),
    [string]$ExpectedNetwork = '',
    [switch]$RequireEqualHeight,
    [switch]$RequireEqualSupply,
    [switch]$RequireEqualStateRoot
)

. (Join-Path $PSScriptRoot 'node-script-common.ps1')

try {
    $targets = @()
    $expectedAuthorityIds = @()

    if (-not [string]::IsNullOrWhiteSpace($CommitteeConfig)) {
        $config = Read-ValidatorCommitteeConfig -Path $CommitteeConfig
        if ([string]::IsNullOrWhiteSpace($ExpectedNetwork)) {
            $ExpectedNetwork = [string]$config.network
        }
        $expectedAuthorityIds = Get-CommitteeAuthorityIds -Config $config
        foreach ($entry in $config.authorities) {
            $targets += [ordered]@{
                AuthorityId = [string]$entry.authority_id
                RpcUrl = [string]$entry.rpc_url
            }
        }
        if ($config.rollout.require_equal_height) { $RequireEqualHeight = $true }
        if ($config.rollout.require_equal_supply) { $RequireEqualSupply = $true }
        if ($config.rollout.require_equal_state_root) { $RequireEqualStateRoot = $true }
    } elseif ($RpcUrls.Count -gt 0) {
        for ($i = 0; $i -lt $RpcUrls.Count; $i++) {
            $targets += [ordered]@{ AuthorityId = ''; RpcUrl = $RpcUrls[$i] }
        }
    } else {
        throw 'Pass -CommitteeConfig or at least one -RpcUrls value.'
    }

    $failures = @()
    $heights = @()
    $supplies = @()
    $roots = @()
    $reportedAuthorities = @()

    for ($i = 0; $i -lt $targets.Count; $i++) {
        $target = $targets[$i]
        $nodeId = $i + 1
        try {
            $result = Test-NodeHealth `
                -RpcUrl $target.RpcUrl `
                -NodeId $nodeId `
                -ExpectedNetwork $ExpectedNetwork `
                -ExpectedAuthorityId $target.AuthorityId

            $heights += [long]$result.Stats.height
            $supplies += [decimal]$result.Stats.total_supply
            $roots += [string]$result.Stats.state_root
            $reportedAuthorities += [string]$result.Network.local_authority_id
        } catch {
            $failures += "Node $nodeId ($($target.RpcUrl)): $($_.Exception.Message)"
        }
    }

    if ($RequireEqualHeight -and @($heights | Sort-Object -Unique).Count -ne 1) {
        $failures += "Height mismatch: $(@($heights | Sort-Object -Unique) -join ', ')"
    }
    if ($RequireEqualSupply -and @($supplies | Sort-Object -Unique).Count -ne 1) {
        $failures += "Supply mismatch: $(@($supplies | Sort-Object -Unique) -join ', ')"
    }
    if ($RequireEqualStateRoot -and @($roots | Sort-Object -Unique).Count -ne 1) {
        $failures += "State-root mismatch: $(@($roots | Sort-Object -Unique) -join ', ')"
    }

    if ($expectedAuthorityIds.Count -gt 0) {
        $expected = @($expectedAuthorityIds | Sort-Object)
        $reported = @($reportedAuthorities | Sort-Object)
        if (($expected -join ',') -ne ($reported -join ',')) {
            $failures += "Authority set mismatch: expected [$($expected -join ', ')], reported [$($reported -join ', ')]"
        }
    }

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            Write-Host $failure -ForegroundColor Red
        }
        throw "Cluster health check failed with $($failures.Count) issue(s)."
    }

    Write-Host "Cluster health check passed for $($targets.Count) validator(s)." -ForegroundColor Green
    Write-Host 'Checkpoint certificates are validated during P2P checkpoint sync; certificate status is not currently exposed by JSON-RPC.' -ForegroundColor DarkGray
} catch {
    Write-Host $_.Exception.Message -ForegroundColor Red
    exit 1
}
