import 'package:flutter/material.dart';

import 'package:kanari_pay/src/providers/wallet_provider.dart';
import 'package:provider/provider.dart';
import 'package:kanari_pay/kanari_pay.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../widgets/app_ui.dart';
import '../widgets/onboarding_sheets.dart';

class WelcomeScreen extends StatefulWidget {
  const WelcomeScreen({super.key});

  @override
  State<WelcomeScreen> createState() => _WelcomeScreenState();
}

class _WelcomeScreenState extends State<WelcomeScreen> {
  bool _isInitialized = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    // ป้องกันการทำงานซ้ำ
    if (_isInitialized) return;
    _isInitialized = true;

    // ตรวจสอบสถานะหลังจาก widget ถูก mount
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
    });
  }

  @override
  Widget build(BuildContext context) {
    final state = context.watch<WalletState>();
    final authClient = context.watch<KanariAuthClient>();
    final colorScheme = Theme.of(context).colorScheme;

    return AppGradientScaffold(
      body: CustomScrollView(
        slivers: [
          SliverFillRemaining(
            hasScrollBody: false,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16.0),
              child: Column(
                children: [
                  const Spacer(),

                  // 🔐 Authentication Section (แสดงเมื่อยังไม่ได้ login)
                  if (!authClient.isAuthenticated) ...[
                    const AuthHero(
                      icon: Icons.blur_on_rounded,
                      title: 'Kanari Wallet',
                      subtitle: 'Secure, Quantum-Safe Digital Wallet',
                    ),
                  ],

                  const Spacer(),

                  // 💼 Wallet Management Section
                  if (authClient.isAuthenticated) ...[
                    // แสดงข้อมูล user ที่ login อยู่
                    AppAccountSummaryPanel(
                      title: authClient.userEmail ?? 'User',
                      subtitle: authClient.walletAddress != null
                          ? '${authClient.walletAddress!.substring(0, 8)}...${authClient.walletAddress!.substring(authClient.walletAddress!.length - 6)}'
                          : null,
                      trailing: IconButton(
                        icon: Icon(
                          Icons.logout_rounded,
                          color: colorScheme.error,
                        ),
                        onPressed: () => _showLogoutDialog(context),
                        tooltip: 'Logout',
                      ),
                    ),
                    const SizedBox(height: 16),
                  ],

                  // Wallet Actions
                  if (state.hasWallet) ...[
                    AppWideButton(
                      onPressed: () => _showUnlockSheet(context),
                      icon: Icons.lock_open_rounded,
                      label: 'Unlock Saved Wallet',
                    ),
                    const SizedBox(height: 8),
                    TextButton.icon(
                      onPressed: () => _showDeleteConfirm(context, state),
                      icon: const Icon(Icons.delete_outline_rounded, size: 20),
                      label: const Text('Clear All Wallet Data'),
                      style: TextButton.styleFrom(
                        foregroundColor: colorScheme.error,
                      ),
                    ),
                    const SizedBox(height: 12),
                  ],

                  AppWideButton(
                    onPressed: () => _showCreateSheet(context),
                    icon: Icons.add_rounded,
                    label: 'Create New Wallet',
                    style: AppWideButtonStyle.tonal,
                  ),
                  const SizedBox(height: 8),
                  AppWideButton(
                    onPressed: () => _showImportDialog(context),
                    icon: Icons.file_download_outlined,
                    label: 'Import Existing Wallet',
                    style: AppWideButtonStyle.outlined,
                  ),
                  const SizedBox(height: 16),

                  // Divider
                  const AppLabeledDivider(),
                  const SizedBox(height: 16),

                  // Login / Register buttons
                  AppWideButton(
                    onPressed: () {
                      Navigator.of(context).pushNamed('/login');
                    },
                    icon: Icons.login_rounded,
                    label: 'Login',
                    style: AppWideButtonStyle.text,
                  ),
                  const SizedBox(height: 4),
                  AppWideButton(
                    onPressed: () {
                      Navigator.of(context).pushNamed('/register');
                    },
                    icon: Icons.person_add_rounded,
                    label: 'Register',
                    style: AppWideButtonStyle.text,
                  ),

                  const Spacer(),
                  const SizedBox(height: 16),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  // --- Dialogs & Sheets ---

  Future<void> _showUnlockSheet(
    BuildContext context, {
    bool isAutoTriggered = false,
  }) async {
    final state = context.read<WalletState>();

    final result = await showAppModalSheet<String>(
      context: context,
      isDismissible: !isAutoTriggered,
      enableDrag: !isAutoTriggered,
      builder: (sheetContext) => AppPinEntrySheet(
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
      showAppErrorSnackBar(context, state.error!);
    }
  }

  void _showCreateSheet(BuildContext context) {
    showAppModalSheet(
      context: context,
      builder: (context) => AppPinEntrySheet(
        title: 'Set PIN',
        subtitle: 'Set a 6-digit PIN to secure your wallet.',
        onComplete: (pin) {
          Future.delayed(const Duration(milliseconds: 150), () {
            if (context.mounted) {
              _showCurveSelectionSheet(context, pin);
            }
          });
        },
      ),
    );
  }

  void _showCurveSelectionSheet(BuildContext context, String pin) {
    final walletState = context.read<WalletState>();

    showAppModalSheet(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) => AppCurveSelectionSheet(
        onConfirm: (selectedCurve, derivationPath) async {
          Navigator.pop(sheetContext);
          _showBusyDialog(context, message: 'Creating wallet...');
          await walletState.createNewWallet(
            curve: selectedCurve,
            pin: pin,
            derivationPath: derivationPath,
          );

          if (!context.mounted) return;
          Navigator.of(context, rootNavigator: true).pop();

          final hasCreatedWallet =
              walletState.hasWallet &&
              (walletState.activeWalletId != null ||
                  walletState.wallet != null);

          if (hasCreatedWallet) {
            Navigator.of(context).pushReplacementNamed('/home');
          } else {
            showAppErrorSnackBar(
              context,
              walletState.error ?? 'Failed to create wallet',
            );
          }
        },
      ),
    );
  }

  // 👉 อัปเดต Import Flow ให้เป็น 2 สเต็ป
  void _showImportDialog(BuildContext context) {
    showAppModalSheet(
      context: context,
      showDragHandle: true,
      builder: (context) => AppImportWalletSheet(
        onContinue:
            (
              String data,
              KanariCurve curve,
              bool isMnemonic,
              String derivationPath,
            ) {
              // พอรับข้อมูลมาแล้ว ให้เปิดหน้าใส่ PIN ทันที
              Future.delayed(const Duration(milliseconds: 150), () {
                if (context.mounted) {
                  _showImportPinSheet(
                    context,
                    data,
                    curve,
                    isMnemonic,
                    derivationPath,
                  );
                }
              });
            },
      ),
    );
  }

  // 👉 หน้าต่างใส่รหัส PIN สำหรับการ Import
  void _showImportPinSheet(
    BuildContext context,
    String data,
    KanariCurve curve,
    bool isMnemonic,
    String derivationPath,
  ) {
    final parentContext = context;
    final walletState = context.read<WalletState>();
    showAppModalSheet(
      context: context,
      builder: (sheetContext) => AppPinEntrySheet(
        title: 'Set PIN',
        subtitle: 'Set a 6-digit PIN to secure your imported wallet.',
        onComplete: (pin) async {
          _showBusyDialog(parentContext, message: 'Importing wallet...');
          if (isMnemonic) {
            await walletState.importFromMnemonic(
              data,
              curve: curve,
              pin: pin,
              derivationPath: derivationPath,
            );
          } else {
            await walletState.importFromPrivateKey(
              data,
              curve: curve,
              pin: pin,
            );
          }

          if (!parentContext.mounted) return;
          Navigator.of(parentContext, rootNavigator: true).pop();

          final hasImportedWallet =
              walletState.hasWallet &&
              (walletState.activeWalletId != null ||
                  walletState.wallet != null);

          if (hasImportedWallet) {
            Navigator.of(parentContext).pushReplacementNamed('/home');
          } else {
            showAppErrorSnackBar(
              parentContext,
              walletState.error ?? 'Failed to import wallet',
            );
          }
        },
      ),
    );
  }

  void _showDeleteConfirm(BuildContext context, WalletState state) {
    showDialog(
      context: context,
      builder: (_) => const AppConfirmationDialog(
        title: 'Delete All Data?',
        content:
            'This action cannot be undone. Make sure you have backed up your seed phrases.',
        confirmLabel: 'Clear Everything',
        isDestructive: true,
      ),
    ).then((confirmed) {
      if (confirmed == true) {
        state.deleteAllWallets();
      }
    });
  }

  void _showLogoutDialog(BuildContext context) {
    final authClient = context.read<KanariAuthClient>();
    final walletState = context.read<WalletState>();
    showDialog(
      context: context,
      builder: (_) => const AppConfirmationDialog(
        title: 'Logout?',
        content: 'Are you sure you want to logout from this account?',
        confirmLabel: 'Logout',
        isDestructive: true,
      ),
    ).then((confirmed) async {
      if (confirmed == true) {
        Object? remoteLogoutError;
        try {
          await authClient.logout();
        } catch (error) {
          remoteLogoutError = error;
        }
        authClient.clearSession();
        final prefs = await SharedPreferences.getInstance();
        await prefs.remove('session_id');
        await prefs.remove('user_email');
        await prefs.remove('wallet_address');
        await walletState.removeAuthenticatedAccountWallet();
        if (!context.mounted) return;
        showAppInfoSnackBar(
          context,
          remoteLogoutError == null
              ? 'Logged out successfully'
              : 'Logged out locally. Server session will expire automatically.',
        );
      }
    });
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
