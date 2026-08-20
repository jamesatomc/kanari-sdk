param(
    [ValidateRange(10000, 1000000)]
    [int]$Transactions = 10000,

    [ValidateRange(1, 1000000)]
    [int]$Senders = $Transactions,

    [ValidateRange(1, 10)]
    [int]$Runs = 1,

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        cargo build --release -p kanari-benchmarks
    } finally {
        Pop-Location
    }
}

# These settings apply only to the child benchmark process.  The benchmark
# creates its RocksDB database in a temporary directory and prints the metrics
# needed to identify write stalls and compaction pressure.
$previousBackend = $env:KANARI_BENCH_STATE_BACKEND
$previousProfile = $env:KANARI_BENCH_ROCKSDB_PROFILE
$previousFlush = $env:KANARI_BENCH_ROCKSDB_FLUSH_PROFILE
$previousCompact = $env:KANARI_BENCH_ROCKSDB_COMPACT_PROFILE
try {
    $env:KANARI_BENCH_STATE_BACKEND = 'rocksdb'
    $env:KANARI_BENCH_ROCKSDB_PROFILE = '1'
    $env:KANARI_BENCH_ROCKSDB_FLUSH_PROFILE = '1'
    $env:KANARI_BENCH_ROCKSDB_COMPACT_PROFILE = '1'

    Write-Host "Persistent RocksDB benchmark profile: txs=$Transactions senders=$Senders runs=$Runs"
    Push-Location $repoRoot
    try {
        cargo run --release -p kanari-benchmarks -- `
            --mode production `
            --txs $Transactions `
            --senders $Senders `
            --runs $Runs `
            --json
    } finally {
        Pop-Location
    }
} finally {
    if ($null -eq $previousBackend) { Remove-Item Env:\KANARI_BENCH_STATE_BACKEND -ErrorAction SilentlyContinue } else { $env:KANARI_BENCH_STATE_BACKEND = $previousBackend }
    if ($null -eq $previousProfile) { Remove-Item Env:\KANARI_BENCH_ROCKSDB_PROFILE -ErrorAction SilentlyContinue } else { $env:KANARI_BENCH_ROCKSDB_PROFILE = $previousProfile }
    if ($null -eq $previousFlush) { Remove-Item Env:\KANARI_BENCH_ROCKSDB_FLUSH_PROFILE -ErrorAction SilentlyContinue } else { $env:KANARI_BENCH_ROCKSDB_FLUSH_PROFILE = $previousFlush }
    if ($null -eq $previousCompact) { Remove-Item Env:\KANARI_BENCH_ROCKSDB_COMPACT_PROFILE -ErrorAction SilentlyContinue } else { $env:KANARI_BENCH_ROCKSDB_COMPACT_PROFILE = $previousCompact }
}
