import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:kanari_crypto/kanari_crypto.dart';
import 'package:kanari_pay/kanari_pay.dart';
import 'package:kanari_pay/src/core/bcs_utils.dart';

void main() {
  group('KanariClient', () {
    test('coin type extraction only accepts actual Coin objects', () {
      expect(
        BcsUtils.extractCoinTypeFromObjectType(
          '0x2::coin::Coin<0x2::kanari::KANARI>',
        ),
        '0x2::kanari::KANARI',
      );

      expect(
        BcsUtils.extractCoinTypeFromObjectType(
          '0xescrow::escrow::EscrowDeal<0x2::kanari::KANARI>',
        ),
        isNull,
      );
    });

    test('coin type extraction canonicalizes whitespace', () {
      expect(
        BcsUtils.extractCoinTypeFromObjectType(
          '0x2::coin::Coin< 0x2::kanari::KANARI >',
        ),
        '0x2::kanari::KANARI',
      );

      expect(
        BcsUtils.extractCoinTypeFromObjectType(
          '0x2::coin::Coin< 0x2::coin::Coin< 0x2::kanari::KANARI > >',
        ),
        '0x2::kanari::KANARI',
      );
    });

    test('getHealth returns health status', () async {
      final mockClient = MockClient((request) async {
        return http.Response(
          jsonEncode({
            'jsonrpc': '2.0',
            'result': {
              'status': 'ok',
              'version': '0.1.0',
              'uptime_seconds': 100,
              'sync_status': 'synced',
            },
            'id': 1,
          }),
          200,
        );
      });

      final client = KanariClient(
        'http://localhost:19001/rpc',
        client: mockClient,
      );
      final health = await client.getHealth();

      expect(health.status, 'ok');
      expect(health.version, '0.1.0');
      expect(health.uptimeSeconds, 100);
      expect(health.syncStatus, 'synced');
    });

    test('fromEnvironment uses correct URL', () {
      final clientDev = KanariClient.fromEnvironment(KanariEnvironment.dev);
      expect(clientDev.url, 'http://192.168.1.101:19001/rpc');

      final clientLocal = KanariClient.fromEnvironment(KanariEnvironment.local);
      expect(clientLocal.url, 'http://127.0.0.1:6767/rpc');
    });

    test('transfer uses buildNativeTransfer before submit', () async {
      final mockWallet = _wallet();
      final methods = <String>[];
      Map<String, dynamic>? builtParams;
      Map<String, dynamic>? submittedParams;

      final mockClient = MockClient((request) async {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        final method = body['method'] as String;
        methods.add(method);

        if (method == 'kanari_buildNativeTransfer') {
          builtParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'sender': 'Ed25519:0x123',
                'coin_object_id':
                    '0x0000000000000000000000000000000000000000000000000000000000000abc',
                'coin_object_ref': {
                  'object_id':
                      '0x0000000000000000000000000000000000000000000000000000000000000abc',
                  'version': 7,
                  'digest': '0xdigest',
                },
                'recipient':
                    '0x0000000000000000000000000000000000000000000000000000000000000456',
                'amount': 1000,
                'gas_limit': 1,
                'gas_price': 1,
                'nonce': 5,
                'gas_payment': {
                  'payment_objects': [
                    {
                      'object_id':
                          '0x0000000000000000000000000000000000000000000000000000000000000def',
                      'version': 11,
                      'digest': '0xgasdigest',
                    },
                  ],
                  'owner': 'Ed25519:0x123',
                  'budget': 1,
                  'price': 1,
                },
                'execute_immediate': true,
              },
              'id': 1,
            }),
            200,
          );
        }

        if (method == 'kanari_submitObjectTransfer') {
          submittedParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'hash': '0xtxhash',
                'status': 'success',
                'gas_used': 100,
              },
              'id': 2,
            }),
            200,
          );
        }

        return http.Response('', 404);
      });

      final client = KanariClient('http://localhost/rpc', client: mockClient);
      final result = await client.transfer(
        wallet: mockWallet,
        recipient: '0x456',
        amount: 1000,
        gasLimit: 1,
        gasPrice: 1,
        excludedObjectIds: ['0xdef'],
      );

      expect(result.hash, '0xtxhash');
      expect(methods, [
        'kanari_buildNativeTransfer',
        'kanari_submitObjectTransfer',
      ]);
      expect(builtParams?['sender'], 'Ed25519:0x123');
      expect(
        builtParams?['recipient'],
        '0x0000000000000000000000000000000000000000000000000000000000000456',
      );
      expect(builtParams?['excluded_object_ids'], [
        '0x0000000000000000000000000000000000000000000000000000000000000def',
      ]);
      expect(submittedParams?['coin_object_ref'], isA<Map>());
      expect(
        submittedParams?['gas_payment']?['payment_objects']?[0]?['object_id'],
        '0x0000000000000000000000000000000000000000000000000000000000000def',
      );
      expect(submittedParams?['nonce'], 5);
      expect(submittedParams?['signature'], isA<List>());
    });

    test('getOwnedObjects returns object-centric owner state', () async {
      final mockClient = MockClient((request) async {
        final body = jsonDecode(request.body);
        final method = body['method'];

        if (method == 'kanari_getOwnedObjects') {
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'objects': [
                  {
                    'id':
                        '0x0000000000000000000000000000000000000000000000000000000000000abc',
                    'owner': '0x123',
                    'type_': '0x2::coin::Coin<0x2::kanari::KANARI>',
                    'data': [...List<int>.filled(40, 0)],
                    'version': 7,
                  },
                ],
              },
              'id': 1,
            }),
            200,
          );
        }

        return http.Response('', 404);
      });

      final client = KanariClient('http://localhost/rpc', client: mockClient);
      final objects = await client.getOwnedObjects(
        '0x123',
        objectType: '0x2::coin::Coin<0x2::kanari::KANARI>',
      );

      expect(objects, hasLength(1));
      expect(
        objects.first.id,
        '0x0000000000000000000000000000000000000000000000000000000000000abc',
      );
      expect(objects.first.version, 7);
    });

    test('getObject returns one on-chain object', () async {
      final mockClient = MockClient((request) async {
        final body = jsonDecode(request.body);
        final method = body['method'];

        if (method == 'kanari_getObject') {
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'id':
                    '0x0000000000000000000000000000000000000000000000000000000000000abc',
                'owner': '0x123',
                'type_': '0x2::coin::Coin<0x2::kanari::KANARI>',
                'data': [...List<int>.filled(40, 0)],
                'version': 9,
              },
              'id': 1,
            }),
            200,
          );
        }

        return http.Response('', 404);
      });

      final client = KanariClient('http://localhost/rpc', client: mockClient);
      final object = await client.getObject(
        '0x0000000000000000000000000000000000000000000000000000000000000abc',
      );

      expect(object.owner, '0x123');
      expect(object.version, 9);
    });

    test('publishModule uses buildPublishModule before submit', () async {
      final mockWallet = _wallet();
      final methods = <String>[];
      Map<String, dynamic>? builtParams;
      Map<String, dynamic>? submittedParams;

      final mockClient = MockClient((request) async {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        final method = body['method'] as String;
        methods.add(method);

        if (method == 'kanari_buildPublishModule') {
          builtParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'sender': 'Ed25519:0x123',
                'module_bytes': [1, 2, 3],
                'module_name': 'TestModule',
                'gas_limit': 100000,
                'gas_price': 1000,
                'nonce': 10,
                'gas_payment': {
                  'payment_objects': const [],
                  'owner': 'Ed25519:0x123',
                  'budget': 100000,
                  'price': 1000,
                },
              },
              'id': 1,
            }),
            200,
          );
        }

        if (method == 'kanari_publishModule') {
          submittedParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'hash': '0xpubhash',
                'status': 'success',
                'gas_used': 500,
              },
              'id': 2,
            }),
            200,
          );
        }

        return http.Response('', 404);
      });

      final client = KanariClient('http://localhost/rpc', client: mockClient);
      final result = await client.publishModule(
        wallet: mockWallet,
        moduleBytes: [1, 2, 3],
        moduleName: 'TestModule',
      );

      expect(result.hash, '0xpubhash');
      expect(methods, ['kanari_buildPublishModule', 'kanari_publishModule']);
      expect(builtParams?['sender'], 'Ed25519:0x123');
      expect(submittedParams?['nonce'], 10);
      expect(submittedParams?['signature'], isA<List>());
    });

    test('executeFunction uses buildCallFunction before submit', () async {
      final mockWallet = _wallet();
      final methods = <String>[];
      Map<String, dynamic>? builtParams;
      Map<String, dynamic>? submittedParams;

      final mockClient = MockClient((request) async {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        final method = body['method'] as String;
        methods.add(method);

        if (method == 'kanari_buildCallFunction') {
          builtParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'sender': 'Ed25519:0x123',
                'package': '0x1',
                'module': 'test',
                'function': 'run',
                'type_args': const [],
                'args': const [
                  [1],
                  [2],
                ],
                'object_inputs': const [],
                'gas_limit': 100000,
                'gas_price': 1000,
                'nonce': 15,
                'gas_payment': {
                  'payment_objects': const [],
                  'owner': 'Ed25519:0x123',
                  'budget': 100000,
                  'price': 1000,
                },
              },
              'id': 1,
            }),
            200,
          );
        }

        if (method == 'kanari_callFunction') {
          submittedParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'hash': '0xcallhash',
                'status': 'success',
                'gas_used': 200,
              },
              'id': 2,
            }),
            200,
          );
        }

        return http.Response('', 404);
      });

      final client = KanariClient('http://localhost/rpc', client: mockClient);
      final result = await client.executeFunction(
        wallet: mockWallet,
        package: '0x1',
        module: 'test',
        function: 'run',
        args: const [
          [1],
          [2],
        ],
      );

      expect(result.hash, '0xcallhash');
      expect(methods, ['kanari_buildCallFunction', 'kanari_callFunction']);
      expect(builtParams?['package'], '0x1');
      expect(submittedParams?['args'], const [
        [1],
        [2],
      ]);
      expect(submittedParams?['nonce'], 15);
      expect(submittedParams?['signature'], isA<List>());
    });

    test('burn uses buildCallFunction before submit', () async {
      final mockWallet = _wallet();
      final methods = <String>[];
      Map<String, dynamic>? builtParams;
      Map<String, dynamic>? submittedParams;

      final mockClient = MockClient((request) async {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        final method = body['method'] as String;
        methods.add(method);

        if (method == 'kanari_buildCallFunction') {
          builtParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'sender': 'Ed25519:0x123',
                'package': '0x2',
                'module': 'kanari',
                'function': 'burn_amount',
                'type_args': const [],
                'args': const [
                  [244, 1, 0, 0, 0, 0, 0, 0],
                ],
                'object_inputs': const [],
                'gas_limit': 100000,
                'gas_price': 1000,
                'nonce': 20,
                'gas_payment': {
                  'payment_objects': const [],
                  'owner': 'Ed25519:0x123',
                  'budget': 100000,
                  'price': 1000,
                },
                'execute_immediate': true,
              },
              'id': 1,
            }),
            200,
          );
        }

        if (method == 'kanari_callFunction') {
          submittedParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'hash': '0xburnhash',
                'status': 'success',
                'gas_used': 50,
              },
              'id': 2,
            }),
            200,
          );
        }

        return http.Response('', 404);
      });

      final client = KanariClient('http://localhost/rpc', client: mockClient);
      final result = await client.burn(wallet: mockWallet, amount: 500);

      expect(result.hash, '0xburnhash');
      expect(methods, ['kanari_buildCallFunction', 'kanari_callFunction']);
      expect(builtParams?['function'], 'burn_amount');
      expect(submittedParams?['nonce'], 20);
      expect(submittedParams?['signature'], isA<List>());
    });

    test('transferToken uses buildTokenTransfer before submit', () async {
      final mockWallet = _wallet();
      final methods = <String>[];
      Map<String, dynamic>? builtParams;
      Map<String, dynamic>? submittedParams;

      final mockClient = MockClient((request) async {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        final method = body['method'] as String;
        methods.add(method);

        if (method == 'kanari_buildTokenTransfer') {
          builtParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'sender': 'Ed25519:0x123',
                'package': '0x2',
                'module': 'demo_token',
                'function': 'transfer_amount',
                'type_args': const [],
                'args': const [
                  [10, 188],
                  [100, 0, 0, 0, 0, 0, 0, 0],
                  [
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    4,
                    86,
                  ],
                ],
                'object_inputs': [
                  {
                    'object_ref': {
                      'object_id':
                          '0x0000000000000000000000000000000000000000000000000000000000000abc',
                      'version': 9,
                      'digest': '0xdigest',
                    },
                    'owner': {'AddressOwner': 'Ed25519:0x123'},
                    'mutable': true,
                  },
                ],
                'gas_limit': 100000,
                'gas_price': 1000,
                'nonce': 8,
                'gas_payment': {
                  'payment_objects': const [],
                  'owner': 'Ed25519:0x123',
                  'budget': 100000,
                  'price': 1000,
                },
              },
              'id': 1,
            }),
            200,
          );
        }

        if (method == 'kanari_callFunction') {
          submittedParams = Map<String, dynamic>.from(body['params'] as Map);
          return http.Response(
            jsonEncode({
              'jsonrpc': '2.0',
              'result': {
                'hash': '0xtokenhash',
                'status': 'success',
                'gas_used': 75,
              },
              'id': 2,
            }),
            200,
          );
        }

        return http.Response('', 404);
      });

      final client = KanariClient('http://localhost/rpc', client: mockClient);
      final result = await client.transferToken(
        wallet: mockWallet,
        recipient: '0x456',
        tokenType: '0x2::demo_token::DEMO',
        amount: 100,
      );

      expect(result.hash, '0xtokenhash');
      expect(methods, ['kanari_buildTokenTransfer', 'kanari_callFunction']);
      expect(
        builtParams?['recipient'],
        '0x0000000000000000000000000000000000000000000000000000000000000456',
      );
      expect(builtParams?['token_type'], '0x2::demo_token::DEMO');
      expect(submittedParams?['nonce'], 8);
      expect(submittedParams?['signature'], isA<List>());
    });
  });
}

KanariWallet _wallet() {
  return KanariWallet(
    KeyPairData(
      privateKey: 'priv',
      publicKey: 'pub',
      address: '0x123',
      taggedAddress: 'Ed25519:0x123',
      rawPublicKey: Uint8List(32),
      curveType: 'Ed25519',
    ),
  );
}
