import 'dart:typed_data';

import 'package:bcs/bcs.dart';
import 'package:http/http.dart' as http;
import 'package:kanari_crypto/kanari_crypto.dart';

import '../../core/bcs_utils.dart';
import '../../core/rpc_utils.dart';
import '../../core/token_utils.dart' as token_utils;
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
  });

  TransactionOperations(this.url, this.queries, this.client);

  void _requirePositiveAmount(int amount, String name) {
    if (amount <= 0) {
      throw ArgumentError.value(amount, name, 'must be greater than 0');
    }
  }

  String _getSenderForTx(KanariWallet wallet) => wallet.taggedAddress;

  String? _normalizedTokenTypeFromCoinObject(String objectType) {
    final start = objectType.indexOf('<');
    final end = objectType.lastIndexOf('>');
    if (start == -1 || end == -1 || end <= start) {
      return null;
    }

    final outerType = objectType.substring(0, start);
    if (!outerType.endsWith('::coin::Coin') &&
        !outerType.endsWith('::coin::coin::Coin')) {
      return null;
    }

    final tokenType = objectType.substring(start + 1, end);
    return BcsUtils.normalizeTokenType(tokenType);
  }

  int? _readCoinBalance(List<int> data) {
    if (data.length < 40) {
      return null;
    }

    final balanceBytes = Uint8List.fromList(data.sublist(32, 40));
    return ByteData.sublistView(balanceBytes).getUint64(0, Endian.little);
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
    final senderAddress = _getSenderForTx(wallet);

    final txData = {
      'PublishModule': {
        'sender': senderAddress,
        'module_bytes': moduleBytes,
        'module_name': moduleName,
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'sequence_number': account.sequenceNumber,
      },
    };

    final params = {
      'sender': senderAddress,
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

  String _findSpendableCoinObjectId(
    AccountInfo account,
    String tokenType,
    int amount, {
    bool requireExactAmount = false,
  }) {
    final wantedToken = BcsUtils.normalizeTokenType(tokenType);

    for (final obj in account.ownedObjects ?? const []) {
      final objToken = _normalizedTokenTypeFromCoinObject(obj.type);
      if (objToken == null ||
          !BcsUtils.tokenTypesEqual(objToken, wantedToken)) {
        continue;
      }

      final coinBalance = _readCoinBalance(obj.data);
      if (coinBalance == null || coinBalance < amount) {
        continue;
      }
      if (requireExactAmount && coinBalance != amount) {
        continue;
      }

      return obj.id;
    }

    throw Exception(
      requireExactAmount
          ? 'No Coin<$tokenType> object with exactly $amount found.\n'
                'This transfer entry moves the whole Coin object.'
          : 'No spendable Coin<$tokenType> object with at least $amount found.\n'
                'This wallet needs one Coin object large enough for this transfer.',
    );
  }

  Future<TransactionResult> _transferCoinObject({
    required KanariWallet wallet,
    required String recipient,
    required String tokenType,
    required int amount,
    required int gasLimit,
    required int gasPrice,
    String function = 'transfer_amount',
    bool includeAmountArg = true,
    bool requireExactAmount = false,
  }) async {
    _requirePositiveAmount(amount, 'amount');
    final account = await queries.getAccount(wallet.address);
    final normalizedRecipient = BcsUtils.normalizeAddress(recipient);
    final wantedToken = BcsUtils.normalizeTokenType(tokenType);
    final coinObjectId = _findSpendableCoinObjectId(
      account,
      wantedToken,
      amount,
      requireExactAmount: requireExactAmount,
    );

    final parts = wantedToken.split('::');
    if (parts.length < 3) {
      throw ArgumentError(
        'Invalid token format. Expected: address::module::struct',
      );
    }

    final args = <List<int>>[
      BcsUtils.hexToBytes(BcsUtils.normalizeObjectId(coinObjectId)),
      if (includeAmountArg) BcsUtils.encodeU64(amount),
      BcsUtils.hexToBytes(normalizedRecipient),
    ];

    return executeFunction(
      wallet: wallet,
      package: parts[0],
      module: parts[1],
      function: function,
      typeArgs: const <String>[],
      args: args,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      executeImmediate: true,
    );
  }

  Future<TransactionResult> transfer({
    required KanariWallet wallet,
    required String recipient,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) {
    return _transferCoinObject(
      wallet: wallet,
      recipient: recipient,
      tokenType: token_utils.kanariTokenType,
      amount: amount,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      function: 'transfer',
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
    final account = await queries.getAccount(wallet.address);
    final senderAddress = _getSenderForTx(wallet);
    final packageAddress = BcsUtils.normalizeAnyAddress(package);

    final txData = {
      'ExecuteFunction': {
        'sender': senderAddress,
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
      'sender': senderAddress,
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
    _requirePositiveAmount(amount, 'amount');
    final account = await queries.getAccount(wallet.address);
    final senderAddress = _getSenderForTx(wallet);

    final txData = {
      'ExecuteFunction': {
        'sender': senderAddress,
        'module': TransactionConstants.nativeKanariModule,
        'function': TransactionConstants.nativeBurnAmountFunction,
        'type_args': const <String>[],
        'args': [BcsUtils.encodeU64(amount)],
        'gas_limit': gasLimit,
        'gas_price': gasPrice,
        'sequence_number': account.sequenceNumber,
      },
    };

    final params = {
      'sender': senderAddress,
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
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
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
