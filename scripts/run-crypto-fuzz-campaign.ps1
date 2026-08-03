param(
    [ValidateRange(0, 168)]
    [int]$Hours = 8,

    [ValidateRange(0, 10080)]
    [int]$Minutes = 0,

    [ValidateRange(1, 1000000)]
    [int]$Cases = 4096,

    [ValidateRange(1, 256)]
    [int]$Workers = 1,

    [switch]$SkipClippy,

    [switch]$SkipAudit
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$duration = if ($Minutes -gt 0) {
    [TimeSpan]::FromMinutes($Minutes)
} else {
    [TimeSpan]::FromHours($Hours)
}
$deadline = (Get-Date).Add($duration)
$iteration = 0

Write-Host "Kanari crypto fuzz/dependency campaign"
Write-Host "  deadline=$deadline duration=$duration cases=$Cases workers=$Workers"

Push-Location $repoRoot
try {
    $previousCases = $env:KANARI_CRYPTO_PROPTEST_CASES
    $env:KANARI_CRYPTO_PROPTEST_CASES = [string]$Cases

    if (-not $SkipClippy) {
        cargo clippy -p kanari-crypto --all-features -- -D warnings
    }

    if (-not $SkipAudit) {
        cargo audit -q
    }

    while ((Get-Date) -lt $deadline) {
        $iteration += 1
        Write-Host "[$(Get-Date -Format o)] crypto fuzz iteration $iteration"

        cargo test -p kanari-crypto --all-features --test fuzz_tests -- --test-threads=$Workers
        cargo test -p kanari-crypto --all-features --test key_test -- --test-threads=$Workers
        cargo test -p kanari-crypto --all-features --test signatures_test -- --test-threads=$Workers
        cargo test -p kanari-crypto --all-features --test compatibility_test -- --test-threads=$Workers
    }
} finally {
    if ($null -eq $previousCases) {
        Remove-Item Env:\KANARI_CRYPTO_PROPTEST_CASES -ErrorAction SilentlyContinue
    } else {
        $env:KANARI_CRYPTO_PROPTEST_CASES = $previousCases
    }
    Pop-Location
}

Write-Host "Kanari crypto fuzz/dependency campaign completed: $iteration iteration(s) without failure."
