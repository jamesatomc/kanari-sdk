param(
    [Parameter(Mandatory=$true)]
    [string]$SourceDataDir,
    [string]$BackupRoot = "$env:USERPROFILE\.kanari\backups",
    [string]$Label = 'node-backup',
    [Nullable[long]]$CheckpointHeight = $null,
    [string]$CheckpointStateRoot = '',
    [string]$Network = '',
    [string]$AuthorityId = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

try {
    if (Get-Process -Name 'kanari-node' -ErrorAction SilentlyContinue) {
        throw 'Stop every kanari-node process before creating a filesystem backup.'
    }

    $source = (Resolve-Path -LiteralPath $SourceDataDir).Path
    if (-not (Test-Path -LiteralPath $BackupRoot)) {
        New-Item -ItemType Directory -Path $BackupRoot -Force | Out-Null
    }

    $backupDir = Join-Path $BackupRoot ("{0}-{1}" -f $Label, (Get-Date -Format 'yyyyMMdd-HHmmss'))
    $dataDir = Join-Path $backupDir 'data'
    New-Item -ItemType Directory -Path $dataDir -Force | Out-Null
    Copy-Item -Path (Join-Path $source '*') -Destination $dataDir -Recurse -Force -ErrorAction Stop

    $manifest = @()
    Get-ChildItem -LiteralPath $dataDir -Recurse -File | Sort-Object FullName | ForEach-Object {
        $manifest += [ordered]@{
            path = $_.FullName.Substring($dataDir.Length).TrimStart('\')
            length = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $backupDir 'file-manifest.json') -Encoding UTF8

    $metadata = [ordered]@{
        schema_version = 2
        created_at = (Get-Date).ToUniversalTime().ToString('o')
        source_data_dir = $source
        host = $env:COMPUTERNAME
        file_count = $manifest.Count
        checkpoint_height = $CheckpointHeight
        checkpoint_state_root = if ([string]::IsNullOrWhiteSpace($CheckpointStateRoot)) { $null } else { $CheckpointStateRoot }
        network = if ([string]::IsNullOrWhiteSpace($Network)) { $null } else { $Network }
        authority_id = if ([string]::IsNullOrWhiteSpace($AuthorityId)) { $null } else { $AuthorityId }
        node_was_stopped = $true
    }
    $metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $backupDir 'backup-metadata.json') -Encoding UTF8

    Write-Host "Backup completed: $backupDir" -ForegroundColor Green
    Write-Host "Files: $($manifest.Count) | height=$CheckpointHeight | root=$CheckpointStateRoot" -ForegroundColor Cyan
} catch {
    Write-Host "Backup failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
