import 'package:kanari_crypto/kanari_crypto.dart';

/// Minimal example demonstrating basic usage of `kanari_crypto`.
Future<void> main() async {
  // Generate a mnemonic
  final mnemonic = await generateMnemonicApi(wordCount: BigInt.from(12));
  print('mnemonic: $mnemonic');

  // List supported curves
  final curves = await listSupportedCurves();
  print('Supported curves: ${curves.map((c) => c.name).join(', ')}');

  // Generate a keypair for the first supported curve
  if (curves.isNotEmpty) {
    final kp = await generateKeypairApi(curveName: curves.first.name);
    print('Generated address: ${kp.address}');
  }
}
