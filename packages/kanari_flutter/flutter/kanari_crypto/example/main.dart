import 'package:flutter/foundation.dart';
import 'package:kanari_crypto/kanari_crypto.dart';

/// Minimal example demonstrating basic usage of `kanari_crypto`.
Future<void> main() async {
  // Generate a mnemonic
  final mnemonic = await generateMnemonicApi(wordCount: BigInt.from(12));
  debugPrint('mnemonic: $mnemonic');

  // List supported curves
  final curves = await listSupportedCurves();
  debugPrint('Supported curves: ${curves.map((c) => c.name).join(', ')}');

  // Generate a keypair for the first supported curve
  if (curves.isNotEmpty) {
    final kp = await generateKeypairApi(curveName: curves.first.name);
    debugPrint('Generated address: ${kp.address}');
  }
}
