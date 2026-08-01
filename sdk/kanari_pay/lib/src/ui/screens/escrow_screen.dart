import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_spinkit/flutter_spinkit.dart';
import 'package:provider/provider.dart';

import '../../core/token_utils.dart' as token_utils;
import '../../client/escrow_client.dart';
import '../../models/transaction.dart';
import '../../providers/wallet_provider.dart';
import '../widgets/app_ui.dart';
import '../widgets/escrow_widgets.dart';

class _EscrowTokenOption {
  final String tokenType;
  final String label;
  final bool isSpendable;

  const _EscrowTokenOption({
    required this.tokenType,
    required this.label,
    required this.isSpendable,
  });
}

class EscrowScreen extends StatefulWidget {
  const EscrowScreen({super.key});

  @override
  State<EscrowScreen> createState() => _EscrowScreenState();
}

class _EscrowScreenState extends State<EscrowScreen>
    with SingleTickerProviderStateMixin {
  late final TabController _tabController;

  final _dealIdController = TextEditingController();
  final _sellerAddressController = TextEditingController();
  final _amountController = TextEditingController();
  final _descriptionController = TextEditingController();
  final _buyerAddressController = TextEditingController();
  final _disputeReasonController = TextEditingController();

  bool _isLoading = false;
  bool _isLoadingTokens = false;
  String? _loadedWalletAddress;
  String? _selectedTokenType;
  String? _errorMessage;
  String? _successMessage;
  List<String> _spendableCoinTypes = const [];
  int? _currentDealState;
  Map<String, dynamic>? _dealDetails;

  // สำหรับจัดการ multiple deals
  List<Map<String, dynamic>> _allDeals = [];
  String? _selectedDealId;
  Map<String, dynamic>? _selectedDeal;

  @override
  void initState() {
    super.initState();
    _tabController = TabController(length: 3, vsync: this);
    _dealIdController.text = _generateDealId();
  }

  @override
  void dispose() {
    _tabController.dispose();
    _dealIdController.dispose();
    _sellerAddressController.dispose();
    _amountController.dispose();
    _descriptionController.dispose();
    _buyerAddressController.dispose();
    _disputeReasonController.dispose();
    super.dispose();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final walletAddress = context.read<WalletState>().wallet?.address;
    if (walletAddress != null && walletAddress != _loadedWalletAddress) {
      _loadedWalletAddress = walletAddress;
      unawaited(_loadSpendableCoinTypes());
    }
  }

  EscrowClient? _escrowClient(WalletState state) {
    final client = state.client;
    if (client == null) return null;
    return EscrowClient(client);
  }

  Future<void> _loadSpendableCoinTypes() async {
    final walletState = context.read<WalletState>();
    final wallet = walletState.wallet;
    final escrow = _escrowClient(walletState);
    if (wallet == null || escrow == null) return;

    setState(() {
      _isLoadingTokens = true;
    });

    try {
      final coinTypes = await escrow.getSpendableCoinTypes(wallet.address);
      if (!mounted) return;
      setState(() {
        _spendableCoinTypes = coinTypes;
        _isLoadingTokens = false;
        if (_selectedTokenType == null ||
            !coinTypes.contains(_selectedTokenType)) {
          _selectedTokenType = coinTypes.isEmpty ? null : coinTypes.first;
        }
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _spendableCoinTypes = const [];
        _isLoadingTokens = false;
        _selectedTokenType = null;
      });
    }
  }

  List<_EscrowTokenOption> _buildTokenOptions(WalletState walletState) {
    final spendableTypes = _spendableCoinTypes.toSet();
    final optionsByType = <String, _EscrowTokenOption>{};

    for (final token in walletState.tokenBalances) {
      final isSpendable = spendableTypes.contains(token.tokenType);
      optionsByType[token.tokenType] = _EscrowTokenOption(
        tokenType: token.tokenType,
        label:
            '${token.symbol} (${token_utils.formatDisplayAmount(token.amount, token.decimals)})${isSpendable ? '' : ' - balance only'}',
        isSpendable: isSpendable,
      );
    }

    for (final tokenType in _spendableCoinTypes) {
      optionsByType.putIfAbsent(
        tokenType,
        () => _EscrowTokenOption(
          tokenType: tokenType,
          label: '$tokenType - spendable',
          isSpendable: true,
        ),
      );
    }

    final options = optionsByType.values.toList()
      ..sort((a, b) => a.label.compareTo(b.label));
    return options;
  }

  int _getDecimalsForTokenType(String tokenType, [WalletState? walletState]) {
    if (walletState != null) {
      for (final token in walletState.tokenBalances) {
        if (token.tokenType == tokenType) {
          return token.decimals;
        }
      }
    }
    return token_utils.defaultDecimalsForTokenType(tokenType);
  }

  String _toHumanAmount(int rawAmount, int decimals) {
    if (rawAmount == 0) return '0';
    return token_utils.formatDisplayAmount(
      rawAmount,
      decimals,
      fractionDigits: 6,
    );
  }

  String _friendlyError(Object error) {
    final text = error.toString();

    if (text.contains('SocketException') ||
        text.contains('connection was forcibly closed') ||
        text.contains('errno = 10054')) {
      return 'Kanari node connection lost. Please check if the node is running and try again.';
    }
    if (text.contains('Timeout') || text.contains('timed out')) {
      return 'Request timed out. Please try again.';
    }
    if (text.contains('Only transfer or burn transactions are supported') ||
        text.contains('not supported')) {
      return 'This node does not support escrow Move calls yet.';
    }
    if (text.contains('Escrow objects were not found')) {
      return 'No escrow objects found for this buyer address yet.';
    }
    if (text.contains('No spendable Coin<')) {
      return 'This wallet does not have a spendable coin object for the selected token.';
    }
    if (text.contains('KANARI can be used in DeFi')) {
      return 'KANARI can be used in DeFi, but Move execution needs two Coin<KANARI> objects: one for escrow funds and one separate gas coin.';
    }
    if (text.contains('Escrow module is not deployed') ||
        text.contains('Cannot find ModuleId') ||
        text.contains('not found in data cache')) {
      return 'Escrow contract is not deployed on this network. Publish the escrow package to DEV first.';
    }
    if (text.contains('No spendable native gas coin object found')) {
      return 'No separate KANARI gas coin object was found. Fund this wallet with a second Coin<KANARI> object, then try again.';
    }
    return 'An unexpected error occurred: ${text.split('\n').first}';
  }

  Future<void> _runAction(
    Future<void> Function(EscrowClient escrow, WalletState walletState) action,
  ) async {
    final walletState = context.read<WalletState>();
    final escrow = _escrowClient(walletState);
    if (walletState.wallet == null || escrow == null) {
      setState(() {
        _errorMessage = 'Wallet or RPC client is not available.';
      });
      return;
    }

    final authorized = await showAppPinVerificationSheet(
      context: context,
      onVerify: walletState.verifyPin,
      lockRemaining: walletState.pinLockRemaining,
      title: 'Confirm Escrow Action',
      subtitle: 'Enter your 6-digit PIN to authorize this escrow transaction.',
    );

    if (!mounted || !authorized) return;

    setState(() {
      _isLoading = true;
      _errorMessage = null;
      _successMessage = null;
    });

    try {
      await action(escrow, walletState);
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _errorMessage = _friendlyError(error);
      });
    } finally {
      if (mounted) {
        setState(() {
          _isLoading = false;
        });
      }
    }
  }

  Future<void> _createDeal() async {
    await _runAction((escrow, walletState) async {
      final wallet = walletState.wallet!;
      final tokenType = _selectedTokenType;
      final buyerAddress = wallet.address;
      final sellerAddress = _sellerAddressController.text.trim();

      if (_dealIdController.text.trim().isEmpty) {
        throw Exception('Deal ID is required');
      }
      if (sellerAddress.isEmpty) {
        throw Exception('Seller address is required');
      }
      if (_amountController.text.trim().isEmpty) {
        throw Exception('Amount is required');
      }
      if (tokenType == null || tokenType.isEmpty) {
        throw Exception('Escrow token is required');
      }

      // ตรวจสอบว่า buyer ≠ seller
      if (buyerAddress.toLowerCase() == sellerAddress.toLowerCase()) {
        throw Exception(
          'Buyer and seller cannot be the same address. '
          'Please enter a different seller address.',
        );
      }

      // CRITICAL: Validate and normalize seller address format
      // Remove '0x' prefix if present
      var cleanAddress = sellerAddress;
      if (cleanAddress.toLowerCase().startsWith('0x')) {
        cleanAddress = cleanAddress.substring(2);
      }

      // Validate hex characters
      if (!RegExp(r'^[0-9a-fA-F]+$').hasMatch(cleanAddress)) {
        throw Exception(
          'Invalid seller address format. Address must contain only hex characters (0-9, a-f).',
        );
      }

      // Validate address length
      // Kanari supports both short addresses (like 0x2) and full addresses (64 hex chars)
      if (cleanAddress.length > 64) {
        throw Exception(
          'Invalid seller address length. Address must be at most 64 hex characters.\n'
          'Current length: ${cleanAddress.length} characters',
        );
      }

      if (cleanAddress.isEmpty) {
        throw Exception('Seller address cannot be empty.');
      }

      // Convert human-readable amount to raw amount based on token decimals
      final decimals = _getDecimalsForTokenType(tokenType, walletState);
      final rawAmount = token_utils.baseUnitsFromDisplayString(
        _amountController.text.trim(),
        decimals,
      );

      if (rawAmount <= 0) {
        throw Exception('Invalid amount. Please enter a valid number.');
      }

      final result = await escrow
          .createDeal(
            wallet: wallet,
            dealId: _dealIdController.text.trim(),
            sellerAddress: sellerAddress,
            amount: rawAmount,
            description: _descriptionController.text.trim(),
            tokenType: tokenType,
          )
          .timeout(const Duration(seconds: 30));

      await walletState.refreshBalance();
      if (!mounted) return;
      setState(() {
        _successMessage =
            'Deal created successfully! Tx Hash: ${result.hash.substring(0, 16)}...';
        _buyerAddressController.text = buyerAddress;
      });
      await _fetchAllDeals(buyerAddress);
      final effects =
          result.effects ?? await _loadTransactionEffects(escrow, result.hash);

      if (_allDeals.isEmpty && effects != null) {
        final effectDeal = await escrow
            .getDealFromEffects(
              wallet: wallet,
              effects: effects,
              buyerAddress: buyerAddress,
            )
            .timeout(const Duration(seconds: 10));

        if (effectDeal != null && mounted) {
          setState(() {
            _allDeals = [effectDeal];
            _selectedDealId = effectDeal['deal_id'] as String?;
            _selectedDeal = effectDeal;
            _currentDealState = effectDeal['state'] as int?;
            _dealDetails = {
              'deal_id': effectDeal['deal_id'],
              'buyer': effectDeal['buyer'],
              'seller': effectDeal['seller'],
              'amount': effectDeal['amount'],
              'coin_type': effectDeal['coin_type'],
            };
            _errorMessage = null;
          });
        }
      }
      _dealIdController.clear();
      _sellerAddressController.clear();
      _amountController.clear();
      _descriptionController.clear();
    });
  }

  Future<TransactionEffectsInfo?> _loadTransactionEffects(
    EscrowClient escrow,
    String hash,
  ) async {
    try {
      final tx = await escrow.rpc.getTransaction(hash);
      return tx.effects;
    } catch (_) {
      return null;
    }
  }

  /// Seller: Confirm delivery
  Future<void> _confirmDelivery() async {
    if (_selectedDeal == null) {
      setState(() {
        _errorMessage = 'Please select a deal first';
      });
      return;
    }

    final objectId = _selectedDeal!['object_id'] as String?;
    final coinType = _selectedDeal!['coin_type'] as String?;
    final proofId = _selectedDeal!['proof_id'] as String?;

    if (objectId == null || coinType == null) {
      setState(() {
        _errorMessage = 'Missing deal information';
      });
      return;
    }

    if (proofId == null) {
      setState(() {
        _errorMessage = 'Proof object not found for this deal';
      });
      return;
    }
    await _runAction((escrow, walletState) async {
      final result = await escrow
          .confirmDelivery(
            wallet: walletState.wallet!,
            dealObjectId: objectId,
            coinType: coinType,
            proofObjectId: proofId,
          )
          .timeout(const Duration(seconds: 30));

      // Check if transaction failed
      if (result.status == 'failed') {
        throw Exception(
          result.errorMessage ??
              'Transaction failed on-chain. Check Move VM logs or use Kanari CLI for detailed error.',
        );
      }

      // Refresh deals list after successful transaction
      await _fetchAllDeals(walletState.wallet!.address);
    });
  }

  /// Buyer: Release funds
  Future<void> _releaseFunds() async {
    if (_selectedDeal == null) {
      setState(() {
        _errorMessage = 'Please select a deal first';
      });
      return;
    }

    final objectId = _selectedDeal!['object_id'] as String?;
    final coinType = _selectedDeal!['coin_type'] as String?;
    final proofId = _selectedDeal!['proof_id'] as String?;

    if (objectId == null || coinType == null) {
      setState(() {
        _errorMessage = 'Missing deal information';
      });
      return;
    }

    if (proofId == null) {
      setState(() {
        _errorMessage =
            'Proof object not found for this deal. Cannot release funds.';
      });
      return;
    }
    await _runAction((escrow, walletState) async {
      final result = await escrow
          .releaseFunds(
            wallet: walletState.wallet!,
            dealObjectId: objectId,
            coinType: coinType,
            proofObjectId: proofId,
          )
          .timeout(const Duration(seconds: 30));

      // Check if transaction failed
      if (result.status == 'failed') {
        throw Exception(
          result.errorMessage ??
              'Transaction failed on-chain. Check Move VM logs or use Kanari CLI for detailed error.',
        );
      }

      // Refresh deals list after successful transaction
      await _fetchAllDeals(walletState.wallet!.address);
    });
  }

  /// Buyer or Seller: Raise dispute
  Future<void> _raiseDispute() async {
    if (_selectedDeal == null) {
      setState(() {
        _errorMessage = 'Please select a deal first';
      });
      return;
    }

    final objectId = _selectedDeal!['object_id'] as String?;
    final coinType = _selectedDeal!['coin_type'] as String?;
    final proofId = _selectedDeal!['proof_id'] as String?;

    if (objectId == null || coinType == null) {
      setState(() {
        _errorMessage = 'Missing deal information';
      });
      return;
    }

    if (proofId == null) {
      setState(() {
        _errorMessage =
            'Proof object not found for this deal. Cannot raise dispute.';
      });
      return;
    }
    await _runAction((escrow, walletState) async {
      final result = await escrow
          .raiseDispute(
            wallet: walletState.wallet!,
            dealObjectId: objectId,
            coinType: coinType,
            proofObjectId: proofId,
            reason: _disputeReasonController.text.trim().isEmpty
                ? 'No reason provided'
                : _disputeReasonController.text.trim(),
          )
          .timeout(const Duration(seconds: 30));

      // Check if transaction failed
      if (result.status == 'failed') {
        throw Exception(
          result.errorMessage ??
              'Transaction failed on-chain. Check Move VM logs or use Kanari CLI for detailed error.',
        );
      }

      // Refresh deals list after successful transaction
      await _fetchAllDeals(walletState.wallet!.address);
    });
  }

  Future<void> _checkDealState() async {
    await _runAction((escrow, walletState) async {
      final wallet = walletState.wallet!;
      final buyer = _buyerAddressController.text.trim();
      if (buyer.isEmpty) {
        throw Exception('Buyer address is required');
      }

      //  FIXED: Use selected deal if available, otherwise fetch latest
      if (_selectedDeal != null) {
        // User selected a specific deal from dropdown
        final objectId = _selectedDeal!['object_id'] as String?;
        final coinType = _selectedDeal!['coin_type'] as String?;
        final dealId = _selectedDeal!['deal_id'] as String?;

        if (objectId == null || coinType == null || dealId == null) {
          throw Exception('Invalid selected deal data');
        }

        final state = await escrow
            .getDealStateByObjectId(
              wallet: wallet,
              dealObjectId: objectId,
              coinType: coinType,
            )
            .timeout(const Duration(seconds: 30));

        // Parse details from selected deal
        final rawAmount = _selectedDeal!['amount'] as int? ?? 0;
        final buyerAddr = _selectedDeal!['buyer'] as String? ?? buyer;
        final sellerAddr = _selectedDeal!['seller'] as String? ?? 'N/A';

        final details = {
          'deal_id': dealId,
          'buyer': buyerAddr,
          'seller': sellerAddr,
          'amount': rawAmount,
          'coin_type': coinType,
        };

        if (!mounted) return;
        setState(() {
          _currentDealState = state;
          _dealDetails = details;
          _successMessage = 'Deal state retrieved successfully!';
        });
      } else {
        // Fallback: Get latest deal for buyer address
        final deals = await escrow
            .getAllDeals(wallet: wallet, buyerAddress: buyer)
            .timeout(const Duration(seconds: 30));

        if (!mounted) return;
        if (deals.isEmpty) {
          setState(() {
            _currentDealState = null;
            _dealDetails = null;
            _errorMessage = 'No deals found for this buyer address.';
          });
          return;
        }

        // Get first deal
        final firstDeal = deals.first;
        final objectId = firstDeal['object_id'] as String?;
        final coinType = firstDeal['coin_type'] as String?;
        final dealId = firstDeal['deal_id'] as String?;

        if (objectId == null || coinType == null) {
          throw Exception('Invalid deal data');
        }

        final state = await escrow
            .getDealStateByObjectId(
              wallet: wallet,
              dealObjectId: objectId,
              coinType: coinType,
            )
            .timeout(const Duration(seconds: 10));

        final details = {
          'deal_id': dealId,
          'buyer': firstDeal['buyer'] ?? buyer,
          'seller': firstDeal['seller'] ?? 'N/A',
          'amount': firstDeal['amount'] ?? 0,
          'coin_type': coinType,
        };

        if (!mounted) return;
        setState(() {
          _currentDealState = state;
          _dealDetails = details;
          _successMessage = 'Deal state retrieved successfully!';
        });
      }
    });
  }

  /// Auto-check state when buyer address changes
  Future<void> _autoCheckDealState() async {
    final buyer = _buyerAddressController.text.trim();
    if (buyer.isEmpty) {
      setState(() {
        _currentDealState = null;
        _dealDetails = null;
        _allDeals = [];
        _selectedDealId = null;
        _selectedDeal = null;
      });
      return;
    }

    await _fetchAllDeals(buyer);
  }

  /// Fetch all deals for a buyer address
  Future<void> _fetchAllDeals(String buyerAddress) async {
    final walletState = context.read<WalletState>();
    final escrow = _escrowClient(walletState);
    if (walletState.wallet == null || escrow == null) return;

    try {
      final deals = await escrow
          .getAllDeals(wallet: walletState.wallet!, buyerAddress: buyerAddress)
          .timeout(const Duration(seconds: 15));

      if (!mounted) return;
      setState(() {
        _allDeals = deals;
        _selectedDealId = deals.isNotEmpty ? deals.first['deal_id'] : null;
        _selectedDeal = deals.isNotEmpty ? deals.first : null;
        _currentDealState = null;
        _dealDetails = null;
      });

      if (deals.isNotEmpty) {
        await _loadSelectedDeal();
      }
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _allDeals = [];
        _selectedDealId = null;
        _selectedDeal = null;
        _currentDealState = null;
        _dealDetails = null;
        _errorMessage = 'Failed to fetch deals: $e';
      });
    }
  }

  /// Load details for selected deal
  Future<void> _loadSelectedDeal() async {
    if (_selectedDeal == null) return;

    final walletState = context.read<WalletState>();
    final escrow = _escrowClient(walletState);
    if (walletState.wallet == null || escrow == null) return;

    try {
      final objectId = _selectedDeal!['object_id'] as String?;
      final coinType = _selectedDeal!['coin_type'] as String?;

      if (objectId == null || coinType == null) {
        throw Exception('Missing object_id or coin_type in deal data');
      }

      final existingState = _selectedDeal!['state'];
      final state = existingState is num
          ? existingState.toInt()
          : await escrow
                .getDealStateByObjectId(
                  wallet: walletState.wallet!,
                  dealObjectId: objectId,
                  coinType: coinType,
                )
                .timeout(const Duration(seconds: 10));

      // Extract details from selectedDeal (already have from getAllDeals)
      final dealId = _selectedDeal!['deal_id'] as String? ?? objectId;
      final buyer = _selectedDeal!['buyer'] as String? ?? 'N/A';
      final seller = _selectedDeal!['seller'] as String? ?? 'N/A';
      final amount = _selectedDeal!['amount'] as int? ?? 0;

      final details = {
        'deal_id': dealId,
        'buyer': buyer,
        'seller': seller,
        'amount': amount,
        'coin_type': coinType,
      };

      if (!mounted) return;
      setState(() {
        _currentDealState = state;
        _dealDetails = details;
        _errorMessage = null;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _currentDealState = null;
        _dealDetails = null;
        _errorMessage = 'Failed to load deal: $e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return AppGradientScaffold(
      backgroundColor: colorScheme.surface,
      body: _isLoadingTokens
          ? Center(child: SpinKitFadingCircle(color: colorScheme.primary))
          : RefreshIndicator(
              onRefresh: _loadSpendableCoinTypes,
              backgroundColor: colorScheme.surface,
              color: colorScheme.primary,
              child: AppTabPageSection(
                controller: _tabController,
                tabBarMargin: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 8,
                ),
                tabs: const [
                  Row(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Icon(Icons.add_business_rounded, size: 18),
                      SizedBox(width: 6),
                      Text('Create'),
                    ],
                  ),
                  Row(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Icon(Icons.list_rounded, size: 18),
                      SizedBox(width: 6),
                      Text('Deals'),
                    ],
                  ),
                  Row(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Icon(Icons.history_rounded, size: 18),
                      SizedBox(width: 6),
                      Text('History'),
                    ],
                  ),
                ],
                children: [
                  _buildCreateDealTab(),
                  _buildManageDealsTab(),
                  _buildHistoryTab(),
                ],
              ),
            ),
    );
  }

  Widget _buildCreateDealTab() {
    final walletState = context.watch<WalletState>();
    final colorScheme = Theme.of(context).colorScheme;

    final allOptions = _buildTokenOptions(walletState);
    final spendableOptions = allOptions
        .where((option) => option.isSpendable)
        .toList();
    final balanceOnlyOptions = allOptions
        .where((option) => !option.isSpendable)
        .toList();

    if (_selectedTokenType != null &&
        !spendableOptions.any(
          (option) => option.tokenType == _selectedTokenType,
        )) {
      _selectedTokenType = spendableOptions.isEmpty
          ? null
          : spendableOptions.first.tokenType;
    }

    return SingleChildScrollView(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _buildBanner(
            title: 'Create New Escrow Deal',
            subtitle:
                'Create a new deal and lock funds using a spendable coin object.',
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (_isLoadingTokens)
                  const Padding(
                    padding: EdgeInsets.only(top: 12),
                    child: LinearProgressIndicator(),
                  )
                else if (spendableOptions.isEmpty)
                  Padding(
                    padding: const EdgeInsets.only(top: 12),
                    child: const AppStatusBanner(
                      message:
                          'No spendable escrow token was found in this wallet yet.',
                      tone: AppStatusTone.warning,
                    ),
                  )
                else
                  Padding(
                    padding: const EdgeInsets.only(top: 12),
                    child: AppDropdownField<String>(
                      initialValue: _selectedTokenType,
                      label: 'Escrow Token',
                      prefixIcon: Icons.account_balance_wallet_outlined,
                      isExpanded:
                          true, // ← เพิ่มเพื่อให้ dropdown ใช้พื้นที่เต็มที่
                      items: spendableOptions
                          .map(
                            (option) => DropdownMenuItem<String>(
                              value: option.tokenType,
                              child: Text(
                                option.label,
                                overflow:
                                    TextOverflow.ellipsis, // ← เพิ่ม ellipsis
                              ),
                            ),
                          )
                          .toList(),
                      onChanged: _isLoading
                          ? null
                          : (value) {
                              setState(() {
                                _selectedTokenType = value;
                              });
                            },
                    ),
                  ),
                if (balanceOnlyOptions.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(top: 12),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'Balances found but not selectable for escrow:',
                          style: Theme.of(context).textTheme.bodyMedium
                              ?.copyWith(
                                color: colorScheme.onSurfaceVariant,
                                fontWeight: FontWeight.w600,
                              ),
                        ),
                        const SizedBox(height: 8),
                        ...balanceOnlyOptions.map(
                          (option) => Text('- ${option.label}'),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          _buildFeedback(colorScheme),

          // ใช้ reusable widgets
          _buildDealIdInput(),

          const SizedBox(height: 12),

          _buildAddressInput(
            controller: _sellerAddressController,
            label: 'Seller Address',
            hintText: '0x...',
            prefixIcon: Icons.person,
            onAutofill: _autofillSellerAddress,
            helperText: 'Address of the seller',
          ),
          const SizedBox(height: 12),
          AppTextInput(
            controller: _amountController,
            label: 'Amount',
            hintText: 'Enter amount in smallest unit',
            prefixIcon: Icons.payments,
            keyboardType: TextInputType.number,
          ),
          const SizedBox(height: 12),
          AppTextInput(
            controller: _descriptionController,
            label: 'Description',
            hintText: 'Describe the deal',
            prefixIcon: Icons.description,
            maxLines: 3,
          ),
          const SizedBox(height: 24),
          // ใช้ AppWideButton แทน ElevatedButton
          _buildPrimaryButton(
            onPressed: _createDeal,
            icon: Icons.lock_outline,
            label: 'Create Deal & Lock Funds',
            isLoading: _isLoading || spendableOptions.isEmpty,
          ),
        ],
      ),
    );
  }

  Widget _buildManageDealsTab() {
    final colorScheme = Theme.of(context).colorScheme;

    return SingleChildScrollView(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _buildBanner(
            title: 'Deal Actions',
            subtitle:
                'Use the buyer address to operate on the latest escrow deal.',
          ),
          const SizedBox(height: 16),
          _buildFeedback(colorScheme),

          // แสดง current deal state ถ้ามี
          if (_currentDealState != null) ...[
            AppAccountSummaryPanel(
              title: 'Current State',
              subtitle: 'Live state for the selected deal',
              trailing: _buildStateBadge(_currentDealState!),
            ),
            const SizedBox(height: 16),
          ],

          // เพิ่ม dropdown สำหรับเลือก Deal ID ถ้ามีหลาย deal
          if (_allDeals.length > 1) ...[
            AppSectionTitle('Select Deal'),
            const SizedBox(height: 8),
            AppDropdownField<String>(
              initialValue: _selectedDealId,
              label: 'Deal ID',
              isExpanded: true, // ← เพิ่มเพื่อใช้พื้นที่เต็มที่
              items: _allDeals.map((deal) {
                final dealId = deal['deal_id'] as String? ?? 'Unknown';
                final rawAmount = deal['amount'] as int? ?? 0;
                final coinType = deal['coin_type'] as String? ?? '';
                final coinName = coinType.split('::').lastOrNull ?? '';

                // Convert raw amount to human-readable
                final decimals = _getDecimalsForTokenType(coinType);
                final humanAmount = _toHumanAmount(rawAmount, decimals);

                // แสดงเฉพาะส่วนสั้นๆ ของ deal ID
                final shortDealId = dealId.length > 16
                    ? '${dealId.substring(0, 8)}...${dealId.substring(dealId.length - 6)}'
                    : dealId;

                return DropdownMenuItem<String>(
                  value: dealId,
                  child: Text(
                    '$shortDealId • $humanAmount $coinName',
                    overflow: TextOverflow.ellipsis, // ← เพิ่ม ellipsis
                  ),
                );
              }).toList(),
              onChanged: (value) {
                if (value != null) {
                  setState(() {
                    _selectedDealId = value;
                    _selectedDeal = _allDeals.firstWhere(
                      (deal) => deal['deal_id'] == value,
                      orElse: () => {},
                    );
                  });
                  _loadSelectedDeal();
                }
              },
            ),
            const SizedBox(height: 16),
          ],

          // ใช้ reusable widget สำหรับ buyer address
          _buildAddressInput(
            controller: _buyerAddressController,
            label: 'Buyer Address',
            hintText: '0x...',
            prefixIcon: Icons.account_balance_wallet,
            onAutofill: () {
              _autofillBuyerAddress();
              _autoCheckDealState();
            },
            helperText: 'Address of the buyer who created the deal',
            onChanged: _autoCheckDealState,
          ),
          if (_buyerAddressController.text.trim().isNotEmpty &&
              _allDeals.isEmpty &&
              _currentDealState == null &&
              _errorMessage == null) ...[
            const SizedBox(height: 12),
            AppStatusBanner(
              message:
                  'No escrow deal objects found for this buyer on ${context.read<WalletState>().environment.name.toUpperCase()}. Check the buyer address, network, or create a deal first.',
              tone: AppStatusTone.info,
              icon: Icons.search_off_rounded,
            ),
          ],
          const SizedBox(height: 24),
          AppSectionTitle('Seller Actions'),
          const SizedBox(height: 8),
          _buildOutlinedButton(
            onPressed: _currentDealState == 1 ? _confirmDelivery : null,
            icon: Icons.local_shipping,
            label: _currentDealState == 1
                ? 'Confirm Delivery'
                : 'Deal is not in LOCKED state',
            color: colorScheme.primary,
          ),
          const SizedBox(height: 24),
          AppSectionTitle('Buyer Actions'),
          const SizedBox(height: 8),
          _buildPrimaryButton(
            onPressed: _currentDealState == 2 ? _releaseFunds : null,
            icon: Icons.payment,
            label: _currentDealState == 2
                ? 'Release Funds'
                : 'Deal is not in DELIVERED state',
            isLoading: false,
          ),
          const SizedBox(height: 12),
          // เพิ่ม input field สำหรับเหตุผล dispute
          AppTextInput(
            controller: _disputeReasonController,
            label: 'Dispute Reason',
            hintText: 'Explain why you are raising a dispute',
            prefixIcon: Icons.gavel,
            maxLines: 2,
          ),
          const SizedBox(height: 12),
          // Warning: Dispute will refund immediately
          const AppStatusBanner(
            message:
                'Dispute will refund funds to buyer immediately. Cannot be reversed.',
            tone: AppStatusTone.warning,
          ),
          const SizedBox(height: 12),
          _buildOutlinedButton(
            onPressed: (_currentDealState == 1 || _currentDealState == 2)
                ? _raiseDispute
                : null,
            icon: Icons.gavel,
            label: _getDisputeButtonLabel(),
            color: colorScheme.error,
          ),
        ],
      ),
    );
  }

  Widget _buildHistoryTab() {
    final colorScheme = Theme.of(context).colorScheme;

    return SingleChildScrollView(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _buildBanner(
            title: 'Check Deal Status',
            subtitle: 'Inspect the latest deal owned by a buyer address.',
          ),
          const SizedBox(height: 16),
          _buildFeedback(colorScheme),
          _buildAddressInput(
            controller: _buyerAddressController,
            label: 'Buyer Address',
            hintText: '0x...',
            prefixIcon: Icons.search,
            onAutofill: () {
              _autofillBuyerAddress();
              _autoCheckDealState();
            },
            helperText: 'Address to check deal status',
            onChanged: _autoCheckDealState,
          ),
          const SizedBox(height: 16),
          _buildPrimaryButton(
            onPressed: _checkDealState,
            icon: Icons.search,
            label: 'Check Status',
          ),
          const SizedBox(height: 24),

          // 🔥 NEW: แสดง Deal List ทั้งหมดเป็น card
          if (_allDeals.isNotEmpty) ...[
            AppSectionTitle('All Deals (${_allDeals.length})'),
            const SizedBox(height: 12),
            ListView.builder(
              shrinkWrap: true,
              physics: const NeverScrollableScrollPhysics(),
              itemCount: _allDeals.length,
              itemBuilder: (context, index) {
                final deal = _allDeals[index];
                final dealId = deal['deal_id'] as String? ?? 'N/A';
                final isSelected = _selectedDealId == dealId;

                return DealCard(
                  deal: deal,
                  isSelected: isSelected,
                  colorScheme: colorScheme,
                  onTap: () {
                    setState(() {
                      _selectedDealId = dealId;
                      _selectedDeal = deal;
                    });
                    _checkDealState();
                  },
                );
              },
            ),
            const SizedBox(height: 24),
          ],

          // แสดงสถานะ deal
          if (_currentDealState != null)
            AppPanel(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      AppSectionTitle('Current State'),
                      _buildStateBadge(_currentDealState!),
                    ],
                  ),
                  if (_dealDetails != null && _dealDetails!.isNotEmpty) ...[
                    const Divider(height: 24),
                    _buildDetailRow(
                      'Deal ID',
                      _dealDetails!['deal_id'] ?? 'N/A',
                    ),
                    const SizedBox(height: 8),
                    _buildDetailRow('Buyer', _dealDetails!['buyer'] ?? 'N/A'),
                    const SizedBox(height: 8),
                    _buildDetailRow('Seller', _dealDetails!['seller'] ?? 'N/A'),
                    const SizedBox(height: 8),
                    _buildDetailRow(
                      'Amount',
                      _dealDetails!['amount'] != null
                          ? '${_toHumanAmount(_dealDetails!['amount'] as int, _getDecimalsForTokenType(_dealDetails!['coin_type'] ?? ''))} ${(_dealDetails!['coin_type'] as String? ?? '').split('::').lastOrNull ?? 'units'}'
                          : 'N/A',
                    ),
                  ],
                ],
              ),
            ),
          const SizedBox(height: 16),
        ],
      ),
    );
  }

  Widget _buildBanner({
    required String title,
    required String subtitle,
    Widget? child,
  }) {
    return AppFormSection(
      title: title,
      subtitle: subtitle,
      padding: const EdgeInsets.all(12),
      spacing: AppUiTokens.contentSpacing,
      child: child,
    );
  }

  Widget _buildFeedback(ColorScheme colorScheme) {
    if (_errorMessage == null && _successMessage == null) {
      return const SizedBox.shrink();
    }

    final isError = _errorMessage != null;
    final message = isError ? _errorMessage! : _successMessage!;

    return Column(
      children: [
        if (isError)
          AppErrorBanner(message: message)
        else
          AppStatusBanner(
            message: message,
            tone: AppStatusTone.success,
            onDismiss: () {
              setState(() {
                _errorMessage = null;
                _successMessage = null;
              });
            },
          ),
      ],
    );
  }

  Widget _buildPrimaryButton({
    required VoidCallback? onPressed,
    required IconData icon,
    required String label,
    bool isLoading = false,
  }) {
    return AppWideButton(
      onPressed: (isLoading || onPressed == null) ? null : onPressed,
      icon: icon,
      label: isLoading ? 'Processing...' : label,
      style: AppWideButtonStyle.primary,
    );
  }

  Widget _buildOutlinedButton({
    required VoidCallback? onPressed,
    required IconData icon,
    required String label,
    Color? color,
    bool isLoading = false,
  }) {
    return AppWideButton(
      onPressed: (isLoading || onPressed == null) ? null : onPressed,
      icon: icon,
      label: isLoading ? 'Processing...' : label,
      style: AppWideButtonStyle.outlined,
    );
  }

  /// สร้าง Deal ID อัตโนมัติในรูปแบบ deal-{timestamp}-{random}
  String _generateDealId() {
    final timestamp = DateTime.now().millisecondsSinceEpoch;
    final random = DateTime.now().microsecondsSinceEpoch % 10000;
    return 'deal-$timestamp-${random.toString().padLeft(4, '0')}';
  }

  /// ดึง wallet address ปัจจุบัน
  String? _getCurrentWalletAddress() {
    try {
      final walletState = context.read<WalletState>();
      return walletState.wallet?.address;
    } catch (_) {
      return null;
    }
  }

  /// เติม Seller Address ด้วย wallet address ปัจจุบัน
  void _autofillSellerAddress() {
    final address = _getCurrentWalletAddress();
    if (address != null) {
      setState(() {
        _sellerAddressController.text = address;
      });
    }
  }

  /// เติม Buyer Address ด้วย wallet address ปัจจุบัน
  void _autofillBuyerAddress() {
    final address = _getCurrentWalletAddress();
    if (address != null) {
      setState(() {
        _buyerAddressController.text = address;
      });
    }
  }

  /// Get button label based on deal state
  String _getDisputeButtonLabel() {
    switch (_currentDealState) {
      case 1: // STATE_LOCKED
        return 'Raise Dispute & Refund Buyer';
      case 2: // STATE_DELIVERED
        return 'Raise Dispute & Refund Buyer';
      case 3: // STATE_COMPLETED
        return 'Deal is already completed';
      case 4: // STATE_DISPUTED
        return 'Dispute already raised';
      default:
        return 'Deal is not found';
    }
  }

  /// รีเฟรช Deal ID ใหม่
  void _refreshDealId() {
    setState(() {
      _dealIdController.text = _generateDealId();
    });
  }

  /// Widget สำหรับ Address input พร้อมปุ่ม autofill
  Widget _buildAddressInput({
    required TextEditingController controller,
    required String label,
    required String hintText,
    required IconData prefixIcon,
    required VoidCallback onAutofill,
    String? helperText,
    VoidCallback? onChanged,
  }) {
    return AppActionTextField(
      controller: controller,
      label: label,
      hintText: hintText,
      prefixIcon: prefixIcon,
      helperText: helperText,
      enabled: !_isLoading,
      onChanged: onChanged == null ? null : (_) => onChanged(),
      onAction: onAutofill,
      actionIcon: Icons.person_pin,
      actionTooltip: 'Use my wallet address',
    );
  }

  /// Widget สำหรับ Deal ID input พร้อมปุ่ม refresh
  Widget _buildDealIdInput() {
    return AppActionTextField(
      controller: _dealIdController,
      label: 'Deal ID',
      hintText: 'Auto-generated',
      prefixIcon: Icons.tag,
      helperText: 'Auto-generated, but you can edit it',
      enabled: !_isLoading,
      onAction: _refreshDealId,
      actionIcon: Icons.refresh_rounded,
      actionTooltip: 'Generate new Deal ID',
    );
  }

  /// Widget สำหรับแสดง state badge
  Widget _buildStateBadge(int state) => StateBadge(state: state);

  /// Widget สำหรับ detail row
  Widget _buildDetailRow(String label, String value) =>
      AppDetailRow(label: label, value: value);
}
