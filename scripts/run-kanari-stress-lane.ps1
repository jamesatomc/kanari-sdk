param(
    [Parameter(Mandatory = $true)]
    [string]$KanariExe,

    [Parameter(Mandatory = $true)]
    [string]$KeystorePath,

    [Parameter(Mandatory = $true)]
    [string]$Sender,

    [Parameter(Mandatory = $true)]
    [string]$Recipient,

    [Parameter(Mandatory = $true)]
    [double]$Amount,

    [Parameter(Mandatory = $true)]
    [int]$Count,

    [Parameter(Mandatory = $true)]
    [string]$Rpc,

    # Alternate replicas for transport-only failover. Transaction validation
    # errors are never retried through a different endpoint.
    # Comma-separated so Start-Process forwards it as one argument on Windows.
    [string]$RpcFallbackCsv = '',

    [ValidateRange(20, 7200)]
    [int]$CommitTimeoutSec = 60
)

$ErrorActionPreference = 'Stop'
$env:KANARI_KEYSTORE_PATH = $KeystorePath
$password = $env:KANARI_LOAD_PASSWORD
if ([string]::IsNullOrWhiteSpace($password)) {
    throw 'KANARI_LOAD_PASSWORD is required.'
}

$arguments = @(
    'client',
    'stress-test',
    '--from', $Sender,
    '--to', $Recipient,
    '--amount', $Amount,
    '--count', $Count,
    '--commit-timeout-sec', $CommitTimeoutSec,
    '--rpc', $Rpc,
    '-p', $password
)
foreach ($endpoint in ($RpcFallbackCsv -split ',')) {
    if (-not [string]::IsNullOrWhiteSpace($endpoint) -and $endpoint -ne $Rpc) {
        $arguments += @('--rpc-fallback', $endpoint)
    }
}

& $KanariExe @arguments

exit $LASTEXITCODE
