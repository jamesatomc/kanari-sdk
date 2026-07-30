import 'dart:typed_data';

import 'package:bcs/bcs.dart';
import 'package:http/http.dart' as http;
import 'package:kanari_crypto/kanari_crypto.dart';

import '../../core/bcs_utils.dart';
import '../../core/rpc_utils.dart';
import '../../kanari_wallet.dart';
import '../../models/transaction.dart';
import 'constants.dart';

class TransactionOperations {
  final String url;
  final http.Client client;
  static const _bcsWireNonceField = 'nonce';

  static final _objectRefBcs = Bcs.struct('ObjectRef', {
    'object_id': Bcs.string(),
    'version': Bcs.option(Bcs.u64()),
    'digest': Bcs.option(Bcs.string()),
  });

  static final _objectOwnerKindBcs = Bcs.enumeration('ObjectOwnerKind', {
    'AddressOwner': Bcs.string(),
    'Shared': null,
    'Immutable': null,
  });

  static final _objectInputBcs = Bcs.struct('ObjectInput', {
    'object_ref': _objectRefBcs,
    'owner': Bcs.option(_objectOwnerKindBcs),
    'mutable': Bcs.u8(),
  });

  static final _gasPaymentBcs = Bcs.struct('GasPayment', {
    'payment_objects': Bcs.vector(_objectRefBcs),
    'owner': Bcs.string(),
    'budget': Bcs.u64(),
    'price': Bcs.u64(),
  });

  static final _transactionBcs = Bcs.enumeration('Transaction', {
    'PublishModule': Bcs.struct('PublishModule', {
      'sender': Bcs.string(),
      'module_bytes': Bcs.vector(Bcs.u8()),
      'module_name': Bcs.string(),
      'gas_payment': Bcs.option(_gasPaymentBcs),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      _bcsWireNonceField: Bcs.u64(),
    }),
    // Keep this variant even though the current Flutter facade does not
    // publish packages. BCS enum indexes must match the Rust Transaction
    // declaration: PublishModule, PublishPackage, ExecuteFunction.
    'PublishPackage': Bcs.struct('PublishPackage', {
      'sender': Bcs.string(),
      'modules': Bcs.vector(
        Bcs.struct('PublishedModule', {
          'module_name': Bcs.string(),
          'module_bytes': Bcs.vector(Bcs.u8()),
        }),
      ),
      'gas_payment': Bcs.option(_gasPaymentBcs),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      _bcsWireNonceField: Bcs.u64(),
    }),
    // Keep these variants even though the current Flutter facade does not
    // expose upgrade calls yet. BCS enum indexes must match the Rust
    // Transaction declaration exactly:
    // PublishModule, PublishPackage, UpgradeModule, UpgradePackage,
    // ExecuteFunction.
    'UpgradeModule': Bcs.struct('UpgradeModule', {
      'sender': Bcs.string(),
      'module_bytes': Bcs.vector(Bcs.u8()),
      'module_name': Bcs.string(),
      'gas_payment': Bcs.option(_gasPaymentBcs),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      _bcsWireNonceField: Bcs.u64(),
    }),
    'UpgradePackage': Bcs.struct('UpgradePackage', {
      'sender': Bcs.string(),
      'modules': Bcs.vector(
        Bcs.struct('PublishedModule', {
          'module_name': Bcs.string(),
          'module_bytes': Bcs.vector(Bcs.u8()),
        }),
      ),
      'gas_payment': Bcs.option(_gasPaymentBcs),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      _bcsWireNonceField: Bcs.u64(),
    }),
    'ExecuteFunction': Bcs.struct('ExecuteFunction', {
      'sender': Bcs.string(),
      'module': Bcs.string(),
      'function': Bcs.string(),
      'type_args': Bcs.vector(Bcs.string()),
      'args': Bcs.vector(Bcs.vector(Bcs.u8())),
      'object_inputs': Bcs.vector(_objectInputBcs),
      'gas_payment': Bcs.option(_gasPaymentBcs),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      _bcsWireNonceField: Bcs.u64(),
    }),
  });

  TransactionOperations(this.url, this.client);

  void _requirePositiveAmount(int amount, String name) {
    if (amount <= 0) {
      throw ArgumentError.value(amount, name, 'must be greater than 0');
    }
  }

  String _getSenderForTx(KanariWallet wallet) => wallet.taggedAddress;

  Map<String, dynamic> _objectRefForBcs(Map<String, dynamic> objectRef) {
    return {
      'object_id': objectRef['object_id'],
      'version': objectRef['version'],
      'digest': objectRef['digest'],
    };
  }

  Map<String, dynamic>? _gasPaymentForBcs(dynamic gasPayment) {
    if (gasPayment == null) {
      return null;
    }

    final map = Map<String, dynamic>.from(gasPayment as Map);
    final paymentObjects = (map['payment_objects'] as List? ?? const [])
        .map((item) => _objectRefForBcs(Map<String, dynamic>.from(item as Map)))
        .toList();

    return {
      'payment_objects': paymentObjects,
      'owner': map['owner'],
      'budget': map['budget'],
      'price': map['price'],
    };
  }

  List<Map<String, dynamic>> _objectInputsForBcs(dynamic objectInputs) {
    final items = objectInputs as List? ?? const [];
    return items.map((item) {
      final map = Map<String, dynamic>.from(item as Map);
      final mutable = map['mutable'] == true ? 1 : 0;
      final owner = map['owner'] == null
          ? null
          : Map<String, dynamic>.from(map['owner'] as Map);

      return {
        'object_ref': _objectRefForBcs(
          Map<String, dynamic>.from(map['object_ref'] as Map),
        ),
        'owner': owner,
        'mutable': mutable,
      };
    }).toList();
  }

  int _preparedNonce(Map<String, dynamic> prepared) {
    final value = prepared['nonce'];
    if (value is int && value > 0) {
      return value;
    }
    if (value is num && value > 0) {
      return value.toInt();
    }
    throw Exception('Prepared transaction is missing a valid nonce');
  }

  Map<String, dynamic> _bcsWireNonceEntry(int nonce) => {
    _bcsWireNonceField: nonce,
  };

  Map<String, dynamic> _finalizeSignedRequest(
    Map<String, dynamic> prepared,
    Uint8List signature,
  ) {
    final nonce = _preparedNonce(prepared);
    return {
      ...prepared,
      'nonce': nonce,
      'signature': signature.toList(),
    };
  }

  Future<List<int>> _signingHash(List<int> serializedTx) async {
    return blake3HashApi(data: serializedTx);
  }

  Future<Map<String, dynamic>> _requestPrepared(
    String method,
    Map<String, dynamic> params,
  ) async {
    final resp = await RpcUtils.request(
      client,
      url,
      method,
      params,
      (json) => Map<String, dynamic>.from(json as Map),
    );

    if (resp.error != null) {
      throw Exception(resp.error!.message);
    }

    final result = resp.result;
    if (result == null) {
      throw Exception('RPC returned no result for $method');
    }

    return Map<String, dynamic>.from(result);
  }

  Future<TransactionResult> _submitTransaction(
    String method,
    Map<String, dynamic> params,
  ) async {
    final resp = await RpcUtils.request(
      client,
      url,
      method,
      params,
      (json) => TransactionResult.fromJson(json as Map<String, dynamic>),
    );

    if (resp.error != null) {
      throw Exception(resp.error!.message);
    }

    final result = resp.result!;
    final status = result.status.toLowerCase();
    if (status != 'pending' &&
        status != 'simulated_pending' &&
        status != 'executed' &&
        status != 'committed' &&
        status != 'success') {
      throw Exception(
        result.errorMessage?.isNotEmpty == true
            ? result.errorMessage
            : 'Transaction was not successful (status: ${result.status}, hash: ${result.hash})',
      );
    }

    return result;
  }

  Future<Map<String, dynamic>> _signPublishRequest(
    KanariWallet wallet,
    Map<String, dynamic> prepared,
  ) async {
    final txData = {
      'PublishModule': {
        'sender': prepared['sender'],
        'module_bytes': prepared['module_bytes'],
        'module_name': prepared['module_name'],
        'gas_payment': _gasPaymentForBcs(prepared['gas_payment']),
        'gas_limit': prepared['gas_limit'],
        'gas_price': prepared['gas_price'],
        ..._bcsWireNonceEntry(_preparedNonce(prepared)),
      },
    };

    final txBytes = _transactionBcs.serialize(txData).toBytes();
    final signature = await wallet.sign(await _signingHash(txBytes));
    return _finalizeSignedRequest(prepared, signature);
  }

  Future<Map<String, dynamic>> _signCallRequest(
    KanariWallet wallet,
    Map<String, dynamic> prepared,
  ) async {
    final txData = {
      'ExecuteFunction': {
        'sender': prepared['sender'],
        'module': '${prepared['package']}::${prepared['module']}',
        'function': prepared['function'],
        'type_args': prepared['type_args'] ?? const <String>[],
        'args': prepared['args'] ?? const <List<int>>[],
        'object_inputs': _objectInputsForBcs(prepared['object_inputs']),
        'gas_payment': _gasPaymentForBcs(prepared['gas_payment']),
        'gas_limit': prepared['gas_limit'],
        'gas_price': prepared['gas_price'],
        ..._bcsWireNonceEntry(_preparedNonce(prepared)),
      },
    };

    final txBytes = _transactionBcs.serialize(txData).toBytes();
    final signature = await wallet.sign(await _signingHash(txBytes));
    return _finalizeSignedRequest(prepared, signature);
  }

  Future<Map<String, dynamic>> _signNativeTransferRequest(
    KanariWallet wallet,
    Map<String, dynamic> prepared,
  ) async {
    final sender = prepared['sender'] as String;
    final recipient = prepared['recipient'] as String;
    final coinObjectRef = _objectRefForBcs(Map<String, dynamic>.from(
      prepared['coin_object_ref'] as Map,
    ));

    final txData = {
      'ExecuteFunction': {
        'sender': sender,
        'module': TransactionConstants.nativeKanariModule,
        'function': 'transfer',
        'type_args': const <String>[],
        'args': [
          BcsUtils.hexToBytes(coinObjectRef['object_id'] as String),
          BcsUtils.encodeU64(prepared['amount'] as int),
          BcsUtils.hexToBytes(BcsUtils.normalizeAddress(recipient)),
        ],
        'object_inputs': [
          {
            'object_ref': coinObjectRef,
            'owner': {'AddressOwner': sender},
            'mutable': 1,
          },
        ],
        'gas_payment': _gasPaymentForBcs(prepared['gas_payment']),
        'gas_limit': prepared['gas_limit'],
        'gas_price': prepared['gas_price'],
        ..._bcsWireNonceEntry(_preparedNonce(prepared)),
      },
    };

    final txBytes = _transactionBcs.serialize(txData).toBytes();
    final signature = await wallet.sign(await _signingHash(txBytes));
    return _finalizeSignedRequest(prepared, signature);
  }

  Future<TransactionResult> publishModule({
    required KanariWallet wallet,
    required List<int> moduleBytes,
    required String moduleName,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
    bool? executeImmediate,
  }) async {
    final prepared = await _requestPrepared(
      TransactionConstants.rpcBuildPublishModule,
      {
        'sender': _getSenderForTx(wallet),
        'module_bytes': moduleBytes,
        'module_name': moduleName,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'execute_immediate': executeImmediate,
      },
    );

    final signed = await _signPublishRequest(wallet, prepared);
    return _submitTransaction(TransactionConstants.rpcPublishModule, signed);
  }

  Future<TransactionResult> transfer({
    required KanariWallet wallet,
    required String recipient,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
    List<String> excludedObjectIds = const [],
  }) async {
    _requirePositiveAmount(amount, 'amount');

    final prepared = await _requestPrepared(
      TransactionConstants.rpcBuildNativeTransfer,
      {
        'sender': _getSenderForTx(wallet),
        'recipient': BcsUtils.normalizeAddress(recipient),
        'amount': amount,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'excluded_object_ids': excludedObjectIds
            .map(BcsUtils.normalizeAddress)
            .toList(growable: false),
        'execute_immediate': true,
      },
    );

    final signed = await _signNativeTransferRequest(wallet, prepared);
    return _submitTransaction(
      TransactionConstants.rpcSubmitObjectTransfer,
      signed,
    );
  }

  Future<TransactionResult> executeFunction({
    required KanariWallet wallet,
    required String package,
    required String module,
    required String function,
    List<String> typeArgs = const [],
    List<List<int>> args = const [],
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
    bool? executeImmediate,
  }) async {
    final prepared = await _requestPrepared(
      TransactionConstants.rpcBuildCallFunction,
      {
        'sender': _getSenderForTx(wallet),
        'package': BcsUtils.normalizeAnyAddress(package),
        'module': module,
        'function': function,
        'type_args': typeArgs,
        'args': args,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'execute_immediate': executeImmediate,
      },
    );

    final signed = await _signCallRequest(wallet, prepared);
    return _submitTransaction(TransactionConstants.rpcCallFunction, signed);
  }

  Future<TransactionResult> burn({
    required KanariWallet wallet,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    _requirePositiveAmount(amount, 'amount');
    return executeFunction(
      wallet: wallet,
      package: '0x2',
      module: 'kanari',
      function: TransactionConstants.nativeBurnAmountFunction,
      args: [BcsUtils.encodeU64(amount)],
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      executeImmediate: true,
    );
  }

  Future<TransactionResult> transferToken({
    required KanariWallet wallet,
    required String recipient,
    required String tokenType,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    _requirePositiveAmount(amount, 'amount');

    final prepared = await _requestPrepared(
      TransactionConstants.rpcBuildTokenTransfer,
      {
        'sender': _getSenderForTx(wallet),
        'recipient': BcsUtils.normalizeAddress(recipient),
        'token_type': BcsUtils.normalizeTokenType(tokenType),
        'amount': amount,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'execute_immediate': true,
      },
    );

    final signed = await _signCallRequest(wallet, prepared);
    return _submitTransaction(TransactionConstants.rpcCallFunction, signed);
  }
}
