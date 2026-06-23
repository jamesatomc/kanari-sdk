import 'dart:typed_data';

import 'package:bcs/bcs.dart';
import 'package:http/http.dart' as http;
import 'package:kanari_crypto/kanari_crypto.dart';

import '../../core/bcs_utils.dart';
import '../../core/token_utils.dart' as token_utils;
import '../../core/rpc_utils.dart';
import '../../kanari_wallet.dart';
import '../../models/account.dart';
import '../../models/transaction.dart';
import '../queries.dart';
import 'constants.dart';

class TransactionOperations {
  final String url;
  final QueriesModule queries;
  final http.Client client;

  static final _transactionBcs = Bcs.enumeration('Transaction', {
    'PublishModule': Bcs.struct('PublishModule', {
      'sender': Bcs.string(),
      'module_bytes': Bcs.vector(Bcs.u8()),
      'module_name': Bcs.string(),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      'sequence_number': Bcs.u64(),
    }),
    'ExecuteFunction': Bcs.struct('ExecuteFunction', {
      'sender': Bcs.string(),
      'module': Bcs.string(),
      'function': Bcs.string(),
      'type_args': Bcs.vector(Bcs.string()),
      'args': Bcs.vector(Bcs.vector(Bcs.u8())),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      'sequence_number': Bcs.u64(),
    }),
    'Transfer': Bcs.struct('Transfer', {
      'from': Bcs.string(),
      'to': Bcs.string(),
      'amount': Bcs.u64(),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      'sequence_number': Bcs.u64(),
    }),
    'Burn': Bcs.struct('Burn', {
      'from': Bcs.string(),
      'amount': Bcs.u64(),
      'gas_limit': Bcs.u64(),
      'gas_price': Bcs.u64(),
      'sequence_number': Bcs.u64(),
    }),
  });

  TransactionOperations(this.url, this.queries, this.client);

  String _getSenderForTx(KanariWallet wallet) => wallet.taggedAddress;

  String? _normalizedTokenTypeFromCoinObject(String objectType) {
    final start = objectType.indexOf('<');
    final end = objectType.lastIndexOf('>');
    if (start == -1 || end == -1 || end <= start) return null;

    final outerType = objectType.substring(0, start);
    if (!outerType.endsWith('::coin::Coin') &&
        !outerType.endsWith('::coin::coin::Coin')) {
      return null;
    }

    return BcsUtils.normalizeTokenType(objectType.substring(start + 1, end));
  }

  int? _readCoinBalance(List<int> data) {
    if (data.length < 40) return null;
    final bytes = Uint8List.fromList(data.sublist(32, 40));
    return ByteData.sublistView(bytes).getUint64(0, Endian.little);
  }

  Future<TransactionResult> _signAndSubmit({
    required KanariWallet wallet,
    required Map<String, dynamic> txData,
    required String rpcMethod,
    required Map<String, dynamic> params,
  }) async {
    final serializedTx = _transactionBcs.serialize(txData).toBytes();

    List<int> messageToSign;
    try {
      messageToSign = await blake3HashApi(data: serializedTx);
    } catch (e) {
      if (e.toString().contains(
        'flutter_rust_bridge has not been initialized',
      )) {
        messageToSign = serializedTx;
      } else {
        rethrow;
      }
    }

    final signature = await wallet.sign(messageToSign);
    params['signature'] = signature.toList();

    final resp = await RpcUtils.request(
      client,
      url,
      rpcMethod,
      params,
      (j) => TransactionResult.fromJson(j as Map<String, dynamic>),
    );

    if (resp.error != null) throw Exception(resp.error!.message);

    final result = resp.result!;
    final status = result.status.toLowerCase();
    if (status != 'pending' &&
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

  Future<TransactionResult> publishModule({
    required KanariWallet wallet,
    required List<int> moduleBytes,
    required String moduleName,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
    bool? executeImmediate,
  }) async {
    final account = await queries.getAccount(wallet.address);
    final sender = _getSenderForTx(wallet);
    final txData = {
      'PublishModule': {
        'sender': sender,
        'module_bytes': moduleBytes,
        'module_name': moduleName,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'sequence_number': account.sequenceNumber,
      },
    };
    final params = {
      'sender': sender,
      'module_bytes': moduleBytes,
      'module_name': moduleName,
      'gas_limit': gasLimit,
      'gas_price': gasPrice,
      'sequence_number': account.sequenceNumber,
      'execute_immediate': executeImmediate,
    };

    return _signAndSubmit(
      wallet: wallet,
      txData: txData,
      rpcMethod: TransactionConstants.rpcPublishModule,
      params: params,
    );
  }

  String _findSpendableCoinObjectId(AccountInfo account, String tokenType) {
    final wantedToken = BcsUtils.normalizeTokenType(tokenType);
    for (final obj in account.ownedObjects ?? const []) {
      if (_normalizedTokenTypeFromCoinObject(obj.type) != wantedToken) continue;
      final balance = _readCoinBalance(obj.data);
      if (balance != null && balance > 0) return obj.id;
    }
    throw Exception(
      'No spendable Coin<$tokenType> object found.\n'
      'This wallet needs a spendable Coin object for the selected token.',
    );
  }

  Future<TransactionResult> _transferCoinObject({
    required KanariWallet wallet,
    required String recipient,
    required String tokenType,
    required int amount,
    required int gasLimit,
    required int gasPrice,
  }) async {
    final account = await queries.getAccount(wallet.address);
    final normalizedRecipient = BcsUtils.normalizeAddress(recipient);
    final wantedToken = BcsUtils.normalizeTokenType(tokenType);
    final coinObjectId = _findSpendableCoinObjectId(account, wantedToken);
    final parts = wantedToken.split('::');
    if (parts.length < 3) {
      throw ArgumentError(
        'Invalid token format. Expected: address::module::struct',
      );
    }

    return executeFunction(
      wallet: wallet,
      package: parts[0],
      module: parts[1],
      function: 'transfer_amount',
      args: <List<int>>[
        BcsUtils.hexToBytes(BcsUtils.normalizeObjectId(coinObjectId)),
        BcsUtils.encodeU64(amount),
        BcsUtils.hexToBytes(normalizedRecipient),
      ],
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      executeImmediate: true,
    );
  }

  /// Transfers the native KANARI balance through the canonical account path.
  ///
  /// Native KANARI must not be routed through the generic Coin-object call path:
  /// account gas debits are maintained outside the Move Coin object and a later
  /// object recomputation can otherwise restore an earlier gas debit. The server
  /// represents this request as the canonical 0x2::kanari::transfer_amount native
  /// transaction, so the signed BCS payload below must match that representation.
  Future<TransactionResult> transfer({
    required KanariWallet wallet,
    required String recipient,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    final account = await queries.getAccount(wallet.address);
    final sender = _getSenderForTx(wallet);
    final normalizedRecipient = BcsUtils.normalizeAddress(recipient);
    final txData = {
      'ExecuteFunction': {
        'sender': sender,
        'module': '0x2::kanari',
        'function': 'transfer_amount',
        'type_args': <String>[],
        'args': <List<int>>[
          BcsUtils.encodeU64(amount),
          BcsUtils.encodeString(normalizedRecipient),
        ],
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'sequence_number': account.sequenceNumber,
      },
    };
    final params = {
      'sender': sender,
      'recipient': normalizedRecipient,
      'amount': amount,
      'gas_limit': gasLimit,
      'gas_price': gasPrice,
      'sequence_number': account.sequenceNumber,
      'execute_immediate': true,
    };

    return _signAndSubmit(
      wallet: wallet,
      txData: txData,
      rpcMethod: TransactionConstants.rpcSubmitTransaction,
      params: params,
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
    int gasPrice = 0,
    bool? executeImmediate,
  }) async {
    final account = await queries.getAccount(wallet.address);
    final sender = _getSenderForTx(wallet);
    final packageAddress = BcsUtils.normalizeAnyAddress(package);
    final txData = {
      'ExecuteFunction': {
        'sender': sender,
        'module': '$packageAddress::$module',
        'function': function,
        'type_args': typeArgs,
        'args': args,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'sequence_number': account.sequenceNumber,
      },
    };
    final params = {
      'sender': sender,
      'package': packageAddress,
      'module': module,
      'function': function,
      'type_args': typeArgs,
      'args': args,
      'gas_limit': gasLimit,
      'gas_price': gasPrice,
      'sequence_number': account.sequenceNumber,
      'execute_immediate': executeImmediate,
    };

    return _signAndSubmit(
      wallet: wallet,
      txData: txData,
      rpcMethod: TransactionConstants.rpcCallFunction,
      params: params,
    );
  }

  Future<TransactionResult> burn({
    required KanariWallet wallet,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    final account = await queries.getAccount(wallet.address);
    final sender = _getSenderForTx(wallet);
    final txData = {
      'Burn': {
        'from': sender,
        'amount': amount,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'sequence_number': account.sequenceNumber,
      },
    };
    final params = {
      'sender': sender,
      'amount': amount,
      'gas_limit': gasLimit,
      'gas_price': gasPrice,
      'sequence_number': account.sequenceNumber,
    };

    return _signAndSubmit(
      wallet: wallet,
      txData: txData,
      rpcMethod: TransactionConstants.rpcSubmitTransaction,
      params: params,
    );
  }

  Future<TransactionResult> transferToken({
    required KanariWallet wallet,
    required String recipient,
    required String tokenType,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = 0,
  }) {
    return _transferCoinObject(
      wallet: wallet,
      recipient: recipient,
      tokenType: tokenType,
      amount: amount,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
    );
  }
}
