# kanari_crypto Flutter package

kanari_crypto is a small Flutter/Dart wrapper providing convenient access
to cryptographic utilities used by the Kanari SDK. It exposes common
operations such as key derivation, signing, verification, and keystore
management that integrate with the native Rust-based Kanari cryptography
library through the package's platform bindings.

This README gives a concise overview for Flutter app and plugin authors on
how to add, configure, and use `kanari_crypto` in your application.

## Features

- High-level APIs for key generation and deterministic wallets (HD wallets).
- Message and transaction signing / verification helpers.
- Keystore encryption and secure backup helpers.
- Utilities for key format conversion and compression.

## Getting started

### Prerequisites

- Flutter 3.0+ and a Dart SDK compatible with your Flutter release.
- Platform-specific native bindings (the package ships platform code in the
 `flutter` folder). Ensure your project supports the target platforms
 (Android, iOS, Linux, macOS, Windows) you intend to build for.

### Installation

1.Open your Flutter project's `pubspec.yaml` and add:

```yaml
dependencies:
 kanari_crypto:
  path: ../packages/kanari_flutter/flutter/kanari_crypto
```

2.Run `flutter pub get`.

If `kanari_crypto` is published to a registry (pub.dev or private), replace
the local path with the published version constraint.

## Usage

The following examples show common usage patterns. Import the package in
your Dart code:

```dart
import 'package:kanari_crypto/kanari_crypto.dart';
```

### Generate a new keypair

```dart
final keypair = await KanariCrypto.generateKeypair();
// keypair.publicKey, keypair.privateKey
```

### Derive keys from a mnemonic (HD wallet)

```dart
final mnemonic = 'abandon abandon abandon ...';
final wallet = await KanariCrypto.fromMnemonic(mnemonic);
final accountKey = wallet.deriveAccount(0);
```

### Sign and verify

```dart
final message = utf8.encode('hello world');
final signature = await KanariCrypto.sign(message, keypair.privateKey);
final ok = await KanariCrypto.verify(message, signature, keypair.publicKey);
```

### Keystore encryption and backup

```dart
final encrypted = await Keystore.encrypt(keypair.privateKey, password: 's3cr3t');
await Keystore.saveToFile(encrypted, File(path));
// restore
final restored = await Keystore.loadFromFile(File(path));
```

Note: the above APIs are illustrative. See the package API docs and source
for exact method names and parameter types.

## Examples

Longer, runnable examples are provided in the `example/` directory. Examples
demonstrate integration points with Flutter widgets, secure storage, and
platform-specific key management.

## Testing

Run package unit tests (if included) with:

```bash
flutter test
```

## Contributing

Contributions are welcome. Please follow the repository contribution
guidelines in the main project README. Typical contributions include:

- Bug fixes and test coverage
- New examples and documentation
- Platform binding improvements

When opening pull requests, include tests or example code that demonstrates
the change.

## Security

This package deals with sensitive cryptographic material. Follow these
guidelines:

- Never log private keys or mnemonics.
- Use secure platform storage (Keychain / Keystore) where available.
- Be careful when serializing keys to files; always encrypt backups with a
 strong passphrase.

Refer to `kanari-crypto` crate documentation for low-level security details
and recommended practices.

## License

Check the root repository LICENSE file for licensing information.

## Where to get help

For questions, bug reports, or feature requests, please open an issue in
the main repository or contact the maintainers listed in the project
documentation.
