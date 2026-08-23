# Implementation Plan - Create Wallet Helper

The goal is to provide a high-level `Wallet` abstraction in the `kanari-kotlin` SDK to simplify wallet creation and management (mnemonic + derived keys).

## Proposed Changes

### [kanari-kotlin]

#### [NEW] [Wallet.kt](file:///D:/move-jamae/kanari-sdk/packages/kanari-kotlin/android/kanari-crypto/src/main/kotlin/com/kanari/kanari_crypto/model/Wallet.kt)
Create a new data class `Wallet` to represent a user's wallet, containing the mnemonic phrase and a list of derived accounts (`KeyPairModel`).

- Add `create()` static method to generate a new mnemonic and the initial account.
- Add `fromMnemonic()` static method to recover a wallet from an existing mnemonic.
- Add `deriveAccount()` method to derive additional accounts from the mnemonic.

#### [MODIFY] [KanariCrypto.kt](file:///D:/move-jamae/kanari-sdk/packages/kanari-kotlin/android/kanari-crypto/src/main/kotlin/com/kanari/kanari_crypto/KanariCrypto.kt)
Ensure all necessary functions are exposed and consistent. (Optional, if any gaps are found).

## Verification Plan

### Manual Verification
- Update or create a small unit test to verify that `Wallet.create()` returns a valid mnemonic and a matching address.
- Verify that `Wallet.fromMnemonic()` with the same mnemonic and path produces the same address.
