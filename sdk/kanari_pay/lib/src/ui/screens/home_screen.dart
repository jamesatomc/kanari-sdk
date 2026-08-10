import 'dart:async';
import 'dart:ui';
import 'package:flutter/material.dart';
import 'package:flutter_spinkit/flutter_spinkit.dart';
import 'package:kanari_pay/src/ui/widgets/token_logo.dart';
import 'package:provider/provider.dart';

import 'package:kanari_pay/src/providers/wallet_provider.dart';
import 'package:kanari_pay/kanari_pay.dart';
import '../../core/token_utils.dart' as token_utils;
import '../widgets/security_card.dart';
import '../network_selector.dart';
import '../wallet_info_card.dart';
import '../widgets/app_ui.dart';
import 'wallet_transactions_screen.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  HomeScreenState createState() => HomeScreenState();
}

class HomeScreenState extends State<HomeScreen> {
  late PageController _pageController;
  late ScrollController _scrollController;
  Timer? _debounce;

  @override
  void initState() {
    super.initState();
    final state = context.read<WalletState>();
    int initialPage = 0;
    if (state.activeWalletId != null) {
      initialPage = state.wallets.indexWhere(
        (w) => w['id'] == state.activeWalletId,
      );
      if (initialPage == -1) initialPage = 0;
    }

    _pageController = PageController(
      initialPage: initialPage,
      viewportFraction: 0.9,
    );

    _scrollController = ScrollController();
  }

  @override
  void dispose() {
    _pageController.dispose();
    _scrollController.dispose();
    _debounce?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final state = context.watch<WalletState>();
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;
    final screenWidth = MediaQuery.of(context).size.width;
    final isSmallScreen = screenWidth < 360;
    final isMediumScreen = screenWidth >= 360 && screenWidth < 600;

    final screenPadding = isSmallScreen ? 12.0 : (isMediumScreen ? 16.0 : 20.0);
    final wallets = state.wallets;
    final pageCount = wallets.length + 1;

    return Scaffold(
      backgroundColor: colorScheme.surface,
      body: state.isLoading && wallets.isEmpty
          ? Center(child: SpinKitFadingCircle(color: colorScheme.primary))
          : Stack(
              children: [
                RefreshIndicator(
                  onRefresh: () => state.refreshBalance(),
                  backgroundColor: colorScheme.surface,
                  color: colorScheme.primary,
                  child: CustomScrollView(
                    controller: _scrollController,
                    slivers: [
                      SliverAppBar(
                        floating: true,
                        snap: true,
                        pinned: false,
                        backgroundColor: Colors.transparent,
                        surfaceTintColor: Colors.transparent,
                        elevation: 0,
                        toolbarHeight: 88,
                        flexibleSpace: SafeArea(
                          child: Container(
                            margin: EdgeInsets.symmetric(
                              horizontal: screenPadding,
                              vertical: 12,
                            ),
                            padding: const EdgeInsets.symmetric(
                              horizontal: 12,
                              vertical: 8,
                            ),
                            decoration: BoxDecoration(
                              color: colorScheme.surfaceContainerLowest,
                              borderRadius: BorderRadius.circular(32),
                              border: Border.all(
                                color: colorScheme.outlineVariant,
                              ),
                            ),
                            child: Row(
                              mainAxisAlignment: MainAxisAlignment.spaceBetween,
                              children: [
                                Row(
                                  children: [
                                    ClipOval(
                                      child: Image.asset(
                                        'assets/branding/kariicon1.png',
                                        width: 40,
                                        height: 40,
                                        fit: BoxFit.cover,
                                      ),
                                    ),
                                    const SizedBox(width: 12),
                                    Text(
                                      'KANARI',
                                      style: theme.textTheme.titleLarge
                                          ?.copyWith(
                                            fontWeight: FontWeight.w800,
                                            letterSpacing: 0,
                                            color: colorScheme.onSurface,
                                          ),
                                    ),
                                  ],
                                ),
                                Row(
                                  children: [
                                    const NetworkSelector(),
                                    const SizedBox(width: 4),
                                    IconButton(
                                      onPressed: () {
                                        Navigator.of(
                                          context,
                                        ).pushNamed('/settings');
                                      },
                                      icon: Icon(
                                        Icons.settings_rounded,
                                        color: colorScheme.onSurface,
                                      ),
                                      tooltip: 'Settings',
                                    ),
                                  ],
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),

                      SliverToBoxAdapter(
                        child: Padding(
                          padding: const EdgeInsets.only(top: 8.0),
                          child: SizedBox(
                            height: isSmallScreen ? 280 : 340,
                            child: PageView.builder(
                              controller: _pageController,
                              itemCount: pageCount,
                              scrollDirection: Axis.horizontal,
                              physics: const BouncingScrollPhysics(),
                              scrollBehavior: const MaterialScrollBehavior()
                                  .copyWith(
                                    dragDevices: {
                                      PointerDeviceKind.touch,
                                      PointerDeviceKind.mouse,
                                    },
                                  ),
                              onPageChanged: (index) {
                                if (_debounce?.isActive ?? false) {
                                  _debounce!.cancel();
                                }
                                _debounce = Timer(
                                  const Duration(milliseconds: 300),
                                  () {
                                    if (mounted && index < wallets.length) {
                                      final targetWalletId =
                                          wallets[index]['id'];
                                      if (context
                                              .read<WalletState>()
                                              .activeWalletId !=
                                          targetWalletId) {
                                        context
                                            .read<WalletState>()
                                            .switchWallet(targetWalletId);
                                      }
                                    }
                                  },
                                );
                              },
                              itemBuilder: (context, index) {
                                return Padding(
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: 8.0,
                                    vertical: 8.0,
                                  ),
                                  child: (index < wallets.length)
                                      ? _buildWalletCardPage(
                                          context,
                                          colorScheme,
                                          wallets[index],
                                          theme,
                                          isSmallScreen,
                                        )
                                      : _buildCreateWalletCardPage(
                                          context,
                                          colorScheme,
                                          theme,
                                          isSmallScreen,
                                        ),
                                );
                              },
                            ),
                          ),
                        ),
                      ),

                      SliverToBoxAdapter(
                        child: Padding(
                          padding: EdgeInsets.fromLTRB(
                            screenPadding,
                            16,
                            screenPadding,
                            0,
                          ),
                          child: _buildWalletStatusSection(
                            context,
                            theme,
                            colorScheme,
                          ),
                        ),
                      ),

                      SliverToBoxAdapter(
                        child: Padding(
                          padding: EdgeInsets.fromLTRB(
                            screenPadding,
                            16,
                            screenPadding,
                            0,
                          ),
                          child: _buildAssetsSection(
                            context,
                            theme,
                            colorScheme,
                            isSmallScreen,
                          ),
                        ),
                      ),

                      SliverToBoxAdapter(
                        child: Padding(
                          padding: EdgeInsets.fromLTRB(
                            screenPadding,
                            24,
                            screenPadding,
                            0,
                          ),
                          child: const WalletInfoCard(),
                        ),
                      ),

                      SliverToBoxAdapter(
                        child: Padding(
                          padding: EdgeInsets.fromLTRB(
                            screenPadding,
                            20,
                            screenPadding,
                            0,
                          ),
                          child: const SecurityCard(),
                        ),
                      ),

                      const SliverToBoxAdapter(child: SizedBox(height: 100)),
                    ],
                  ),
                ),
              ],
            ),
    );
  }

  Widget _buildWalletStatusSection(
    BuildContext context,
    ThemeData theme,
    ColorScheme colorScheme,
  ) {
    final state = context.watch<WalletState>();
    final error = state.error;

    if (error != null && error.isNotEmpty) {
      return AppStatusBanner(
        message:
            '$error\nNetwork: ${state.environment.name.toUpperCase()} (${state.environment.rpcUrl})',
        tone: AppStatusTone.warning,
        icon: Icons.cloud_off_rounded,
      );
    }

    if (state.wallet == null || state.tokenBalances.isNotEmpty) {
      return const SizedBox.shrink();
    }

    return Container(
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerLowest,
        borderRadius: BorderRadius.circular(18),
        border: Border.all(color: colorScheme.outlineVariant),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 44,
            height: 44,
            decoration: BoxDecoration(
              color: colorScheme.primaryContainer.withValues(alpha: 0.5),
              borderRadius: BorderRadius.circular(14),
            ),
            child: Icon(
              Icons.account_balance_wallet_outlined,
              color: colorScheme.onPrimaryContainer,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'No on-chain assets loaded',
                  style: theme.textTheme.titleSmall?.copyWith(
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 4),
                Text(
                  'RPC: ${state.environment.rpcUrl}\nTap refresh after faucet/transfer, or switch network if this wallet lives on another node.',
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: colorScheme.onSurfaceVariant,
                    height: 1.35,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          IconButton.filledTonal(
            onPressed: state.isLoading ? null : () => state.refreshBalance(),
            icon: const Icon(Icons.refresh_rounded),
            tooltip: 'Refresh balances',
          ),
        ],
      ),
    );
  }

  Widget _buildAssetsSection(
    BuildContext context,
    ThemeData theme,
    ColorScheme colorScheme,
    bool isSmallScreen,
  ) {
    final state = context.watch<WalletState>();
    final tokens = state.tokenBalances;

    if (tokens.isEmpty) {
      return const SizedBox.shrink();
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: 4, bottom: 12),
          child: Text(
            'Your Assets',
            style: theme.textTheme.labelSmall?.copyWith(
              fontWeight: FontWeight.w600,
              color: theme.colorScheme.onSurface.withValues(alpha: 0.4),
              letterSpacing: 1.5,
            ),
          ),
        ),
        Container(
          decoration: BoxDecoration(
            color: colorScheme.surface,
            borderRadius: BorderRadius.circular(16),
            border: Border.all(
              color: colorScheme.outline.withValues(alpha: 0.2),
            ),
          ),
          child: ListView.separated(
            shrinkWrap: true,
            physics: const NeverScrollableScrollPhysics(),
            itemCount: tokens.length,
            separatorBuilder: (context, index) => Divider(
              height: 1,
              indent: 16,
              endIndent: 16,
              color: colorScheme.outline.withValues(alpha: 0.05),
            ),
            itemBuilder: (context, index) {
              final token = tokens[index];
              final formattedAmount = token_utils.displayAmountFromBaseUnits(
                token.amount,
                token.decimals,
              );
              final isKanari = token_utils.isKanariToken(token);

              return Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: isSmallScreen ? 16 : 20,
                  vertical: isSmallScreen ? 12 : 16,
                ),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    TokenLogo(
                      tokenType: token.tokenType,
                      symbol: token.symbol,
                      size: isSmallScreen ? 40 : 44,
                      logoUrl: token.iconUrl,
                    ),
                    SizedBox(width: isSmallScreen ? 12 : 16),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Text(
                            token.symbol,
                            style: theme.textTheme.titleMedium?.copyWith(
                              fontWeight: FontWeight.bold,
                              height: 1.1,
                            ),
                          ),
                          if (!isKanari) ...[
                            const SizedBox(height: 4),
                            Text(
                              token.tokenType,
                              style: TextStyle(
                                fontSize: 10,
                                color: colorScheme.onSurface.withValues(
                                  alpha: 0.4,
                                ),
                                height: 1.1,
                              ),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ],
                        ],
                      ),
                    ),
                    const SizedBox(width: 8),
                    Text(
                      formattedAmount.toStringAsFixed(4),
                      style: theme.textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ],
                ),
              );
            },
          ),
        ),
      ],
    );
  }

  Widget _buildWalletCardPage(
    BuildContext context,
    ColorScheme colorScheme,
    Map<String, dynamic> walletData,
    ThemeData theme,
    bool isSmallScreen,
  ) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520),
        child: Container(
          padding: EdgeInsets.symmetric(
            horizontal: isSmallScreen ? 16 : 24,
            vertical: isSmallScreen ? 16 : 24,
          ),
          decoration: BoxDecoration(
            gradient: LinearGradient(
              colors: [
                colorScheme.primary.withValues(alpha: 0.85),
                colorScheme.primary,
              ],
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
            ),
            borderRadius: BorderRadius.circular(32),
            boxShadow: [
              BoxShadow(
                color: colorScheme.primary.withValues(alpha: 0.2),
                blurRadius: 15,
                offset: const Offset(0, 8),
              ),
            ],
          ),
          child: Column(
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Expanded(
                    child: Text(
                      walletData['name'] ?? 'Wallet',
                      style: theme.textTheme.titleSmall?.copyWith(
                        color: colorScheme.onPrimary.withValues(alpha: 0.8),
                        fontWeight: FontWeight.bold,
                        letterSpacing: 0.5,
                      ),
                    ),
                  ),
                  Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      _WalletIconAction(
                        icon: Icons.history_rounded,
                        tooltip: 'History',
                        colorScheme: colorScheme,
                        onPressed: () =>
                            _openWalletTransactions(context, walletData),
                      ),
                      const SizedBox(width: 6),
                      _WalletIconAction(
                        icon: Icons.refresh_rounded,
                        tooltip: 'Refresh',
                        colorScheme: colorScheme,
                        onPressed: () {
                          if (walletData['id'] ==
                              context.read<WalletState>().activeWalletId) {
                            context.read<WalletState>().refreshBalance();
                          }
                        },
                      ),
                      const SizedBox(width: 6),
                      _WalletIconAction(
                        icon: Icons.delete_outline_rounded,
                        tooltip: 'Delete Wallet',
                        colorScheme: colorScheme,
                        onPressed: () => _confirmDeleteWallet(
                          context,
                          walletData['id'] as String,
                          walletData['name'] ?? 'Wallet',
                        ),
                      ),
                    ],
                  ),
                ],
              ),

              Expanded(
                child: Center(
                  child: SingleChildScrollView(
                    physics: const BouncingScrollPhysics(),
                    child: Padding(
                      padding: const EdgeInsets.only(top: 8.0),
                      child: _buildIndexSpecificBalanceSection(
                        context,
                        colorScheme,
                        walletData,
                        isSmallScreen,
                        theme,
                      ),
                    ),
                  ),
                ),
              ),

              const SizedBox(height: 2),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _openWalletTransactions(
    BuildContext context,
    Map<String, dynamic> walletData,
  ) async {
    final walletState = context.read<WalletState>();
    final address = await walletState.walletAddressFromData(walletData);

    if (!context.mounted) return;

    if (address == null || address.isEmpty) {
      showAppErrorSnackBar(context, 'Wallet address is unavailable');
      return;
    }

    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => WalletTransactionsScreen(
          walletName: walletData['name']?.toString() ?? 'Wallet',
          walletAddress: address,
        ),
      ),
    );
  }

  Widget _buildIndexSpecificBalanceSection(
    BuildContext context,
    ColorScheme colorScheme,
    Map<String, dynamic> walletData,
    bool isSmallScreen,
    ThemeData theme,
  ) {
    final state = context.watch<WalletState>();
    final isActive = walletData['id'] == state.activeWalletId;
    final displayBalance = isActive
        ? token_utils
              .displayAmountFromBaseUnits(
                state.kanariBalance,
                token_utils.kanariDecimals,
              )
              .toStringAsFixed(6)
        : "---";

    return Container(
      width: double.infinity,
      decoration: BoxDecoration(
        color: colorScheme.onPrimary.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(24),
      ),
      padding: EdgeInsets.symmetric(vertical: isSmallScreen ? 16.0 : 28.0),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            'Total Balance',
            style: theme.textTheme.labelMedium?.copyWith(
              color: colorScheme.onPrimary.withValues(alpha: 0.7),
              letterSpacing: 1.5,
              fontWeight: FontWeight.w500,
            ),
          ),
          SizedBox(height: isSmallScreen ? 8 : 16),
          FittedBox(
            fit: BoxFit.scaleDown,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16.0),
              child: Text(
                displayBalance,
                style: TextStyle(
                  fontSize: isSmallScreen ? 40.0 : 52.0,
                  fontWeight: FontWeight.w300,
                  color: colorScheme.onPrimary,
                  letterSpacing: -1.5,
                  height: 1.0,
                ),
              ),
            ),
          ),
          SizedBox(height: isSmallScreen ? 8 : 16),
          Container(
            padding: EdgeInsets.symmetric(
              horizontal: isSmallScreen ? 12 : 16,
              vertical: isSmallScreen ? 4 : 6,
            ),
            decoration: BoxDecoration(
              color: colorScheme.onPrimary.withValues(alpha: 0.12),
              borderRadius: BorderRadius.circular(20),
            ),
            child: Text(
              'KANARI',
              style: TextStyle(
                fontSize: isSmallScreen ? 10 : 11,
                color: colorScheme.onPrimary,
                fontWeight: FontWeight.w600,
                letterSpacing: 2,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildCreateWalletCardPage(
    BuildContext context,
    ColorScheme colorScheme,
    ThemeData theme,
    bool isSmallScreen,
  ) {
    return Container(
      padding: EdgeInsets.all(isSmallScreen ? 16 : 24),
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerHighest.withValues(alpha: 0.3),
        borderRadius: BorderRadius.circular(32),
        border: Border.all(
          color: colorScheme.outlineVariant.withValues(alpha: 0.5),
          width: 2,
        ),
      ),
      child: Center(
        child: SingleChildScrollView(
          physics: const BouncingScrollPhysics(),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Container(
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                  color: colorScheme.surfaceContainerHighest,
                  shape: BoxShape.circle,
                  border: Border.all(
                    color: colorScheme.outline.withValues(alpha: 0.3),
                  ),
                ),
                child: Icon(
                  Icons.add_rounded,
                  size: 32,
                  color: colorScheme.onSurface,
                ),
              ),
              const SizedBox(height: 16),
              Text(
                'Create New Wallet',
                style: theme.textTheme.titleMedium?.copyWith(
                  fontWeight: FontWeight.bold,
                  color: colorScheme.onSurface,
                ),
              ),
              const SizedBox(height: 8),
              Text(
                'Swipe left to manage existing wallets\nor tap below to add a new one.',
                textAlign: TextAlign.center,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: colorScheme.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: 24),
              FilledButton.tonal(
                onPressed: () => _showCreateDialog(context),
                child: const Text('Add Wallet'),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _confirmDeleteWallet(
    BuildContext context,
    String walletId,
    String walletName,
  ) async {
    final walletState = context.read<WalletState>();
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Delete Wallet'),
        content: Text(
          'Are you sure you want to delete "$walletName"? This cannot be undone.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(dialogContext).pop(true),
            style: FilledButton.styleFrom(
              backgroundColor: Theme.of(context).colorScheme.error,
              foregroundColor: Theme.of(context).colorScheme.onError,
            ),
            child: const Text('Delete'),
          ),
        ],
      ),
    );

    if (confirmed != true) return;

    await walletState.removeWallet(walletId);
    if (!mounted) return;

    if (_pageController.hasClients) {
      final currentPage =
          _pageController.page?.round() ?? _pageController.initialPage;
      final remainingWallets = walletState.wallets.length;
      final targetPage = remainingWallets > 0
          ? currentPage.clamp(0, remainingWallets - 1)
          : 0;

      if (targetPage != currentPage) {
        _pageController.jumpToPage(targetPage);
      }
    }
  }

  void _showCreateDialog(BuildContext context) {
    KanariCurve selectedCurve = KanariCurve.ed25519;
    final derivationPathController = TextEditingController(
      text: KanariWallet.defaultDerivationPath,
    );

    showDialog(
      context: context,
      builder: (dialogContext) => StatefulBuilder(
        builder: (context, setState) => AlertDialog(
          title: const Text('Create New Wallet'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text(
                'Select the cryptographic curve for your new wallet.',
                style: TextStyle(fontSize: 13, color: Colors.grey),
              ),
              const SizedBox(height: 16),
              DropdownButtonFormField<KanariCurve>(
                initialValue: selectedCurve,
                isExpanded: true,
                decoration: const InputDecoration(
                  labelText: 'Curve Type',
                  border: OutlineInputBorder(),
                ),
                items: KanariCurve.values.map((curve) {
                  return DropdownMenuItem(
                    value: curve,
                    child: Text(curve.name, overflow: TextOverflow.ellipsis),
                  );
                }).toList(),
                onChanged: (val) => setState(() => selectedCurve = val!),
              ),
              if (!selectedCurve.isPostQuantum) ...[
                const SizedBox(height: 16),
                TextField(
                  controller: derivationPathController,
                  decoration: const InputDecoration(
                    labelText: 'HD Path',
                    hintText: KanariWallet.defaultDerivationPath,
                    border: OutlineInputBorder(),
                  ),
                ),
              ],
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(dialogContext),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () async {
                final walletState = context.read<WalletState>();
                final derivationPath = selectedCurve.isPostQuantum
                    ? KanariWallet.defaultDerivationPath
                    : derivationPathController.text.trim();
                if (!selectedCurve.isPostQuantum &&
                    !KanariWallet.isValidDerivationPath(derivationPath)) {
                  showAppErrorSnackBar(
                    context,
                    "Invalid HD path. Example: ${KanariWallet.defaultDerivationPath}",
                  );
                  return;
                }

                final authorized = await showAppPinVerificationSheet(
                  context: context,
                  onVerify: walletState.verifyPin,
                  lockRemaining: walletState.pinLockRemaining,
                  title: 'Confirm PIN',
                  subtitle: 'Enter your 6-digit PIN to create a new wallet.',
                );

                if (!mounted || !dialogContext.mounted || !authorized) return;

                await walletState.createNewWallet(
                  curve: selectedCurve,
                  pin: '',
                  derivationPath: derivationPath,
                );

                if (!mounted || !dialogContext.mounted) return;

                Navigator.pop(dialogContext);
                final targetPage = walletState.wallets.length - 1;

                Future.delayed(const Duration(milliseconds: 300), () {
                  if (!mounted || !_pageController.hasClients) return;
                  _pageController.animateToPage(
                    targetPage,
                    duration: const Duration(milliseconds: 400),
                    curve: Curves.easeOutCubic,
                  );
                });
              },
              child: const Text('Generate'),
            ),
          ],
        ),
      ),
    ).whenComplete(derivationPathController.dispose);
  }
}

class _WalletIconAction extends StatelessWidget {
  final IconData icon;
  final String tooltip;
  final ColorScheme colorScheme;
  final VoidCallback onPressed;

  const _WalletIconAction({
    required this.icon,
    required this.tooltip,
    required this.colorScheme,
    required this.onPressed,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox.square(
      dimension: 36,
      child: IconButton(
        onPressed: onPressed,
        icon: Icon(icon, size: 19, color: colorScheme.onPrimary),
        tooltip: tooltip,
        style: IconButton.styleFrom(
          backgroundColor: colorScheme.onPrimary.withValues(alpha: 0.08),
          hoverColor: colorScheme.onPrimary.withValues(alpha: 0.12),
          padding: EdgeInsets.zero,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(14),
          ),
        ),
      ),
    );
  }
}
