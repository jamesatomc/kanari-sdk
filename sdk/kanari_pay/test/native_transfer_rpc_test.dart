import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:kanari_crypto/kanari_crypto.dart';
import 'package:kanari_pay/kanari_pay.dart';

void main() {
  test('native KANARI transfer uses submitTransaction instead of Coin call', () async {
    final wallet = KanariWallet(
      KeyPairData(
        privateKey: 'priv',
        publicKey: 'pub',
        address: '0x123',
        taggedAddress: 'Ed25519:0x123',
        rawPublicKey: Uint8List(32),
        curveType: 'Ed25519',
      ),
    );

    String? submittedMethod;
    Map<String, dynamic>? submittedParams;
    var accountReads = 0;

    final mockClient = MockClient((request) async {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      final method = body['method'] as String;

      if (method == 'kanari_getAccount') {
        accountReads += 1;
        return http.Response(
          jsonEncode({
            'jsonrpc': '2.0',
            'result': {
              'address': '0x123',
              'balance': 5000000,
              'sequence_number': 7,
              'modules': [],
              'token_balances': {},
              'owned_objects': [],
            },
            'id': 1,
          }),
          200,
        );
      }

      submittedMethod = method;
      submittedParams = Map<String, dynamic>.from(body['params'] as Map);
      return http.Response(
        jsonEncode({
          'jsonrpc': '2.0',
          'result': {
            'hash': '0xnative',
            'status': 'success',
            'gas_used': 100,
          },
          'id': 2,
        }),
        200,
      );
    });

    final client = KanariClient('http://localhost/rpc', client: mockClient);
    final result = await client.transfer(
      wallet: wallet,
      recipient: '0x456',
      amount: 1000,
    );

    expect(result.hash, '0xnative');
    expect(accountReads, 1);
    expect(submittedMethod, 'kanari_submitTransaction');
    expect(submittedMethod, isNot('kanari_callFunction'));
    expect(submittedParams, isNotNull);
    expect(submittedParams!['sender'], 'Ed25519:0x123');
    expect(
      submittedParams!['recipient'],
      '0x${'456'.padLeft(64, '0')}',
    );
    expect(submittedParams!['amount'], 1000);
    expect(submittedParams!['sequence_number'], 7);
    expect(submittedParams!['execute_immediate'], true);
    expect(submittedParams!['signature'], isA<List>());
  });
}
