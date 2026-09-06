param(
    [ValidateSet("debug", "release")]
    [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$PackageRoot = Split-Path $PSScriptRoot -Parent
$RustDir = Join-Path $PackageRoot "rust"
$LibDir = Join-Path $PackageRoot "flutter\kanari_crypto\lib"

Push-Location $RustDir
try {
    Write-Host "Building host target ($Profile)..."
    if ($Profile -eq "release") {
        cargo build --release
    } else {
        cargo build
    }

    $libName = "rust.dll"
    $subDir = if ($Profile -eq "release") { "release" } else { "debug" }
    $sourceLib = Join-Path $RustDir "target\$subDir\$libName"
    if (-not (Test-Path $sourceLib)) {
        throw "Expected library not found: $sourceLib"
    }

    New-Item -ItemType Directory -Force -Path $LibDir | Out-Null
    Copy-Item $sourceLib (Join-Path $LibDir $libName) -Force
    Write-Host "Copied to $LibDir\$libName"
} finally {
    Pop-Location
}

Write-Host "Windows native library ready in $LibDir"