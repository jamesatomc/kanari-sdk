import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:kanari_pay/kanari_pay.dart';
import 'package:kanari_pay/src/providers/wallet_provider.dart';
import 'package:kanari_pay/theme.dart';
import 'package:provider/provider.dart';

import '../widgets/app_ui.dart';
import '../widgets/onboarding_sheets.dart';

class KanariWelcomeScreen extends StatefulWidget {
  const KanariWelcomeScreen({super.key});

  @override
  State<KanariWelcomeScreen> createState() => _KanariWelcomeScreenState();
}

class _KanariWelcomeScreenState extends State<KanariWelcomeScreen>
    with SingleTickerProviderStateMixin {
  late final AnimationController _revealController;

  @override
  void initState() {
    super.initState();
    _revealController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 900),
    )..forward();
  }

  @override
  void dispose() {
    _revealController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final wallet = context.watch<WalletState>();
    final auth = context.watch<KanariAuthClient>();
    final reduceMotion = MediaQuery.disableAnimationsOf(context);

    if (reduceMotion && _revealController.value != 1) {
      _revealController.value = 1;
    }

    return AppGradientScaffold(
      body: LayoutBuilder(
        builder: (context, constraints) {
          final wide = constraints.maxWidth >= 850;
          return SingleChildScrollView(
            padding: EdgeInsets.symmetric(
              horizontal: wide ? 40 : 20,
              vertical: 24,
            ),
            child: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 1120),
                child: Column(
                  children: [
                    _Reveal(
                      animation: _revealController,
                      interval: const Interval(
                        0,
                        .55,
                        curve: Curves.easeOutCubic,
                      ),
                      offset: const Offset(0, -18),
                      child: const _BrandBar(),
                    ),
                    SizedBox(height: wide ? 60 : 28),
                    if (wide)
                      Row(
                        crossAxisAlignment: CrossAxisAlignment.center,
                        children: [
                          Expanded(
                            flex: 6,
                            child: _Reveal(
                              animation: _revealController,
                              interval: const Interval(
                                .12,
                                .82,
                                curve: Curves.easeOutCubic,
                              ),
                              offset: const Offset(-28, 22),
                              child: const _BrandHero(),
                            ),
                          ),
                          const SizedBox(width: 64),
                          Expanded(
                            flex: 5,
                            child: _Reveal(
                              animation: _revealController,
                              interval: const Interval(
                                .28,
                                1,
                                curve: Curves.easeOutCubic,
                              ),
                              offset: const Offset(30, 26),
                              child: _AccessPanel(
                                wallet: wallet,
                                auth: auth,
                                onUnlock: () => _showUnlock(context),
                                onCreate: () => _showCreate(context),
                                onImport: () => _showImport(context),
                              ),
                            ),
                          ),
                        ],
                      )
                    else
                      Column(
                        children: [
                          _Reveal(
                            animation: _revealController,
                            interval: const Interval(
                              .12,
                              .75,
                              curve: Curves.easeOutCubic,
                            ),
                            offset: const Offset(0, 20),
                            child: const _BrandHero(compact: true),
                          ),
                          const SizedBox(height: 24),
                          _Reveal(
                            animation: _revealController,
                            interval: const Interval(
                              .25,
                              .92,
                              curve: Curves.easeOutCubic,
                            ),
                            offset: const Offset(0, 24),
                            child: _AccessPanel(
                              wallet: wallet,
                              auth: auth,
                              onUnlock: () => _showUnlock(context),
                              onCreate: () => _showCreate(context),
                              onImport: () => _showImport(context),
                            ),
                          ),
                        ],
                      ),
                  ],
                ),
              ),
            ),
          );
        },
      ),
    );
  }

  Future<void> _showUnlock(BuildContext context) async {
    final state = context.read<WalletState>();
    final result = await showAppModalSheet<String>(
      context: context,
      builder: (_) => AppPinEntrySheet(
        title: 'Unlock Wallet',
        subtitle: 'Enter your 6-digit PIN',
        onBiometricAuthenticate: state.unlockWithBiometric,
        biometricReason: 'Unlock your Kanari wallet',
        biometricHandlesPrompt: true,
        onValidate: state.verifyPin,
        onComplete: (pin) async {
          await state.unlockWallet(pin);
          if (!context.mounted) return;
          if (state.isUnlocked && state.hasWallet) {
            Navigator.of(context).pushReplacementNamed('/home');
          }
        },
      ),
    );

    if (!context.mounted || result == null) return;

    if (state.isUnlocked && state.hasWallet) {
      Navigator.of(context).pushReplacementNamed('/home');
    } else if (result == appPinBiometricResult && state.error != null) {
      _showError(context, state.error!);
    }
  }

  void _showCreate(BuildContext context) {
    showAppModalSheet(
      context: context,
      builder: (_) => AppPinEntrySheet(
        title: 'Set PIN',
        subtitle: 'Set a 6-digit PIN to secure your wallet.',
        onComplete: (pin) {
          Future.delayed(const Duration(milliseconds: 150), () {
            if (context.mounted) _showCurve(context, pin);
          });
        },
      ),
    );
  }

  void _showCurve(BuildContext context, String pin) {
    final state = context.read<WalletState>();
    showAppModalSheet(
      context: context,
      builder: (sheetContext) => AppCurveSelectionSheet(
        onConfirm: (curve, derivationPath) async {
          Navigator.pop(sheetContext);
          _showBusyDialog(context, message: 'Creating wallet...');
          await state.createNewWallet(
            curve: curve,
            pin: pin,
            derivationPath: derivationPath,
          );
          if (!context.mounted) return;
          Navigator.of(context, rootNavigator: true).pop();
          if (state.hasWallet) {
            Navigator.of(context).pushReplacementNamed('/home');
          } else {
            _showError(context, state.error ?? 'Failed to create wallet');
          }
        },
      ),
    );
  }

  void _showImport(BuildContext context) {
    showAppModalSheet(
      context: context,
      builder: (_) => AppImportWalletSheet(
        onContinue: (data, curve, isMnemonic, derivationPath) {
          Future.delayed(const Duration(milliseconds: 150), () {
            if (context.mounted) {
              _showImportPin(context, data, curve, isMnemonic, derivationPath);
            }
          });
        },
      ),
    );
  }

  void _showImportPin(
    BuildContext context,
    String data,
    KanariCurve curve,
    bool isMnemonic,
    String derivationPath,
  ) {
    final state = context.read<WalletState>();
    showAppModalSheet(
      context: context,
      builder: (_) => AppPinEntrySheet(
        title: 'Set PIN',
        subtitle: 'Set a 6-digit PIN to secure your imported wallet.',
        onComplete: (pin) async {
          _showBusyDialog(context, message: 'Importing wallet...');
          if (isMnemonic) {
            await state.importFromMnemonic(
              data,
              curve: curve,
              pin: pin,
              derivationPath: derivationPath,
            );
          } else {
            await state.importFromPrivateKey(data, curve: curve, pin: pin);
          }
          if (!context.mounted) return;
          Navigator.of(context, rootNavigator: true).pop();
          if (state.hasWallet) {
            Navigator.of(context).pushReplacementNamed('/home');
          } else {
            _showError(context, state.error ?? 'Failed to import wallet');
          }
        },
      ),
    );
  }

  void _showError(BuildContext context, String message) {
    showAppErrorSnackBar(context, message);
  }

  void _showBusyDialog(BuildContext context, {required String message}) {
    showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (dialogContext) => PopScope(
        canPop: false,
        child: AlertDialog(
          content: Row(
            children: [
              const SizedBox(
                width: 22,
                height: 22,
                child: CircularProgressIndicator(strokeWidth: 2.4),
              ),
              const SizedBox(width: 16),
              Expanded(child: Text(message)),
            ],
          ),
        ),
      ),
    );
  }
}

class _Reveal extends StatelessWidget {
  final Animation<double> animation;
  final Interval interval;
  final Offset offset;
  final Widget child;

  const _Reveal({
    required this.animation,
    required this.interval,
    required this.offset,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final curved = CurvedAnimation(parent: animation, curve: interval);
    return FadeTransition(
      opacity: curved,
      child: SlideTransition(
        position: Tween<Offset>(
          begin: Offset(offset.dx / 100, offset.dy / 100),
          end: Offset.zero,
        ).animate(curved),
        child: child,
      ),
    );
  }
}

class _BrandBar extends StatelessWidget {
  const _BrandBar();

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return Container(
      height: 64,
      padding: const EdgeInsets.all(8),
      decoration: BoxDecoration(
        color: colors.surfaceContainerLowest,
        border: Border.all(color: colors.outlineVariant),
        borderRadius: BorderRadius.circular(32),
      ),
      child: Row(
        children: [
          ClipOval(
            child: Image.asset(
              'assets/branding/kariicon1.png',
              width: 46,
              height: 46,
              fit: BoxFit.cover,
            ),
          ),
          const SizedBox(width: 10),
          Text(
            'KANARI',
            style: Theme.of(
              context,
            ).textTheme.titleLarge?.copyWith(fontWeight: FontWeight.w900),
          ),
          const Spacer(),
          if (MediaQuery.sizeOf(context).width >= 520)
            TextButton(
              onPressed: () => Navigator.of(context).pushNamed('/login'),
              child: const Text('LOGIN'),
            ),
          FilledButton(
            onPressed: () => Navigator.of(context).pushNamed('/register'),
            style: FilledButton.styleFrom(
              minimumSize: const Size(0, 44),
              backgroundColor: KanariColors.lime,
              foregroundColor: KanariColors.ink,
            ),
            child: const Text('REGISTER'),
          ),
        ],
      ),
    );
  }
}

class _BrandHero extends StatelessWidget {
  final bool compact;

  const _BrandHero({this.compact = false});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final wide = MediaQuery.sizeOf(context).width >= 850;
    return Column(
      crossAxisAlignment: wide
          ? CrossAxisAlignment.start
          : CrossAxisAlignment.center,
      children: [
        Text(
          'YOUR ASSETS. YOUR CONTROL.',
          style: theme.textTheme.labelMedium?.copyWith(
            color: theme.colorScheme.secondary,
          ),
        ),
        const SizedBox(height: 22),
        Text(
          'Own your\nnext move.',
          textAlign: wide ? TextAlign.left : TextAlign.center,
          style: compact
              ? theme.textTheme.displayMedium
              : theme.textTheme.displayLarge,
        ),
        SizedBox(height: compact ? 12 : 18),
        Text(
          'A secure Kanari wallet for Move-powered digital assets.',
          textAlign: wide ? TextAlign.left : TextAlign.center,
          style: theme.textTheme.bodyLarge?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
        if (!compact) ...[const SizedBox(height: 30), const _NetworkMark()],
      ],
    );
  }
}

class _NetworkMark extends StatefulWidget {
  const _NetworkMark();

  @override
  State<_NetworkMark> createState() => _NetworkMarkState();
}

class _NetworkMarkState extends State<_NetworkMark>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 26),
    )..repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final viewport = MediaQuery.sizeOf(context);
    final size = viewport.width >= 850
        ? 340.0
        : math.min(viewport.width - 72, 240.0);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final nodeSize = size < 240 ? 38.0 : 48.0;
    final orbitRadius = size * (size < 240 ? .34 : .36);

    return AnimatedBuilder(
      animation: _controller,
      builder: (context, child) {
        final progress = reduceMotion ? 0.0 : _controller.value;
        final angle = progress * math.pi * 2;
        final pulse = reduceMotion ? 1.0 : 1 + math.sin(angle * 2) * .025;
        final float = reduceMotion ? 0.0 : math.sin(angle) * 7;

        return Transform.translate(
          offset: Offset(0, float),
          child: SizedBox(
            width: size,
            height: size,
            child: Stack(
              alignment: Alignment.center,
              children: [
                Container(
                  decoration: BoxDecoration(
                    color: KanariColors.lavender,
                    shape: BoxShape.circle,
                    border: Border.all(
                      color: KanariColors.ink.withValues(alpha: .16),
                    ),
                    boxShadow: [
                      BoxShadow(
                        color: KanariColors.purple.withValues(alpha: .16),
                        blurRadius: 44,
                        offset: const Offset(0, 22),
                      ),
                    ],
                  ),
                ),
                Stack(
                  alignment: Alignment.center,
                  children: [
                    for (final factor in [.72, .48])
                      Container(
                        width: size * factor,
                        height: size * factor,
                        decoration: BoxDecoration(
                          shape: BoxShape.circle,
                          border: Border.all(
                            color: KanariColors.ink.withValues(alpha: .2),
                          ),
                        ),
                      ),
                    _OrbitNode(
                      label: 'K',
                      angle: angle - math.pi * .78,
                      radius: orbitRadius,
                      size: nodeSize,
                    ),
                    _OrbitNode(
                      label: 'M',
                      angle: angle + math.pi * .05,
                      radius: orbitRadius * .82,
                      size: nodeSize,
                      dark: true,
                    ),
                    _OrbitNode(
                      label: '01',
                      angle: angle + math.pi * .78,
                      radius: orbitRadius,
                      size: nodeSize,
                    ),
                  ],
                ),
                Transform.scale(
                  scale: pulse,
                  child: Container(
                    width: size * .26,
                    height: size * .26,
                    padding: const EdgeInsets.all(10),
                    decoration: BoxDecoration(
                      color: KanariColors.lime,
                      shape: BoxShape.circle,
                      boxShadow: [
                        BoxShadow(
                          color: KanariColors.lime.withValues(alpha: .28),
                          blurRadius: 22,
                          spreadRadius: 6,
                        ),
                      ],
                    ),
                    child: ClipOval(
                      child: Image.asset('assets/branding/kariicon1.png'),
                    ),
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _Node extends StatelessWidget {
  final String label;
  final bool dark;
  final double size;

  const _Node(this.label, {this.dark = false, this.size = 48});

  @override
  Widget build(BuildContext context) {
    return Container(
      height: size,
      width: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: dark ? KanariColors.ink : KanariColors.paper,
        shape: BoxShape.circle,
        border: Border.all(color: KanariColors.ink.withValues(alpha: .16)),
      ),
      child: Text(
        label,
        style: TextStyle(
          color: dark ? KanariColors.cream : KanariColors.ink,
          fontWeight: FontWeight.w900,
        ),
      ),
    );
  }
}

class _OrbitNode extends StatelessWidget {
  final String label;
  final double angle;
  final double radius;
  final double size;
  final bool dark;

  const _OrbitNode({
    required this.label,
    required this.angle,
    required this.radius,
    required this.size,
    this.dark = false,
  });

  @override
  Widget build(BuildContext context) {
    return Transform.translate(
      offset: Offset(math.cos(angle) * radius, math.sin(angle) * radius),
      child: _Node(label, dark: dark, size: size),
    );
  }
}

class _AccessPanel extends StatefulWidget {
  final WalletState wallet;
  final KanariAuthClient auth;
  final VoidCallback onUnlock;
  final VoidCallback onCreate;
  final VoidCallback onImport;

  const _AccessPanel({
    required this.wallet,
    required this.auth,
    required this.onUnlock,
    required this.onCreate,
    required this.onImport,
  });

  @override
  State<_AccessPanel> createState() => _AccessPanelState();
}

class _AccessPanelState extends State<_AccessPanel> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = theme.colorScheme;
    final reduceMotion = MediaQuery.disableAnimationsOf(context);

    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: AnimatedContainer(
        duration: reduceMotion
            ? Duration.zero
            : const Duration(milliseconds: 260),
        curve: Curves.easeOutCubic,
        transform: Matrix4.translationValues(0, _hovered ? -6 : 0, 0),
        padding: const EdgeInsets.all(24),
        decoration: BoxDecoration(
          color: colors.surfaceContainerLowest,
          borderRadius: BorderRadius.circular(24),
          border: Border.all(
            color: _hovered ? colors.secondary : colors.outlineVariant,
          ),
          boxShadow: _hovered
              ? [
                  BoxShadow(
                    color: colors.shadow.withValues(alpha: .12),
                    blurRadius: 28,
                    offset: const Offset(0, 16),
                  ),
                ]
              : const [],
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('WALLET ACCESS', style: theme.textTheme.labelMedium),
            const SizedBox(height: 10),
            Text(
              widget.wallet.hasWallet ? 'Welcome back.' : 'Start with Kanari.',
              style: theme.textTheme.headlineLarge,
            ),
            const SizedBox(height: 8),
            Text(
              widget.wallet.hasWallet
                  ? 'Unlock your saved wallet or add another account.'
                  : 'Create a wallet or import an existing private key.',
              style: theme.textTheme.bodyMedium?.copyWith(
                color: colors.onSurfaceVariant,
              ),
            ),
            if (widget.auth.isAuthenticated) ...[
              const SizedBox(height: 20),
              AppAccountSummaryPanel(title: widget.auth.userEmail ?? 'User'),
            ],
            const SizedBox(height: 26),
            if (widget.wallet.hasWallet) ...[
              AppWideButton(
                onPressed: widget.onUnlock,
                icon: Icons.lock_open_rounded,
                label: 'Unlock Saved Wallet',
              ),
              const SizedBox(height: 10),
            ],
            AppWideButton(
              onPressed: widget.onCreate,
              icon: Icons.add_rounded,
              label: 'Create New Wallet',
              style: AppWideButtonStyle.tonal,
            ),
            const SizedBox(height: 10),
            AppWideButton(
              onPressed: widget.onImport,
              icon: Icons.file_download_outlined,
              label: 'Import Existing Wallet',
              style: AppWideButtonStyle.outlined,
            ),
            const SizedBox(height: 22),
            const AppLabeledDivider(label: 'ACCOUNT'),
            const SizedBox(height: 8),
            TextButton.icon(
              onPressed: () => Navigator.of(context).pushNamed('/login'),
              icon: const Icon(Icons.login_rounded),
              label: const Text('Login to Kanari account'),
            ),
          ],
        ),
      ),
    );
  }
}
