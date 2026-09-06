# UniFFI Codegen Guide (Kanari Kotlin)

Guide for generating Kotlin bindings from the Rust crate `kanari-kotlin`

## Prerequisites

- Rust toolchain
- `cargo` in PATH

## Generate Kotlin bindings

```powershell
cd packages/kanari-kotlin
.\scripts\generate-bindings.ps1
```

Or run manually:

```powershell
cargo run --bin uniffi-bindgen -- generate `
  --language kotlin `
  --no-format `
  -o android/kanari-crypto/src/main/kotlin `
  src/kanari_kotlin.udl
```

Main output:

- `android/kanari-crypto/src/main/kotlin/uniffi/kanari_kotlin/kanari_kotlin.kt`

**Do not edit generated files by hand** — edit `src/lib.rs` and regenerate

## When to regenerate

- Add/remove/change functions in `src/lib.rs` with `#[uniffi::export]`
- Change record types (`KeyPairData`, `CurveInfo`)
- Update `src/kanari_kotlin.udl`

## Build Android `.so` libraries

```powershell
.\scripts\build-android.ps1
```

The script cross-compiles for:

| ABI | Rust target |
| ----- | ------------- |
| arm64-v8a | aarch64-linux-android |
| armeabi-v7a | armv7-linux-androideabi |
| x86_64 | x86_64-linux-android |
| x86 | i686-linux-android |

Output: `android/kanari-crypto/src/main/jniLibs/<abi>/libkanari_kotlin.so`

## Recommended workflow

1. Edit the Rust API in `src/lib.rs`
2. Sync `src/kanari_kotlin.udl` if needed
3. `cargo build` to verify Rust
4. `.\scripts\generate-bindings.ps1`
5. `.\scripts\build-android.ps1`
6. Build the Android project in `android/`
