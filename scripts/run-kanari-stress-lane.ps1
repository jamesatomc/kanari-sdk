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
    [string]$Rpc
)

$ErrorActionPreference = 'Stop'
$env:KANARI_KEYSTORE_PATH = $KeystorePath
$password = $env:KANARI_LOAD_PASSWORD
if ([string]::IsNullOrWhiteSpace($password)) {
    throw 'KANARI_LOAD_PASSWORD is required.'
}

& $KanariExe `
    client `
    stress-test `
    --from $Sender `
    --to $Recipient `
    --amount $Amount `
    --count $Count `
    --rpc $Rpc `
    -p $password

exit $LASTEXITCODE
