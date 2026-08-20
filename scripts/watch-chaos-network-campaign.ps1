param(
    [string]$StatusPath = (Join-Path $PSScriptRoot '..\.codex-runlogs\chaos-campaign-status.json'),

    [ValidateRange(15, 3600)]
    [int]$MaxHeartbeatAgeSec = 180
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $StatusPath -PathType Leaf)) {
    throw "Chaos campaign status file was not found: $StatusPath"
}

$status = Get-Content -LiteralPath $StatusPath -Raw | ConvertFrom-Json
$heartbeat = [DateTimeOffset]::Parse($status.heartbeat_at)
$ageSeconds = [Math]::Floor(((Get-Date).ToUniversalTime() - $heartbeat.UtcDateTime).TotalSeconds)
$process = Get-Process -Id $status.pid -ErrorAction SilentlyContinue

Write-Host "Chaos campaign state=$($status.state) phase=$($status.phase) iteration=$($status.iteration) heartbeat_age_sec=$ageSeconds pid=$($status.pid) process_alive=$($null -ne $process)"

if ($status.state -eq 'completed') {
    exit 0
}
if ($status.state -eq 'failed') {
    throw "Chaos campaign failed: $($status.terminal_error)"
}
if ($null -eq $process) {
    throw "Chaos campaign process $($status.pid) disappeared without a terminal status; last phase=$($status.phase) heartbeat=$($status.heartbeat_at)"
}
if ($ageSeconds -gt $MaxHeartbeatAgeSec) {
    throw "Chaos campaign heartbeat is stale ($ageSeconds sec > $MaxHeartbeatAgeSec sec); last phase=$($status.phase)"
}
