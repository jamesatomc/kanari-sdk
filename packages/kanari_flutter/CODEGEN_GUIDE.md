# Flutter Rust Bridge Codegen Guide (Kanari Flutter)

Guide for generating Dart/Rust bindings from the Rust crate `rust/` for the Flutter package `flutter/kanari_crypto`

## Structure

```md
packages/kanari_flutter/
├── rust/                   # Rust crate (FRB API in src/api.rs)
│   ├── frb.yml             # FRB config
│   └── src/frb_generated.rs
├── flutter/
│   └── kanari_crypto/      # Flutter package
│       ├── lib/            # Dart API + bindings
│       └── android/src/main/jniLibs/   # .so files for Android
└── scripts/
    ├── generate-bindings.ps1
    ├── build-android.ps1
    └── build-ios.ps1
```

## Prerequisites

- Rust toolchain
- [flutter_rust_bridge_codegen](https://cjycode.com/flutter_rust_bridge_guide/) CLI:

  ```bash
  cargo install flutter_rust_bridge_codegen
  ```

- For Android: Android NDK + `ANDROID_NDK_HOME` or `ANDROID_HOME`

## Generate bindings

```powershell
cd packages/kanari_flutter
.\scripts\generate-bindings.ps1
```

Or run manually:

```bash
cd packages/kanari_flutter/rust
flutter_rust_bridge_codegen generate --config-file frb.yml
```

Main outputs:

- Dart: `flutter/kanari_crypto/lib/src/frb_generated.dart`
- Rust: `rust/src/frb_generated.rs`

**Do not edit generated files by hand** — edit `rust/src/api.rs` and regenerate

## When to regenerate

- Add/remove/change functions in `rust/src/api.rs`
- Change record types / enums in the API
- Update `rust/frb.yml`

## Build Android `.so` libraries

```powershell
cd packages/kanari_flutter
.\scripts\build-android.ps1            # release (default)
.\scripts\build-android.ps1 -Profile debug
```

The script cross-compiles for:

| ABI | Rust target |
| ----- | ------------- |
| arm64-v8a | aarch64-linux-android |
| armeabi-v7a | armv7-linux-androideabi |
| x86_64 | x86_64-linux-android |
| x86 | i686-linux-android |

Output: `flutter/kanari_crypto/android/src/main/jniLibs/<abi>/librust.so`

## Example usage

```dart
import 'package:kanari_crypto/kanari_crypto.dart';

final mnemonic = await generateMnemonicApi(wordCount: BigInt.from(12));
final keypair = await generateKeypairApi(curveName: curves.first.name);
```

## Recommended workflow

1. Edit the Rust API in `rust/src/api.rs`
2. `cargo build` to verify Rust
3. `.\scripts\generate-bindings.ps1`
4. `.\scripts\build-android.ps1`
5. Build/test the Flutter package in `flutter/kanari_crypto`

## Troubleshooting

### Common Errors

- **"Prefix not found"**: run from the directory containing `frb.yml` or specify absolute paths
- **"Please migrate configuration rust_input"**: FRB v2 uses `crate::api` instead of `src/api.rs`
- **Path Canonicalization**: on Windows use forward slashes `/` in `frb.yml`
