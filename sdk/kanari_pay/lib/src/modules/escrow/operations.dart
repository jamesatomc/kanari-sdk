// modules/escrow/operations.dart
// Escrow transaction operations.

import '../../client/kanari_client.dart';
import '../../core/bcs_utils.dart';
import '../../kanari_wallet.dart';
import '../../models/transaction.dart';
import '../transactions/constants.dart';
import 'constants.dart';

class EscrowOperations {
  final KanariClient rpc;

  const EscrowOperations(this.rpc);

  /// Validate transaction result
  TransactionResult requireSuccess(TransactionResult result) {
    if (result.status.toLowerCase() == 'failed') {
      throw Exception(
        result.errorMessage?.isNotEmpty == true
            ? result.errorMessage!
            : 'Escrow transaction failed on-chain.',
      );
    }
    return result;
  }

  /// Create new escrow deal
  Future<TransactionResult> createDeal({
    required KanariWallet wallet,
    required String dealId,
    required String sellerAddress,
    required int amount,
    required String description,
    required String tokenType,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    final normalizedToken = BcsUtils.normalizeTokenType(tokenType);
    final coinObjectId = await _findOwnedCoinObjectId(
      ownerAddress: wallet.address,
      tokenType: normalizedToken,
    );

    final args = TransactionArgs()
      ..addString(dealId)
      ..addAddress(sellerAddress)
      ..addAmount(amount)
      ..addString(description)
      ..addObjectId(coinObjectId);

    return rpc.executeFunction(
      wallet: wallet,
      package: EscrowConstants.packageAddress,
      module: EscrowConstants.module,
      function: EscrowConstants.fnCreateDeal,
      typeArgs: [normalizedToken],
      args: args.build(),
      gasLimit: gasLimit,
      gasPrice: gasPrice,
    );
  }

  /// Find owned Coin object for a specific token type
  Future<String> _findOwnedCoinObjectId({
    required String ownerAddress,
    required String tokenType,
  }) async {
    final account = await rpc.getAccount(ownerAddress);

    for (final obj in account.ownedObjects ?? const []) {
      final objToken = BcsUtils.extractCoinTypeFromObjectType(obj.type);
      if (objToken != null && BcsUtils.tokenTypesEqual(objToken, tokenType)) {
        return BcsUtils.normalizeObjectId(obj.id);
      }
    }

    throw Exception(
      'No spendable Coin<$tokenType> object found in wallet.\n'
      'You need to have actual Coin objects, not just balance.\n'
      'Try minting or transferring tokens first.',
    );
  }

  /// Confirm delivery (Seller only)
  Future<TransactionResult> confirmDelivery({
    required KanariWallet wallet,
    required String dealObjectId,
    required String coinType,
    required String proofObjectId,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) => _executeAction(
    wallet: wallet,
    coinType: coinType,
    functionName: EscrowConstants.fnConfirmDelivery,
    args: TransactionArgs()
      ..addObjectId(dealObjectId)
      ..addObjectId(proofObjectId),
    gasLimit: gasLimit,
    gasPrice: gasPrice,
  );

  /// Release funds (Buyer only)
  Future<TransactionResult> releaseFunds({
    required KanariWallet wallet,
    required String dealObjectId,
    required String coinType,
    required String proofObjectId,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) => _executeAction(
    wallet: wallet,
    coinType: coinType,
    functionName: EscrowConstants.fnReleaseFunds,
    args: TransactionArgs()
      ..addObjectId(dealObjectId)
      ..addObjectId(proofObjectId),
    gasLimit: gasLimit,
    gasPrice: gasPrice,
  );

  /// Raise dispute
  Future<TransactionResult> raiseDispute({
    required KanariWallet wallet,
    required String dealObjectId,
    required String coinType,
    required String reason,
    required String proofObjectId,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) => _executeAction(
    wallet: wallet,
    coinType: coinType,
    functionName: EscrowConstants.fnRaiseDispute,
    args: TransactionArgs()
      ..addObjectId(dealObjectId)
      ..addObjectId(proofObjectId)
      ..addString(reason),
    gasLimit: gasLimit,
    gasPrice: gasPrice,
  );

  /// Generic escrow action executor
  Future<TransactionResult> _executeAction({
    required KanariWallet wallet,
    required String coinType,
    required String functionName,
    required TransactionArgs args,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) {
    return rpc.executeFunction(
      wallet: wallet,
      package: EscrowConstants.packageAddress,
      module: EscrowConstants.module,
      function: functionName,
      typeArgs: [BcsUtils.normalizeTokenType(coinType)],
      args: args.build(),
      gasLimit: gasLimit,
      gasPrice: gasPrice,
    );
  }
}
