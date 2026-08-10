import 'dart:async';

import 'package:flutter/material.dart';
import 'package:kanari_pay/kanari_pay.dart';

import '../core/bcs_utils.dart';
import '../core/token_utils.dart' as token_utils;

class WalletState extends ChangeNotifier {
  static const String kanariTokenType = token_utils.kanariTokenType;

  KanariClient? _client;
  KanariWallet? _wallet;
  List<Map<String, dynamic>> _wallets = [];
  final Map<String, Map<String, dynamic>> _decryptedWalletCache = {};
  List<TokenBalance> _tokenBalances = [];

  bool _isLoading = false;
  bool _isSubmittingTransaction = false;
  String? _error;
  String? _activeWalletId;
  String? _authenticatedWalletId;
  String? _sessionPin;
  KanariEnvironment _environment = KanariEnvironment.dev;
  bool _isUnlocked = false;

  KanariClient? get client => _client;
  KanariWallet? get wallet => _wallet;
  List<Map<String, dynamic>> get wallets => _wallets;
  List<TokenBalance> get tokenBalances => _tokenBalances;
  bool get isLoading => _isLoading;
  String? get error => _error;
  String? get activeWalletId => _activeWalletId;
  String? get authenticatedWalletId => _authenticatedWalletId;
  bool get hasWallet => _wallets.isNotEmpty;
  KanariEnvironment get environment => _environment;
  bool get isUnlocked => _isUnlocked;
  bool get requiresUnlock => hasWallet && !_isUnlocked;

  TokenBalance? get kanariTokenBalance {
    for (final token in _tokenBalances) {
      if (token_utils.isKanariToken(token)) {
        return token;
      }
    }
    return null;
  }

  int get kanariBalance => kanariTokenBalance?.amount ?? 0;

  Future<void> initialize() async {
    _updateClient();
    _wallets = await WalletStorage.loadAllWallets();
    notifyListeners();
  }

  Future<bool> syncWalletWithAddress(String walletAddress) async {
    final normalizedTarget = _normalizeWalletAddress(walletAddress);
    _wallets = await WalletStorage.loadAllWallets();
    _authenticatedWalletId = null;

    for (final walletData in _wallets) {
      try {
        final candidateAddress = walletData['address']?.toString();
        if (candidateAddress == null ||
            _normalizeWalletAddress(candidateAddress) != normalizedTarget) {
          continue;
        }

        await WalletStorage.setActiveWallet(walletData['id']);
        _activeWalletId = walletData['id'];
        _authenticatedWalletId = walletData['id'];
        if (_sessionPin != null) {
          await _instantiateWalletById(walletData['id'], pin: _sessionPin);
          _isUnlocked = true;
        }
        notifyListeners();
        return true;
      } catch (e) {
        debugPrint('Failed to compare wallet ${walletData['id']}: $e');
      }
    }

    notifyListeners();
    return false;
  }

  void _updateClient() {
    _client = KanariClient.fromEnvironment(_environment);
  }

  Future<void> setEnvironment(KanariEnvironment env) async {
    if (_environment == env) return;

    _environment = env;
    _updateClient();

    if (_wallet == null) {
      notifyListeners();
      return;
    }

    await _withLoading(() async {
      _clearBalances(notify: true);
      await refreshBalance(notifyListenersOnSuccess: false);
    });
  }

  Future<void> _loadActiveWallet({String? pin}) async {
    final activeId = await WalletStorage.getActiveWalletId();
    _activeWalletId = activeId;
    Map<String, dynamic>? activeWalletData;

    if (activeId != null) {
      activeWalletData = _wallets.cast<Map<String, dynamic>?>().firstWhere(
        (wallet) => wallet?['id'] == activeId,
        orElse: () => null,
      );
    }

    activeWalletData ??= _wallets.isNotEmpty ? _wallets.first : null;
    if (activeWalletData == null) {
      return;
    }

    if (pin != null) {
      await _instantiateWalletById(activeWalletData['id'], pin: pin);
    }
  }

  Future<KanariWallet> _walletFromData(Map<String, dynamic> data) async {
    final curve = KanariCurve.values.firstWhere(
      (item) => item.name == data['curve'],
      orElse: () => KanariCurve.ed25519,
    );

    if (data['mnemonic'] != null &&
        data['mnemonic'].toString().isNotEmpty &&
        !curve.isPostQuantum) {
      return KanariWallet.fromMnemonic(
        data['mnemonic'],
        curve: curve,
        derivationPath:
            data['derivationPath']?.toString() ??
            KanariWallet.defaultDerivationPath,
      );
    }

    return KanariWallet.fromPrivateKey(data['privateKey'], curve: curve);
  }

  Future<String?> walletAddressFromData(Map<String, dynamic> data) async {
    final savedAddress = data['address']?.toString();
    if (savedAddress != null && savedAddress.isNotEmpty) {
      return savedAddress;
    }

    try {
      final wallet = await _walletFromData(data);
      return wallet.address;
    } catch (e) {
      debugPrint('Failed to derive wallet address: $e');
      return null;
    }
  }

  Future<void> _instantiateWallet(Map<String, dynamic> data) async {
    _wallet = await _walletFromData(data);
    final walletAddress = _wallet?.address;
    _clearBalances();
    if (walletAddress != null) {
      _tokenBalances = await WalletStorage.loadCachedBalances(walletAddress);
    }
    notifyListeners();
    unawaited(refreshBalance(notifyListenersOnSuccess: true));
  }

  Future<void> _instantiateWalletById(String walletId, {String? pin}) async {
    final effectivePin = pin ?? _sessionPin;
    if (effectivePin == null) {
      throw StateError('Wallet is locked');
    }

    final cachedWallet = _decryptedWalletCache[walletId];
    final decryptedWallet =
        cachedWallet ??
        await WalletStorage.loadWalletById(walletId, pin: effectivePin);
    if (decryptedWallet == null) {
      throw StateError('Wallet not found');
    }

    _decryptedWalletCache[walletId] = Map<String, dynamic>.from(
      decryptedWallet,
    );

    await _instantiateWallet(decryptedWallet);
  }

  Future<void> switchWallet(String walletId) async {
    final walletData = _wallets.cast<Map<String, dynamic>?>().firstWhere(
      (wallet) => wallet?['id'] == walletId,
      orElse: () => null,
    );

    if (walletData == null) return;

    await WalletStorage.setActiveWallet(walletId);
    _activeWalletId = walletId;
    await _instantiateWalletById(walletData['id'] as String);
    notifyListeners();
  }

  Future<void> addWallet(Map<String, dynamic> walletData, [String? pin]) async {
    _wallets.add(walletData);

    final hasPassword = await WalletStorage.hasPassword();
    final hasValidPin = pin != null && RegExp(r'^\d{6}$').hasMatch(pin);
    final effectivePin = hasValidPin ? pin : _sessionPin;
    if (!hasPassword && !hasValidPin) {
      _wallets.removeWhere((wallet) => wallet['id'] == walletData['id']);
      throw StateError('PIN is required before saving a wallet');
    }

    if (!hasPassword) {
      await WalletStorage.savePassword(pin!);
      _sessionPin = pin;
    }

    if (effectivePin == null) {
      _wallets.removeWhere((wallet) => wallet['id'] == walletData['id']);
      throw StateError('Wallet is locked');
    }

    await WalletStorage.saveAllWallets(_wallets, pin: effectivePin);
    _wallets = await WalletStorage.loadAllWallets();
    _decryptedWalletCache[walletData['id'] as String] =
        Map<String, dynamic>.from(walletData);
    await switchWallet(walletData['id']);
    notifyListeners();
  }

  Future<void> removeWallet(String walletId) async {
    final removedWallet = _wallets.cast<Map<String, dynamic>?>().firstWhere(
      (wallet) => wallet?['id'] == walletId,
      orElse: () => null,
    );
    _wallets.removeWhere((wallet) => wallet['id'] == walletId);
    await WalletStorage.saveAllWallets(_wallets, pin: _sessionPin);
    _wallets = await WalletStorage.loadAllWallets();
    _decryptedWalletCache.remove(walletId);
    final removedAddress = removedWallet?['address']?.toString();
    if (removedAddress != null && removedAddress.isNotEmpty) {
      await WalletStorage.clearCachedBalances(removedAddress);
    }

    if (_authenticatedWalletId == walletId) {
      _authenticatedWalletId = null;
    }

    final activeId = await WalletStorage.getActiveWalletId();
    if (activeId == walletId && _wallets.isNotEmpty) {
      await switchWallet(_wallets.first['id']);
    } else if (_wallets.isEmpty) {
      _wallet = null;
      _clearBalances();
    }

    notifyListeners();
  }

  Future<void> createNewWallet({
    KanariCurve curve = KanariCurve.ed25519,
    required String pin,
    String derivationPath = KanariWallet.defaultDerivationPath,
  }) async {
    await _withLoading(() async {
      _error = null;
      try {
        final wallet = await KanariWallet.generate(
          curve: curve,
          derivationPath: derivationPath,
        );
        final walletData = {
          'id': DateTime.now().millisecondsSinceEpoch.toString(),
          'name': 'Wallet ${_wallets.length + 1}',
          'address': wallet.address,
          'mnemonic': wallet.mnemonic,
          'privateKey': wallet.privateKey,
          'curve': curve.name,
          'derivationPath': wallet.derivationPath,
          'createdAt': DateTime.now().toIso8601String(),
        };

        await addWallet(walletData, pin);
        _isUnlocked = true;
      } catch (e) {
        _error = 'Creation failed: $e';
      }
    });
  }

  Future<void> unlockWallet(String pin) async {
    await _withLoading(() async {
      _error = null;

      final isValid = await WalletStorage.verifyPassword(pin);
      if (!isValid) {
        _error = 'Invalid PIN';
        notifyListeners();
        return;
      }

      _sessionPin = pin;
      _wallets = await WalletStorage.loadAllWallets();
      await _loadActiveWallet(pin: pin);
      if (_wallets.isEmpty) {
        _error = 'No saved wallets';
        notifyListeners();
        return;
      }

      _isUnlocked = true;
      _error = null;
    });
  }

  Future<bool> verifyPin(String pin) async {
    if (pin.length != 6) return false;
    final success = await WalletStorage.verifyPassword(pin);
    if (success) {
      _sessionPin = pin;
    }
    return success;
  }

  Future<Duration?> pinLockRemaining() {
    return WalletStorage.pinLockRemaining();
  }

  Future<bool> hasPinSet() {
    return WalletStorage.hasPassword();
  }

  Future<bool> authorizeWithBiometricSession() async {
    return _sessionPin != null;
  }

  Future<bool> enableBiometricUnlock() async {
    final sessionPin = _sessionPin;
    if (sessionPin == null) return false;

    try {
      await WalletStorage.enableBiometricUnlock(sessionPin);
      notifyListeners();
      return true;
    } catch (e) {
      debugPrint('Failed to enable biometric unlock: $e');
      return false;
    }
  }

  Future<void> disableBiometricUnlock() async {
    await WalletStorage.disableBiometricUnlock();
    notifyListeners();
  }

  Future<bool> unlockWithBiometric() async {
    final pin = _sessionPin ?? await WalletStorage.readBiometricUnlockPin();
    if (pin == null) return false;

    await unlockWallet(pin);
    return _isUnlocked;
  }

  Future<void> importFromPrivateKey(
    String pk, {
    KanariCurve curve = KanariCurve.ed25519,
    String? pin,
  }) async {
    await _withLoading(() async {
      _error = null;
      try {
        final wallet = await KanariWallet.fromPrivateKey(
          pk.trim(),
          curve: curve,
        );
        final walletData = {
          'id': DateTime.now().millisecondsSinceEpoch.toString(),
          'name': 'Imported Wallet ${_wallets.length + 1}',
          'address': wallet.address,
          'mnemonic': '',
          'privateKey': wallet.privateKey,
          'curve': curve.name,
          'createdAt': DateTime.now().toIso8601String(),
        };

        await addWallet(walletData, pin);
        _isUnlocked = true;
      } catch (e) {
        _error = 'Import PK failed: $e';
      }
    });
  }

  Future<bool> upsertAuthenticatedWalletFromPrivateKey(
    String pk, {
    required KanariCurve curve,
    required String expectedAddress,
    String? pin,
    String? name,
  }) async {
    try {
      _error = null;
      final wallet = await KanariWallet.fromPrivateKey(pk.trim(), curve: curve);
      final normalizedWallet = _normalizeWalletAddress(wallet.address);
      final normalizedExpected = _normalizeWalletAddress(expectedAddress);
      if (normalizedWallet != normalizedExpected) {
        _error = 'Login wallet address does not match this account.';
        notifyListeners();
        return false;
      }

      final hasPassword = await WalletStorage.hasPassword();
      final hasValidPin = pin != null && RegExp(r'^\d{6}$').hasMatch(pin);
      final effectivePin = hasValidPin ? pin : _sessionPin;
      if (!hasPassword && !hasValidPin) {
        _error = 'PIN is required before saving this account wallet.';
        notifyListeners();
        return false;
      }

      if (!hasPassword) {
        await WalletStorage.savePassword(pin!);
        _sessionPin = pin;
      }

      if (effectivePin == null) {
        _error = 'Wallet is locked';
        notifyListeners();
        return false;
      }

      _wallets = await WalletStorage.loadAllWallets();
      final existingIndex = _wallets.indexWhere((walletData) {
        final address = walletData['address']?.toString();
        return address != null &&
            _normalizeWalletAddress(address) == normalizedExpected;
      });

      final walletId = existingIndex >= 0
          ? _wallets[existingIndex]['id'] as String
          : DateTime.now().millisecondsSinceEpoch.toString();
      final walletData = {
        'id': walletId,
        'name': name ?? 'Kanari Account Wallet',
        'address': wallet.address,
        'mnemonic': wallet.mnemonic,
        'privateKey': wallet.privateKey,
        'curve': curve.name,
        'derivationPath': wallet.derivationPath,
        'createdAt': existingIndex >= 0
            ? _wallets[existingIndex]['createdAt']
            : DateTime.now().toIso8601String(),
      };

      if (existingIndex >= 0) {
        _wallets[existingIndex] = walletData;
      } else {
        _wallets.add(walletData);
      }

      await WalletStorage.saveAllWallets(_wallets, pin: effectivePin);
      await WalletStorage.setActiveWallet(walletId);
      _wallets = await WalletStorage.loadAllWallets();
      _decryptedWalletCache[walletId] = Map<String, dynamic>.from(walletData);
      _activeWalletId = walletId;
      _authenticatedWalletId = walletId;
      _sessionPin = effectivePin;
      await _instantiateWallet(walletData);
      _isUnlocked = true;
      notifyListeners();
      return true;
    } catch (e) {
      _error = 'Account wallet import failed: $e';
      notifyListeners();
      return false;
    }
  }

  Future<void> importFromMnemonic(
    String mnemonic, {
    KanariCurve curve = KanariCurve.ed25519,
    String? pin,
    String derivationPath = KanariWallet.defaultDerivationPath,
  }) async {
    await _withLoading(() async {
      _error = null;
      try {
        final wallet = await KanariWallet.fromMnemonic(
          mnemonic,
          curve: curve,
          derivationPath: derivationPath,
        );
        final walletData = {
          'id': DateTime.now().millisecondsSinceEpoch.toString(),
          'name': 'Imported Wallet ${_wallets.length + 1}',
          'address': wallet.address,
          'mnemonic': mnemonic,
          'privateKey': wallet.privateKey,
          'curve': curve.name,
          'derivationPath': wallet.derivationPath,
          'createdAt': DateTime.now().toIso8601String(),
        };

        await addWallet(walletData, pin);
        _isUnlocked = true;
      } catch (e) {
        _error = 'Import Mnemonic failed: $e';
      }
    });
  }

  void logout() {
    _wallet = null;
    _sessionPin = null;
    _error = null;
    _activeWalletId = null;
    _authenticatedWalletId = null;
    _isUnlocked = false;
    _decryptedWalletCache.clear();
    _clearBalances();
    notifyListeners();
  }

  Future<void> removeAuthenticatedAccountWallet() async {
    final walletId = _authenticatedWalletId;
    _error = null;

    if (walletId == null) {
      notifyListeners();
      return;
    }

    await removeWallet(walletId);
  }

  Future<void> lockSession() async {
    _wallet = null;
    _sessionPin = null;
    _authenticatedWalletId = null;
    _isUnlocked = false;
    _decryptedWalletCache.clear();
    _clearBalances();
    _wallets = await WalletStorage.loadAllWallets();
    notifyListeners();
  }

  Future<void> deleteAllWallets() async {
    await WalletStorage.deleteAllWallets();
    _wallets = [];
    _wallet = null;
    _sessionPin = null;
    _error = null;
    _activeWalletId = null;
    _authenticatedWalletId = null;
    _isUnlocked = false;
    _decryptedWalletCache.clear();
    _clearBalances();
    notifyListeners();
  }

  Future<void> refreshBalance({bool notifyListenersOnSuccess = true}) async {
    if (_client == null || _wallet == null) {
      return;
    }

    final walletAddress = _wallet!.address;
    _setLoading(true);

    try {
      final balances = await _client!.getAllBalances(walletAddress);
      if (_wallet?.address != walletAddress) {
        return;
      }
      _tokenBalances = balances;
      await WalletStorage.saveCachedBalances(walletAddress, balances);
      _error = null;
    } catch (e) {
      if (_wallet?.address != walletAddress) {
        return;
      }
      _clearBalances();
      _error = 'Refresh balance failed: $e';
      debugPrint(_error);
    } finally {
      _setLoading(false);
    }

    if (notifyListenersOnSuccess) {
      notifyListeners();
    }
  }

  Future<String?> transfer(
    String recipient,
    int amount, {
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    if (_client == null || _wallet == null) return 'Client not initialized';
    return _runTransaction(() async {
      final result = await _client!.transfer(
        wallet: _wallet!,
        recipient: recipient,
        amount: amount,
        gasLimit: gasLimit,
        gasPrice: gasPrice,
      );
      return 'Success: Hash ${result.hash}';
    });
  }

  Future<String?> validateNativeTransferReadiness({
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    if (_client == null || _wallet == null) return 'Client not initialized';

    final requiredGas = gasLimit * gasPrice;
    final ownedObjects = await _client!.getOwnedObjects(_wallet!.address);
    final nativeCoins = ownedObjects
        .where((object) {
          final coinType = BcsUtils.extractCoinTypeFromObjectType(object.type);
          return coinType != null &&
              BcsUtils.tokenTypesEqual(coinType, kanariTokenType);
        })
        .map((object) => BcsUtils.readCoinObjectBalance(object.data) ?? 0)
        .where((balance) => balance > 0)
        .toList();

    if (nativeCoins.length < 2) {
      return 'Native KANARI transfer needs two separate Coin<KANARI> objects: one coin to send and another coin to pay gas. Receive/fund this wallet once more, then try again.';
    }

    for (var i = 0; i < nativeCoins.length; i++) {
      if (nativeCoins[i] < amount) continue;
      for (var j = 0; j < nativeCoins.length; j++) {
        if (i == j) continue;
        if (nativeCoins[j] >= requiredGas) return null;
      }
    }

    return 'Native KANARI transfer needs one Coin<KANARI> with enough send amount and a different Coin<KANARI> with enough gas. Try a smaller amount or fund another gas coin.';
  }

  Future<String?> executeFunction({
    required String packageAddress,
    required String module,
    required String function,
    List<String> typeArgs = const [],
    List<List<int>> args = const [],
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    if (_client == null || _wallet == null) return 'Client not initialized';
    return _runTransaction(() async {
      final result = await _client!.executeFunction(
        wallet: _wallet!,
        package: packageAddress,
        module: module,
        function: function,
        typeArgs: typeArgs,
        args: args,
        gasLimit: gasLimit,
        gasPrice: gasPrice,
      );
      return 'Success: Hash ${result.hash}';
    });
  }

  Future<String?> burn(
    int amount, {
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    if (_client == null || _wallet == null) return 'Client not initialized';
    return _runTransaction(() async {
      final result = await _client!.burn(
        wallet: _wallet!,
        amount: amount,
        gasLimit: gasLimit,
        gasPrice: gasPrice,
      );
      return 'Success: Hash ${result.hash}';
    });
  }

  Future<String?> transferToken(
    String recipient,
    String tokenType,
    int amount, {
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) async {
    if (_client == null || _wallet == null) return 'Client not initialized';
    return _runTransaction(() async {
      final result = await _client!.transferToken(
        wallet: _wallet!,
        recipient: recipient,
        tokenType: tokenType,
        amount: amount,
        gasLimit: gasLimit,
        gasPrice: gasPrice,
      );
      return 'Success: Hash ${result.hash}';
    });
  }

  Future<bool> changePin(String oldPin, String newPin) async {
    try {
      final isValid = await WalletStorage.verifyPassword(oldPin);
      if (!isValid) return false;

      final decryptedWallets = await WalletStorage.loadAllWallets(pin: oldPin);
      _sessionPin = newPin;
      await WalletStorage.savePasswordAndWallets(newPin, decryptedWallets);
      _wallets = await WalletStorage.loadAllWallets();
      notifyListeners();
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<bool> changePinWithSession(String newPin) async {
    final currentSessionPin = _sessionPin;
    if (currentSessionPin == null) return false;

    try {
      final decryptedWallets = await WalletStorage.loadAllWallets(
        pin: currentSessionPin,
      );
      _sessionPin = newPin;
      await WalletStorage.savePasswordAndWallets(newPin, decryptedWallets);
      _wallets = await WalletStorage.loadAllWallets();
      notifyListeners();
      return true;
    } catch (_) {
      _sessionPin = currentSessionPin;
      return false;
    }
  }

  Future<String?> _runTransaction(Future<String> Function() action) async {
    if (_isSubmittingTransaction) {
      return 'Error: Another transaction is already being submitted. Please wait for it to finish.';
    }

    _isSubmittingTransaction = true;
    try {
      return await _withLoading(() async {
        try {
          final message = await action();
          await refreshBalance(notifyListenersOnSuccess: false);
          return message;
        } catch (e) {
          return 'Error: ${_friendlyTransactionError(e)}';
        }
      });
    } finally {
      _isSubmittingTransaction = false;
    }
  }

  Future<T> _withLoading<T>(Future<T> Function() action) async {
    _setLoading(true);
    try {
      return await action();
    } finally {
      _setLoading(false);
    }
  }

  void _setLoading(bool value) {
    _isLoading = value;
    notifyListeners();
  }

  void _clearBalances({bool notify = false}) {
    _tokenBalances = [];
    if (notify) {
      notifyListeners();
    }
  }

  String _normalizeWalletAddress(String address) {
    final lower = address.trim().toLowerCase();
    return lower.startsWith('0x') ? lower.substring(2) : lower;
  }

  String _friendlyTransactionError(Object error) {
    final text = error.toString();
    if (text.contains('Native transfer requires two distinct Coin<') ||
        text.contains('native_transfer_policy_not_satisfied')) {
      return 'Native KANARI transfer needs two separate Coin<KANARI> objects: one coin to send and another coin to pay gas. Receive/fund this wallet once more, then try again.';
    }
    if (text.contains('No spendable native gas coin object found')) {
      return 'No separate spendable Coin<KANARI> gas object was found. Fund this wallet with another KANARI coin object, then try again.';
    }
    return text;
  }
}
