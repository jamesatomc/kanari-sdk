param(
    [Parameter(Mandatory=$true)]
    [string]$BackupDir,
    [Parameter(Mandatory=$true)]
    [string]$TargetDataDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

try {
    if (Get-Process -Name 'kanari-node' -ErrorAction SilentlyContinue) {
        throw 'Stop every kanari-node process before restoring node data.'
    }

    $backupRoot = (Resolve-Path -LiteralPath $BackupDir).Path
    $backupDataDir = Join-Path $backupRoot 'data'
    $metadataPath = Join-Path $backupRoot 'backup-metadata.json'
    $manifestPath = Join-Path $backupRoot 'file-manifest.json'

    foreach ($path in @($backupDataDir, $metadataPath, $manifestPath)) {
        if (-not (Test-Path -LiteralPath $path)) {
            throw "Required backup component not found: $path"
        }
    }

    $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
    if (($metadata.schema_version -as [int]) -ne 2) {
        throw 'Unsupported backup schema. Expected schema_version 2.'
    }

    $manifest = @(Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json)
    foreach ($entry in $manifest) {
        $sourceFile = Join-Path $backupDataDir ([string]$entry.path)
        if (-not (Test-Path -LiteralPath $sourceFile -PathType Leaf)) {
            throw "Backup file is missing: $($entry.path)"
        }
        $actualHash = (Get-FileHash -LiteralPath $sourceFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -ne [string]$entry.sha256) {
            throw "Backup checksum mismatch: $($entry.path)"
        }
    }

    if (Test-Path -LiteralPath $TargetDataDir) {
        $existing = Get-ChildItem -LiteralPath $TargetDataDir -Force -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($existing) {
            throw "Target data directory is not empty: $TargetDataDir. Restore to a fresh directory."
        }
    } else {
        New-Item -ItemType Directory -Path $TargetDataDir -Force | Out-Null
    }

    Copy-Item -Path (Join-Path $backupDataDir '*') -Destination $TargetDataDir -Recurse -Force -ErrorAction Stop

    foreach ($entry in $manifest) {
        $restoredFile = Join-Path $TargetDataDir ([string]$entry.path)
        $restoredHash = (Get-FileHash -LiteralPath $restoredFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($restoredHash -ne [string]$entry.sha256) {
            throw "Restored file checksum mismatch: $($entry.path)"
        }
    }

    Write-Host "Restore completed to $TargetDataDir" -ForegroundColor Green
    Write-Host "Authority: $($metadata.authority_id) | network=$($metadata.network) | height=$($metadata.checkpoint_height) | root=$($metadata.checkpoint_state_root)" -ForegroundColor Cyan
    Write-Host 'Start the validator with the same committee manifest and consensus key files, then run monitor-cluster-health.ps1.' -ForegroundColor Yellow
} catch {
    Write-Host "Restore failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
