// modules/escrow/operations.dart
// Escrow transaction operations.

import 'dart:async';

import '../../client/kanari_client.dart';
import '../../core/bcs_utils.dart';
import '../../core/token_metadata.dart';
import '../../kanari_wallet.dart';
import '../../models/account.dart';
import '../../models/transaction.dart';
import '../transactions/constants.dart';
import 'constants.dart';

class EscrowOperations {
  final KanariClient rpc;

  const EscrowOperations(this.rpc);

  Future<void> _ensurePublished() async {
    try {
      await rpc.getModule(EscrowConstants.packageAddress, EscrowConstants.module);
    } catch (_) {
      throw StateError(
        'Escrow module is not deployed on this network. Publish '
        '${EscrowConstants.packageAddress}::${EscrowConstants.module} first.',
      );
    }
  }

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
    await _ensurePublished();
    final normalizedToken = BcsUtils.normalizeTokenType(tokenType);
    var ownedObjects = await rpc.getOwnedObjects(wallet.address);
    var coinObject = _findOwnedCoinObject(
      ownedObjects: ownedObjects,
      tokenType: normalizedToken,
      requiredAmount: amount,
    );

    if (isKanariType(normalizedToken)) {
      ownedObjects = await _prepareNativeGasCoinIfNeeded(
        ownedObjects: ownedObjects,
        collateralCoin: coinObject,
        requiredGas: gasLimit * gasPrice,
      );
      coinObject = _findOwnedCoinObject(
        ownedObjects: ownedObjects,
        tokenType: normalizedToken,
        requiredAmount: amount,
      );
    }

    final coinObjectId = BcsUtils.normalizeObjectId(coinObject.id);

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
      // Return the created object changes in this response. The Escrow screen
      // needs the new deal/proof IDs before the next checkpoint query catches
      // up with the submitted transaction.
      executeImmediate: true,
    );
  }

  /// Find owned Coin object for a specific token type
  ObjectInfo _findOwnedCoinObject({
    required List<ObjectInfo> ownedObjects,
    required String tokenType,
    int? requiredAmount,
  }) {
    final candidates = ownedObjects.where((obj) {
      final objToken = BcsUtils.extractCoinTypeFromObjectType(obj.type);
      if (objToken == null || !BcsUtils.tokenTypesEqual(objToken, tokenType)) {
        return false;
      }

      if (requiredAmount == null) return true;
      final balance = BcsUtils.readCoinObjectBalance(obj.data);
      return balance != null && balance >= requiredAmount;
    }).toList()
      ..sort((a, b) {
        final aBalance = BcsUtils.readCoinObjectBalance(a.data) ?? 0;
        final bBalance = BcsUtils.readCoinObjectBalance(b.data) ?? 0;
        return bBalance.compareTo(aBalance);
      });

    if (candidates.isNotEmpty) {
      return candidates.first;
    }

    throw Exception(
      'No spendable Coin<$tokenType> object found in wallet.\n'
      'You need to have actual Coin objects, not just balance.\n'
      'Try minting or transferring tokens first.',
    );
  }

  bool _hasSeparateNativeGasCoin({
    required List<ObjectInfo> ownedObjects,
    required String collateralCoinId,
    required int requiredGas,
  }) {
    final collateralId = BcsUtils.normalizeObjectId(collateralCoinId);
    return ownedObjects.any((obj) {
      if (BcsUtils.normalizeObjectId(obj.id) == collateralId) return false;
      final objToken = BcsUtils.extractCoinTypeFromObjectType(obj.type);
      if (objToken == null || !isKanariType(objToken)) return false;
      final balance = BcsUtils.readCoinObjectBalance(obj.data);
      return balance != null && balance >= requiredGas;
    });
  }

  Future<List<ObjectInfo>> _prepareNativeGasCoinIfNeeded({
    required List<ObjectInfo> ownedObjects,
    required ObjectInfo collateralCoin,
    required int requiredGas,
  }) async {
    if (_hasSeparateNativeGasCoin(
      ownedObjects: ownedObjects,
      collateralCoinId: collateralCoin.id,
      requiredGas: requiredGas,
    )) {
      return ownedObjects;
    }

    final collateralBalance = BcsUtils.readCoinObjectBalance(collateralCoin.data);
    throw Exception(
      'KANARI can be used in DeFi, but Move object execution requires a '
      'separate Coin<KANARI> gas object. This wallet has collateral balance '
      '${collateralBalance ?? 0} Mist in the selected coin, but no different '
      'native gas coin with at least $requiredGas Mist. Receive or fund a '
      'second Coin<KANARI> object, then retry.',
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
    objectInputIds: [dealObjectId, proofObjectId],
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
    objectInputIds: [dealObjectId, proofObjectId],
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
    objectInputIds: [dealObjectId, proofObjectId],
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
    List<String> objectInputIds = const [],
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    final objectInputs = <Map<String, dynamic>>[];
    for (final objectId in objectInputIds) {
      objectInputs.add(await _objectInputForAction(objectId));
    }

    return _executePublishedAction(
      wallet: wallet,
      functionName: functionName,
      typeArgs: [BcsUtils.normalizeTokenType(coinType)],
      args: args.build(),
      objectInputs: objectInputs,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
    );
  }

  Future<TransactionResult> _executePublishedAction({
    required KanariWallet wallet,
    required String functionName,
    required List<String> typeArgs,
    required List<List<int>> args,
    List<Map<String, dynamic>> objectInputs = const [],
    required int gasLimit,
    required int gasPrice,
  }) async {
    await _ensurePublished();
    return rpc.executeFunction(
      wallet: wallet,
      package: EscrowConstants.packageAddress,
      module: EscrowConstants.module,
      function: functionName,
      typeArgs: typeArgs,
      args: args,
      objectInputs: objectInputs,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      executeImmediate: true,
    );
  }

  Future<Map<String, dynamic>> _objectInputForAction(String objectId) async {
    final object = await rpc.getObject(objectId);
    final objectRef = <String, dynamic>{
      'object_id': BcsUtils.normalizeObjectId(object.id),
      'version': object.version,
      if (object.digest != null && object.digest!.isNotEmpty)
        'digest': object.digest,
    };

    return {
      'object_ref': objectRef,
      'owner': {'AddressOwner': BcsUtils.normalizeAddress(object.owner)},
      'mutable': true,
    };
  }
}
