// kanari_client.dart - Refactored version following escrow_client.dart pattern
import 'package:http/http.dart' as http;

import '../models/environment.dart';
import '../models/account.dart';
import '../models/block.dart';
import '../models/stats.dart';
import '../models/transaction.dart';
import '../models/module.dart';
import '../models/health.dart';
import '../kanari_wallet.dart';
import '../modules/modules.dart';

/// KanariClient - Facade that delegates to specialized modules
class KanariClient {
  final String url;
  final http.Client _client;

  late final QueriesModule _queries;
  late final TransactionOperations _transactions;

  KanariClient(this.url, {http.Client? client})
    : _client = client ?? http.Client() {
    _queries = QueriesModule(url, _client);
    _transactions = TransactionOperations(url, _client);
  }

  factory KanariClient.fromEnvironment(
    KanariEnvironment environment, {
    http.Client? client,
  }) {
    return KanariClient(environment.rpcUrl, client: client);
  }

  // ==================== QUERIES (Read Operations) ====================

  /// Get account information
  Future<AccountInfo> getAccount(String address) {
    return _queries.getAccount(address);
  }

  /// Get owner-centric state from the RPC server.
  Future<AccountInfo> getOwner(String address) {
    return _queries.getOwner(address);
  }

  /// Get token balance
  Future<TokenBalance> getTokenBalance(String address, String tokenType) {
    return _queries.getTokenBalance(address, tokenType);
  }

  /// Get a single on-chain object by id.
  Future<ObjectInfo> getObject(String objectId) {
    return _queries.getObject(objectId);
  }

  /// Query objects from the RPC object index.
  Future<List<ObjectInfo>> getObjects({
    String? owner,
    String? objectType,
  }) {
    return _queries.getObjects(owner: owner, objectType: objectType);
  }

  /// Get objects owned by one owner, optionally filtered by object type.
  Future<List<ObjectInfo>> getOwnedObjects(
    String owner, {
    String? objectType,
  }) {
    return _queries.getOwnedObjects(owner, objectType: objectType);
  }

  /// Get all token balances
  Future<List<TokenBalance>> getAllBalances(String address) {
    return _queries.getAllBalances(address);
  }

  /// List registered tokens and DeFi-aware supply accounting.
  Future<List<TokenInfo>> listTokens() {
    return _queries.listTokens();
  }

  /// Get Mysticeti checkpoint-backed view by height.
  Future<CheckpointInfo> getCheckpoint(int height) {
    return _queries.getCheckpoint(height);
  }

  /// Get checkpoint-backed block view by height.
  Future<BlockInfo> getBlock(int height) {
    return getCheckpoint(height);
  }

  /// Get current checkpoint height.
  Future<int> getCheckpointHeight() {
    return _queries.getCheckpointHeight();
  }

  /// Get current checkpoint height.
  Future<int> getBlockHeight() {
    return getCheckpointHeight();
  }

  /// Get transaction details
  Future<TransactionDetails> getTransaction(String hash) {
    return _queries.getTransaction(hash);
  }

  /// Get recent transactions, optionally filtered by account address.
  Future<List<TransactionDetails>> getAllTransactions({
    int limit = 50,
    String? account,
  }) {
    return _queries.getAllTransactions(limit: limit, account: account);
  }

  /// Get blockchain statistics
  Future<BlockchainStats> getStats() {
    return _queries.getStats();
  }

  /// Get health status
  Future<HealthStatus> getHealth() {
    return _queries.getHealth();
  }

  /// Get module information
  Future<ModuleInfo> getModule(String address, String name) {
    return _queries.getModule(address, name);
  }

  /// List all modules
  Future<List<ModuleInfo>> listModules() {
    return _queries.listModules();
  }

  /// Verify module bytecode
  Future<VerifyModuleResult> verifyModule(List<int> moduleBytes) {
    return _queries.verifyModule(moduleBytes);
  }

  Future<List<dynamic>> viewFunction({
    required String sender,
    required String package,
    required String module,
    required String function,
    List<String> typeArgs = const [],
    List<List<int>> args = const [],
    List<Map<String, dynamic>> objectInputs = const [],
  }) {
    return _queries.viewFunction(
      sender: sender,
      package: package,
      module: module,
      function: function,
      typeArgs: typeArgs,
      args: args,
      objectInputs: objectInputs,
    );
  }

  // ==================== TRANSACTIONS (Write Operations) ====================

  /// Publish a Move module to the blockchain
  Future<TransactionResult> publishModule({
    required KanariWallet wallet,
    required List<int> moduleBytes,
    required String moduleName,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
    bool? executeImmediate,
  }) {
    return _transactions.publishModule(
      wallet: wallet,
      moduleBytes: moduleBytes,
      moduleName: moduleName,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      executeImmediate: executeImmediate,
    );
  }

  /// Transfer KANARI tokens from one account to another
  Future<TransactionResult> transfer({
    required KanariWallet wallet,
    required String recipient,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
    List<String> excludedObjectIds = const [],
  }) {
    return _transactions.transfer(
      wallet: wallet,
      recipient: recipient,
      amount: amount,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      excludedObjectIds: excludedObjectIds,
    );
  }

  /// Execute a Move function
  Future<TransactionResult> executeFunction({
    required KanariWallet wallet,
    required String package,
    required String module,
    required String function,
    List<String> typeArgs = const [],
    List<List<int>> args = const [],
    List<Map<String, dynamic>> objectInputs = const [],
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
    bool? executeImmediate,
  }) {
    return _transactions.executeFunction(
      wallet: wallet,
      package: package,
      module: module,
      function: function,
      typeArgs: typeArgs,
      args: args,
      objectInputs: objectInputs,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
      executeImmediate: executeImmediate,
    );
  }

  /// Burn KANARI tokens (restricted to system/admin)
  Future<TransactionResult> burn({
    required KanariWallet wallet,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) {
    return _transactions.burn(
      wallet: wallet,
      amount: amount,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
    );
  }

  /// Transfer Custom Token
  Future<TransactionResult> transferToken({
    required KanariWallet wallet,
    required String recipient,
    required String tokenType,
    required int amount,
    int gasLimit = TransactionConstants.defaultGasLimit,
    int gasPrice = TransactionConstants.defaultGasPrice,
  }) {
    return _transactions.transferToken(
      wallet: wallet,
      recipient: recipient,
      tokenType: tokenType,
      amount: amount,
      gasLimit: gasLimit,
      gasPrice: gasPrice,
    );
  }

}
