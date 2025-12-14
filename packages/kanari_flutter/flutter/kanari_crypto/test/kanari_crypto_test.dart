import 'package:flutter_test/flutter_test.dart';
import 'package:kanari_crypto/kanari_crypto.dart';
import 'dart:typed_data';

class MockRustApi extends RustLibApi {
  @override
  Future<KeyPairData> crateApiGenerateKeypairApi({
    required String curveName,
  }) async {
    return KeyPairData(
      privateKey: 'priv_$curveName',
      publicKey: 'pub_$curveName',
      address: 'addr_$curveName',
      rawPublicKey: Uint8List.fromList([]),
      curveType: curveName,
    );
  }

  @override
  Future<String> crateApiGenerateMnemonicApi({
    required BigInt wordCount,
  }) async {
    return 'alpha beta gamma';
  }

  @override
  Future<List<CurveInfo>> crateApiListSupportedCurves() async {
    return const [
      CurveInfo(
        name: 'K256',
        isPostQuantum: false,
        isHybrid: false,
        securityLevel: 128,
      ),
    ];
  }

  @override
  Future<Uint8List> crateApiSignMessageApi({
    required String privateKey,
    required List<int> message,
    required String curveName,
  }) async {
    return Uint8List.fromList([1, 2, 3]);
  }

  @override
  Future<bool> crateApiVerifySignatureApi({
    required String address,
    required List<int> message,
    required List<int> signature,
    required String curveName,
  }) async {
    return true;
  }

  @override
  Future<KeyPairData> crateApiDeriveKeypairFromMnemonic({
    required String mnemonic,
    required String curveName,
    required String password,
  }) async {
    return crateApiGenerateKeypairApi(curveName: curveName);
  }

  @override
  Future<KeyPairData> crateApiImportKeypairFromPrivateKey({
    required String privateKey,
    required String curveName,
  }) async {
    return crateApiGenerateKeypairApi(curveName: curveName);
  }
}

void main() {
  setUpAll(() {
    RustLib.initMock(api: MockRustApi());
  });

  tearDownAll(() {
    RustLib.dispose();
  });

  test('generate mnemonic', () async {
    final mnemonic = await generateMnemonicApi(wordCount: BigInt.from(12));
    expect(mnemonic, isNotEmpty);
  });

  test('generate keypair and sign/verify', () async {
    final kp = await generateKeypairApi(curveName: 'K256');
    expect(kp.address, contains('addr_'));

    final sig = await signMessageApi(
      privateKey: kp.privateKey,
      message: 'hello'.codeUnits,
      curveName: 'K256',
    );
    expect(sig, isNotEmpty);

    final verified = await verifySignatureApi(
      address: kp.address,
      message: 'hello'.codeUnits,
      signature: sig,
      curveName: 'K256',
    );
    expect(verified, isTrue);
  });

  test('list supported curves', () async {
    final curves = await listSupportedCurves();
    expect(curves, isNotEmpty);
    expect(curves.first.name, 'K256');
  });

  test('Dilithium5 keypair, sign and verify', () async {
    final kp = await generateKeypairApi(curveName: 'Dilithium5');
    expect(kp.curveType, contains('Dilithium5'));

    final sig = await signMessageApi(
      privateKey: kp.privateKey,
      message: 'dilithium'.codeUnits,
      curveName: 'Dilithium5',
    );
    expect(sig, isNotEmpty);

    final verified = await verifySignatureApi(
      address: kp.address,
      message: 'dilithium'.codeUnits,
      signature: sig,
      curveName: 'Dilithium5',
    );
    expect(verified, isTrue);
  });
}
