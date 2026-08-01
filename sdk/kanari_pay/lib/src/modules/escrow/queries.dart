// modules/escrow/queries.dart
// Escrow view functions and queries.

import 'dart:convert';

import '../../client/kanari_client.dart';
import '../../core/bcs_utils.dart';
import '../../kanari_wallet.dart';
import '../../models/account.dart';
import '../../models/transaction.dart';
import 'constants.dart';

class EscrowQueries {
  final KanariClient rpc;

  const EscrowQueries(this.rpc);

  /// Get deal state by Object ID
  Future<int> getDealStateByObjectId({
    required KanariWallet wallet,
    required String dealObjectId,
    required String coinType,
  }) async {
    final result = await _callViewFunction(
      wallet: wallet,
      functionName: EscrowConstants.fnGetState,
      coinType: coinType,
      args: TransactionArgs(),
      objectInputId: dealObjectId,
    );

    if (result.isEmpty) return 0;

    return _jsonInt(_unwrapViewResult(result.first));
  }

  /// Get all deals for buyer
  Future<List<Map<String, dynamic>>> getAllDeals({
    required KanariWallet wallet,
    required String buyerAddress,
  }) async {
    final allObjects = await _getEscrowCandidateObjects(buyerAddress);

    final dealObjects = allObjects
        .where((obj) => _isEscrowDealType(obj.type))
        .toList();

    final proofObjects = allObjects
        .where((obj) => _isEscrowProofType(obj.type))
        .toList();

    final deals = <Map<String, dynamic>>[];

    for (final obj in dealObjects) {
      final objectId = obj.id;
      final objectType = obj.type;
      final coinType = _extractEscrowDealCoinType(objectType);

      if (coinType == null) {
        continue;
      }

      Map<String, dynamic> dealDetails;
      int state;
      String? dealLabel;
      String? proofId;
      try {
        dealDetails = await getDealDetailsByObjectId(
          wallet: wallet,
          dealObjectId: objectId,
          coinType: coinType,
        );
        state = await getDealStateByObjectId(
          wallet: wallet,
          dealObjectId: objectId,
          coinType: coinType,
        );
        dealLabel = await _dealLabelForObject(obj);
        proofId = await _matchingProofObjectId(
          proofObjects,
          dealLabel,
        );
      } catch (error) {
        if (_isObjectInputNotFoundError(error)) {
          continue;
        }
        rethrow;
      }

      if (dealLabel == null) {
        continue;
      }

      deals.add({
        'object_id': objectId,
        'coin_type': coinType,
        'object_type': objectType,
        'proof_id': proofId,
        'state': state,
        ...dealDetails,
      });
    }

    return deals;
  }

  /// Get deal details by Object ID
  Future<Map<String, dynamic>> getDealDetailsByObjectId({
    required KanariWallet wallet,
    required String dealObjectId,
    required String coinType,
  }) async {
    final result = await _callViewFunction(
      wallet: wallet,
      functionName: EscrowConstants.fnGetDealDetails,
      coinType: coinType,
      args: TransactionArgs(),
      objectInputId: dealObjectId,
    );

    if (result.isEmpty) {
      return {};
    }

    final resultValue = _unwrapViewResult(result.first);
    if (resultValue is List && resultValue.length >= 4) {
      return {
        'deal_id': resultValue[0].toString(),
        'buyer': resultValue[1].toString(),
        'seller': resultValue[2].toString(),
        'amount': _jsonInt(resultValue[3]),
      };
    }

    return {};
  }

  Future<Map<String, dynamic>?> getDealFromEffects({
    required KanariWallet wallet,
    required TransactionEffectsInfo effects,
    required String buyerAddress,
  }) async {
    final dealChanges = effects.objectChanges
        .where((change) => _isEscrowDealType(change.objectType ?? ''))
        .toList();
    final proofChanges = effects.objectChanges
        .where((change) => _isEscrowProofType(change.objectType ?? ''))
        .toList();

    for (final dealChange in dealChanges) {
      final objectId = dealChange.objectRef.objectId;
      if (objectId.isEmpty) continue;

      final owner = dealChange.owner;
      if (owner != null &&
          owner.isNotEmpty &&
          !_sameAddress(owner, buyerAddress)) {
        continue;
      }

      final objectType = dealChange.objectType ?? '';
      final coinType = _extractEscrowDealCoinType(objectType);
      if (coinType == null) {
        continue;
      }

      try {
        final details = await getDealDetailsByObjectId(
          wallet: wallet,
          dealObjectId: objectId,
          coinType: coinType,
        );
        final state = await getDealStateByObjectId(
          wallet: wallet,
          dealObjectId: objectId,
          coinType: coinType,
        );
        final proofId = _matchingProofId(proofChanges, buyerAddress);

        return {
          'object_id': objectId,
          'coin_type': coinType,
          'object_type': objectType,
          'proof_id': proofId,
          'state': state,
          ...details,
        };
      } catch (_) {
        // The transaction may still be pending in consensus.
      }
    }

    return null;
  }

  Future<List<ObjectInfo>> _getEscrowCandidateObjects(
    String buyerAddress,
  ) async {
    final byId = <String, ObjectInfo>{};

    void addAll(Iterable<ObjectInfo> objects) {
      for (final obj in objects) {
        if (obj.id.isNotEmpty) {
          byId[obj.id.toLowerCase()] = obj;
        }
      }
    }

    addAll(await rpc.getOwnedObjects(buyerAddress));

    addAll(await rpc.getObjects(owner: buyerAddress));

    return byId.values.toList();
  }

  bool _isEscrowDealType(String type) {
    return type.contains(
          '::${EscrowConstants.module}::${EscrowConstants.objectTypeDeal}<',
        ) ||
        type.contains('${EscrowConstants.objectTypeDeal}<');
  }

  bool _isEscrowProofType(String type) {
    return type.contains(
          '::${EscrowConstants.module}::${EscrowConstants.objectTypeProof}',
        ) ||
        type.contains(EscrowConstants.objectTypeProof);
  }

  String? _extractEscrowDealCoinType(String objectType) {
    if (!_isEscrowDealType(objectType)) return null;

    final start = objectType.indexOf('<');
    final end = objectType.lastIndexOf('>');
    if (start == -1 || end == -1 || end <= start) return null;

    final coinType = objectType.substring(start + 1, end).trim();
    return coinType.isEmpty ? null : coinType;
  }

  bool _sameAddress(String left, String right) {
    try {
      return BcsUtils.normalizeAddress(left) ==
          BcsUtils.normalizeAddress(right);
    } catch (_) {
      return left.toLowerCase() == right.toLowerCase();
    }
  }

  Future<String?> _dealLabelForObject(ObjectInfo object) async {
    final fullObject = object.data.isNotEmpty
        ? object
        : await rpc.getObject(object.id);
    return _decodeObjectDealLabel(fullObject);
  }

  bool _isObjectInputNotFoundError(Object error) {
    final text = error.toString().toLowerCase();
    return (text.contains('object input') && text.contains('was not found')) ||
        text.contains('object not found');
  }

  String? _decodeObjectDealLabel(ObjectInfo object) {
    final data = object.data;
    var offset = 0;

    List<int>? readBytes(int length) {
      if (length < 0 || offset + length > data.length) return null;
      final bytes = data.sublist(offset, offset + length);
      offset += length;
      return bytes;
    }

    int? readUleb128() {
      var value = 0;
      var shift = 0;
      while (offset < data.length) {
        final byte = data[offset++];
        value |= (byte & 0x7f) << shift;
        if ((byte & 0x80) == 0) return value;
        shift += 7;
        if (shift > 28) return null;
      }
      return null;
    }

    String? readString() {
      final length = readUleb128();
      if (length == null) return null;
      final bytes = readBytes(length);
      if (bytes == null) return null;
      return utf8.decode(bytes, allowMalformed: true);
    }

    final objectIdBytes = readBytes(32);
    if (objectIdBytes == null) return null;

    return readString();
  }

  dynamic _unwrapViewResult(dynamic value) {
    if (value is Map<String, dynamic> && value.containsKey('result')) {
      return value['result'];
    }
    return value;
  }

  int _jsonInt(dynamic value) {
    if (value is int) return value;
    if (value is num) return value.toInt();
    if (value is String) return int.tryParse(value) ?? 0;
    return 0;
  }

  String? _matchingProofId(
    List<ObjectChangeInfo> proofChanges,
    String buyerAddress,
  ) {
    for (final proofChange in proofChanges) {
      final owner = proofChange.owner;
      if (owner != null &&
          owner.isNotEmpty &&
          !_sameAddress(owner, buyerAddress)) {
        continue;
      }
      final objectId = proofChange.objectRef.objectId;
      if (objectId.isNotEmpty) return objectId;
    }
    return null;
  }

  Future<String?> _matchingProofObjectId(
    List<ObjectInfo> proofObjects,
    String? dealId,
  ) async {
    if (dealId == null || dealId.isEmpty) return null;

    for (final proofObject in proofObjects) {
      ObjectInfo fullProofObject;
      try {
        fullProofObject = proofObject.data.isNotEmpty
            ? proofObject
            : await rpc.getObject(proofObject.id);
      } catch (error) {
        if (_isObjectInputNotFoundError(error)) {
          continue;
        }
        rethrow;
      }
      final proofDealId = _decodeObjectDealLabel(fullProofObject);
      if (proofDealId == dealId) {
        return proofObject.id;
      }
    }

    return null;
  }

  /// Generic view function caller
  Future<List<dynamic>> _callViewFunction({
    required KanariWallet wallet,
    required String functionName,
    required String coinType,
    required TransactionArgs args,
    String? objectInputId,
  }) async {
    final normalizedToken = BcsUtils.normalizeTokenType(coinType);
    final packageAddr = BcsUtils.normalizeAddress(
      EscrowConstants.packageAddress,
    );
    final objectInputs = objectInputId == null
        ? const <Map<String, dynamic>>[]
        : [await _objectInputForView(objectInputId)];

    return rpc.viewFunction(
      sender: wallet.address,
      package: packageAddr,
      module: EscrowConstants.module,
      function: functionName,
      typeArgs: [normalizedToken],
      args: args.build(),
      objectInputs: objectInputs,
    );
  }

  Future<Map<String, dynamic>> _objectInputForView(String objectId) async {
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
      'mutable': false,
    };
  }

}
