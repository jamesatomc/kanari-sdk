param(
    [string]$OutDir = "$PSScriptRoot\..\android\kanari-crypto\src\main\kotlin"
)

$ErrorActionPreference = "Stop"
$PackageRoot = Split-Path $PSScriptRoot -Parent

Push-Location $PackageRoot
try {
    Write-Host "Generating Kotlin bindings..."
    cargo run --bin uniffi-bindgen -- generate `
        --language kotlin `
        --no-format `
        -o $OutDir `
        src/kanari_kotlin.udl
    Write-Host "Kotlin bindings written to $OutDir"
} finally {
    Pop-Location
}
