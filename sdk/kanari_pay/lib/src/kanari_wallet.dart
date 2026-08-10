import 'package:kanari_crypto/kanari_crypto.dart';
import 'dart:typed_data';

import 'package:kanari_pay/src/kanaricurve.dart';

class KanariWallet {
  static const String defaultDerivationPath = "m/44'/637'/0'/0/0";

  final KeyPairData _keyPair;
  final String? mnemonic;
  final String? derivationPath;

  KanariWallet(this._keyPair, {this.mnemonic, this.derivationPath});

  String get address => _keyPair.address;
  String get taggedAddress => _keyPair.taggedAddress;
  String get publicKey => _keyPair.publicKey;
  String get privateKey => _keyPair.privateKey;
  String get curveType => _keyPair.curveType;

  static String normalizeDerivationPath(String? derivationPath) {
    final trimmed = derivationPath?.trim();
    return trimmed == null || trimmed.isEmpty ? defaultDerivationPath : trimmed;
  }

  static bool isValidDerivationPath(String? derivationPath) {
    final path = normalizeDerivationPath(derivationPath);
    return RegExp(r"^m(\/[0-9]+'?)+$").hasMatch(path);
  }

  /// Generate a new wallet with a specific curve and random mnemonic
  static Future<KanariWallet> generate({
    required KanariCurve curve,
    String derivationPath = defaultDerivationPath,
  }) async {
    final normalizedPath = normalizeDerivationPath(derivationPath);
    if (curve.isPostQuantum) {
      // PQC curves use direct random generation as they don't support BIP39 yet
      final keyPair = await generateKeypairApi(curveName: curve.name);
      return KanariWallet(keyPair);
    } else {
      final mnemonic = await generateMnemonicApi(wordCount: BigInt.from(12));
      final keyPair = await deriveKeypairFromPathApi(
        mnemonic: mnemonic,
        derivationPath: normalizedPath,
        curveName: curve.name,
      );
      return KanariWallet(
        keyPair,
        mnemonic: mnemonic,
        derivationPath: normalizedPath,
      );
    }
  }

  /// Create a wallet from a mnemonic
  static Future<KanariWallet> fromMnemonic(
    String mnemonic, {
    required KanariCurve curve,
    String derivationPath = defaultDerivationPath,
  }) async {
    final normalizedPath = normalizeDerivationPath(derivationPath);
    final keyPair = await deriveKeypairFromPathApi(
      mnemonic: mnemonic,
      derivationPath: normalizedPath,
      curveName: curve.name,
    );
    return KanariWallet(
      keyPair,
      mnemonic: mnemonic,
      derivationPath: normalizedPath,
    );
  }

  /// Create a wallet from a private key
  static Future<KanariWallet> fromPrivateKey(
    String privateKey, {
    required KanariCurve curve,
  }) async {
    final keyPair = await importKeypairFromPrivateKey(
      privateKey: privateKey,
      curveName: curve.name,
    );
    return KanariWallet(keyPair);
  }

  /// Sign a message using the wallet's private key
  Future<Uint8List> sign(List<int> message) async {
    return signMessageApi(
      privateKey: _keyPair.privateKey,
      message: message,
      curveName: _keyPair.curveType,
    );
  }
}
