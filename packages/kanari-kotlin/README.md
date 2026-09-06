# Kanari Kotlin / Jetpack Compose

Android library for the Kanari cryptographic SDK that supports **Jetpack Compose**, backed by the Rust core via [UniFFI](https://mozilla.github.io/uniffi-rs/).

## Structure

```md
packages/kanari-kotlin/
├── src/                    # Rust FFI (UniFFI)
├── android/
│   ├── kanari-crypto/      # Android library + Compose UI
│   └── sample/             # Sample Compose app
└── scripts/
    ├── generate-bindings.ps1
    └── build-android.ps1
```

## Features

- Keypair, mnemonic and HD derivation generation
- Sign / verify, Blake3 hash
- Support for post-quantum and hybrid curves
- **Jetpack Compose UI** ready to use:
  - `KanariTheme` — Material 3 theme
  - `KeyGenerationScreen` — wallet creation screen
  - `WalletAddressCard`, `MnemonicDisplay`, `CurveSelector`

## Installing in a Compose project

1. Add the module in `settings.gradle.kts`:

```kotlin
include(":kanari-crypto")
project(":kanari-crypto").projectDir = file("../packages/kanari-kotlin/android/kanari-crypto")
```

1. Add the dependency:

```kotlin
dependencies {
    implementation(project(":kanari-crypto"))
}
```

1. Build the native library before compiling for Android (see below)

## Build native library

**Prerequisites:** Rust, Android NDK, `ANDROID_NDK_HOME` or `ANDROID_HOME`

```powershell
cd packages/kanari-kotlin
.\scripts\build-android.ps1
```

## Generate Kotlin bindings

After editing the Rust API:

```powershell
cd packages/kanari-kotlin
.\scripts\generate-bindings.ps1
```

## Usage examples

### Crypto API

```kotlin
import com.kanari.kanari_crypto.KanariCrypto

val mnemonic = KanariCrypto.generateMnemonic(12)
val keypair = KanariCrypto.deriveKeypairFromMnemonic(mnemonic, "Ed25519")
val signature = KanariCrypto.signMessage(keypair.privateKey, message)
```

### Compose UI

```kotlin
import com.kanari.kanari_crypto.compose.KanariTheme
import com.kanari.kanari_crypto.compose.KeyGenerationScreen

setContent {
    KanariTheme {
        KeyGenerationScreen(
            onKeyPairGenerated = { keyPair ->
                // handle new wallet
            },
        )
    }
}
```

## Run the sample app

```powershell
cd packages/kanari-kotlin
.\scripts\build-android.ps1
cd android
gradle :sample:installDebug
```

## See also

- [CODEGEN_GUIDE.md](./CODEGEN_GUIDE.md) — UniFFI codegen details
