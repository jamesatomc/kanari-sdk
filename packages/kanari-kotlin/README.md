# Kanari Kotlin / Jetpack Compose

Android library สำหรับ Kanari cryptographic SDK ที่รองรับ **Jetpack Compose** โดยใช้ Rust core ผ่าน [UniFFI](https://mozilla.github.io/uniffi-rs/).

## โครงสร้าง

```
packages/kanari-kotlin/
├── src/                    # Rust FFI (UniFFI)
├── android/
│   ├── kanari-crypto/      # Android library + Compose UI
│   └── sample/             # ตัวอย่าง Compose app
└── scripts/
    ├── generate-bindings.ps1
    └── build-android.ps1
```

## ความสามารถ

- สร้าง keypair, mnemonic, HD derivation
- Sign / verify, Blake3 hash
- รองรับ post-quantum และ hybrid curves
- **Jetpack Compose UI** พร้อมใช้:
  - `KanariTheme` — Material 3 theme
  - `KeyGenerationScreen` — หน้าสร้าง wallet
  - `WalletAddressCard`, `MnemonicDisplay`, `CurveSelector`

## การติดตั้งในโปรเจกต์ Compose

1. เพิ่ม module ใน `settings.gradle.kts`:

```kotlin
include(":kanari-crypto")
project(":kanari-crypto").projectDir = file("../packages/kanari-kotlin/android/kanari-crypto")
```

2. เพิ่ม dependency:

```kotlin
dependencies {
    implementation(project(":kanari-crypto"))
}
```

3. Build native library ก่อน compile Android (ดูด้านล่าง)

## Build native library

**Prerequisites:** Rust, Android NDK, `ANDROID_NDK_HOME` หรือ `ANDROID_HOME`

```powershell
cd packages/kanari-kotlin
.\scripts\build-android.ps1
```

## Generate Kotlin bindings

หลังแก้ไข Rust API:

```powershell
cd packages/kanari-kotlin
.\scripts\generate-bindings.ps1
```

## ตัวอย่างการใช้งาน

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

## รัน sample app

```powershell
cd packages/kanari-kotlin
.\scripts\build-android.ps1
cd android
gradle :sample:installDebug
```

## ดูเพิ่มเติม

- [CODEGEN_GUIDE.md](./CODEGEN_GUIDE.md) — รายละเอียด UniFFI codegen
