# UniFFI Codegen Guide (Kanari Kotlin)

คู่มือการ generate Kotlin bindings จาก Rust crate `kanari-kotlin`

## Prerequisites

- Rust toolchain
- `cargo` ใน PATH

## Generate Kotlin bindings

```powershell
cd packages/kanari-kotlin
.\scripts\generate-bindings.ps1
```

หรือรันด้วยมือ:

```powershell
cargo run --bin uniffi-bindgen -- generate `
  --language kotlin `
  --no-format `
  -o android/kanari-crypto/src/main/kotlin `
  src/kanari_kotlin.udl
```

Output หลัก:

- `android/kanari-crypto/src/main/kotlin/uniffi/kanari_kotlin/kanari_kotlin.kt`

**อย่าแก้ไขไฟล์ generated ด้วยมือ** — แก้ที่ `src/lib.rs` แล้ว generate ใหม่

## เมื่อไหร่ต้อง regenerate

- เพิ่ม/ลบ/เปลี่ยน function ใน `src/lib.rs` ที่มี `#[uniffi::export]`
- เปลี่ยน record types (`KeyPairData`, `CurveInfo`)
- อัปเดต `src/kanari_kotlin.udl`

## Build Android `.so` libraries

```powershell
.\scripts\build-android.ps1
```

Script จะ cross-compile สำหรับ:

| ABI | Rust target |
|-----|-------------|
| arm64-v8a | aarch64-linux-android |
| armeabi-v7a | armv7-linux-androideabi |
| x86_64 | x86_64-linux-android |
| x86 | i686-linux-android |

Output: `android/kanari-crypto/src/main/jniLibs/<abi>/libkanari_kotlin.so`

## Workflow แนะนำ

1. แก้ Rust API ใน `src/lib.rs`
2. Sync `src/kanari_kotlin.udl` ถ้าจำเป็น
3. `cargo build` เพื่อ verify Rust
4. `.\scripts\generate-bindings.ps1`
5. `.\scripts\build-android.ps1`
6. Build Android project ใน `android/`
