param(
    [switch]$NoFetch
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
Push-Location $repoRoot
try {
    $auditArgs = @(
        'audit',
        '--ignore', 'RUSTSEC-2026-0118', # hickory-proto optional mDNS lock entry; production default disables p2p-mdns.
        '--ignore', 'RUSTSEC-2026-0119', # hickory-proto optional mDNS lock entry; production default disables p2p-mdns.
        '--ignore', 'RUSTSEC-2023-0071'  # rsa Marvin; Kanari runtime uses public-key RS256 verification only, no private-key RSA ops.
    )
    if ($NoFetch) {
        $auditArgs += '--no-fetch'
    }

    cargo @auditArgs
    cargo clippy --workspace --all-targets --quiet -- -D warnings
}
finally {
    Pop-Location
}
