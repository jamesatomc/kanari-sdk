// Interactive example for kanari_crypto
import 'dart:convert';
import 'dart:io';
import 'package:kanari_crypto/kanari_crypto.dart';

String bytesToHex(List<int> bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

String bytesToBase64(List<int> bytes) => base64.encode(bytes);

Future<void> main(List<String> args) async {
  await RustLib.init();

  try {
    final curves = await listSupportedCurves();
    if (curves.isEmpty) {
      print('No curves available from native library.');
      return;
    }

    print('Available curves:');
    for (var i = 0; i < curves.length; i++) {
      final c = curves[i];
      print(
        '  [$i] ${c.name} (postQuantum=${c.isPostQuantum}, hybrid=${c.isHybrid})',
      );
    }

    String chosen;
    if (args.isNotEmpty) {
      chosen = args.first;
      print('Using curve from CLI arg: $chosen');
    } else {
      stdout.write('Choose curve index (enter for 0): ');
      final line = stdin.readLineSync();
      final idx = int.tryParse(line ?? '') ?? 0;
      chosen = curves[idx.clamp(0, curves.length - 1)].name;
    }

    print('\nGenerating mnemonic (12 words)...');
    final mnemonic = await generateMnemonicApi(wordCount: BigInt.from(12));
    print('Mnemonic: $mnemonic\n');

    print('Generating keypair for $chosen...');
    final kp = await generateKeypairApi(curveName: chosen);
    print('Address: ${kp.address}');
    print('Public key: ${kp.publicKey}');
    print('Raw public key (bytes): ${kp.rawPublicKey.length} bytes');
    print('Private key (KEEP SECRET): ${kp.privateKey}\n');

    final messageText = 'hello from dart example';
    final message = messageText.codeUnits;

    print('Signing message: "$messageText"');
    final sig = await signMessageApi(
      privateKey: kp.privateKey,
      message: message,
      curveName: chosen,
    );

    print('Signature (hex): ${bytesToHex(sig)}');
    print('Signature (base64): ${bytesToBase64(sig)}');

    final addressForVerify = kp.rawPublicKey.isNotEmpty
        ? bytesToHex(kp.rawPublicKey)
        : kp.address;
    final ok = await verifySignatureApi(
      address: addressForVerify,
      message: message,
      signature: sig,
      curveName: chosen,
    );
    print('Verification result: $ok');

    // Demonstrate Dilithium5 specifically (if available)
    final hasDilithium5 = curves.any((c) => c.name == 'Dilithium5');
    if (hasDilithium5) {
      print(
        '\nDilithium5 demo: generating keypair and signing a short message...',
      );
      final dk = await generateKeypairApi(curveName: 'Dilithium5');
      final dmsg = 'pq-demo'.codeUnits;
      final dsig = await signMessageApi(
        privateKey: dk.privateKey,
        message: dmsg,
        curveName: 'Dilithium5',
      );
      final dkAddressForVerify = dk.rawPublicKey.isNotEmpty
          ? bytesToHex(dk.rawPublicKey)
          : dk.address;
      final dver = await verifySignatureApi(
        address: dkAddressForVerify,
        message: dmsg,
        signature: dsig,
        curveName: 'Dilithium5',
      );
      print('Dilithium5 signature base64: ${bytesToBase64(dsig)}');
      print('Dilithium5 verification: $dver');
    } else {
      print('\nDilithium5 not available on this build.');
    }
  } catch (e, st) {
    stderr.writeln('Example failed: $e');
    stderr.writeln(st);
  } finally {
    RustLib.dispose();
  }
}
