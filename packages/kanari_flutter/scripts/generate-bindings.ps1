param(
    [string]$ConfigFile = "frb.yml"
)

$ErrorActionPreference = "Stop"
$PackageRoot = Split-Path $PSScriptRoot -Parent
$RustDir = Join-Path $PackageRoot "rust"
$CargoToml = Join-Path $RustDir "Cargo.toml"

Push-Location $RustDir
try {
    Write-Host "Generating Flutter Rust Bridge bindings..."
    flutter_rust_bridge_codegen generate --config-file $ConfigFile
    Write-Host "Generated Dart + Rust bindings (src/frb_generated.dart, src/frb_generated.rs)."
} finally {
    Pop-Location
}

# flutter_rust_bridge_codegen rewrites Cargo.toml to pin the FRB version,
# replacing workspace dependencies. Restore the original workspace ref.
Write-Host "Restoring Cargo.toml workspace dependency..."
$content = Get-Content -Raw $CargoToml
$restored = $content -replace 'flutter_rust_bridge\s*=\s*"[^"]*"', 'flutter_rust_bridge.workspace = true'
if ($restored -ne $content) {
    Set-Content -Path $CargoToml -Value $restored -NoNewline
    Write-Host "Restored flutter_rust_bridge.workspace = true in Cargo.toml"
} else {
    Write-Host "No Cargo.toml changes needed"
}