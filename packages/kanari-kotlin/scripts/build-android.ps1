param(
    [ValidateSet("debug", "release")]
    [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$PackageRoot = Split-Path $PSScriptRoot -Parent
$JniLibsDir = Join-Path $PackageRoot "android\kanari-crypto\src\main\jniLibs"

$Targets = @(
    @{ Triple = "aarch64-linux-android"; Abi = "arm64-v8a" },
    @{ Triple = "armv7-linux-androideabi"; Abi = "armeabi-v7a" },
    @{ Triple = "x86_64-linux-android"; Abi = "x86_64" },
    @{ Triple = "i686-linux-android"; Abi = "x86" }
)

function Get-NdkHome {
    if ($env:ANDROID_NDK_HOME) { return $env:ANDROID_NDK_HOME }
    if ($env:NDK_HOME) { return $env:NDK_HOME }
    $SdkRoot = $env:ANDROID_HOME
    if (-not $SdkRoot) { $SdkRoot = $env:ANDROID_SDK_ROOT }
    if (-not $SdkRoot) {
        throw "Set ANDROID_NDK_HOME or ANDROID_HOME before building native libraries."
    }
    $ndkDir = Get-ChildItem (Join-Path $SdkRoot "ndk") -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        Select-Object -First 1
    if (-not $ndkDir) {
        throw "No NDK found under $SdkRoot\ndk"
    }
    return $ndkDir.FullName
}

$NdkHome = Get-NdkHome
$HostTag = if ($IsWindows -or $env:OS -like "*Windows*") { "windows-x86_64" } else { "linux-x86_64" }
$ToolchainBin = Join-Path $NdkHome "toolchains\llvm\prebuilt\$HostTag\bin"

$env:CC_aarch64_linux_android = Join-Path $ToolchainBin "aarch64-linux-android21-clang.cmd"
$env:CC_armv7_linux_androideabi = Join-Path $ToolchainBin "armv7a-linux-androideabi21-clang.cmd"
$env:CC_x86_64_linux_android = Join-Path $ToolchainBin "x86_64-linux-android21-clang.cmd"
$env:CC_i686_linux_android = Join-Path $ToolchainBin "i686-linux-android21-clang.cmd"
$env:AR_aarch64_linux_android = Join-Path $ToolchainBin "llvm-ar.exe"
$env:AR_armv7_linux_androideabi = Join-Path $ToolchainBin "llvm-ar.exe"
$env:AR_x86_64_linux_android = Join-Path $ToolchainBin "llvm-ar.exe"
$env:AR_i686_linux_android = Join-Path $ToolchainBin "llvm-ar.exe"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $env:CC_aarch64_linux_android
$env:CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER = $env:CC_armv7_linux_androideabi
$env:CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER = $env:CC_x86_64_linux_android
$env:CARGO_TARGET_I686_LINUX_ANDROID_LINKER = $env:CC_i686_linux_android

Push-Location $PackageRoot
try {
    foreach ($target in $Targets) {
        Write-Host "Building $($target.Triple) ($Profile)..."
        cargo build --release --target $target.Triple

        $libName = "libkanari_kotlin.so"
        $destLibName = "libuniffi_kanari_kotlin.so"
        $sourceLib = Join-Path $PackageRoot "target\$($target.Triple)\release\$libName"
        if (-not (Test-Path $sourceLib)) {
            throw "Expected library not found: $sourceLib"
        }

        $destDir = Join-Path $JniLibsDir $target.Abi
        New-Item -ItemType Directory -Force -Path $destDir | Out-Null
        Copy-Item $sourceLib (Join-Path $destDir $destLibName) -Force
        Write-Host "Copied to $destDir\$destLibName"
    }
} finally {
    Pop-Location
}

Write-Host "Android native libraries ready in $JniLibsDir"
