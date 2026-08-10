import 'package:flutter/material.dart';
import 'package:local_auth/local_auth.dart';
import 'package:provider/provider.dart';
import 'package:qr_flutter/qr_flutter.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../client/auth_client.dart';
import '../../models/auth_models.dart';
import '../../providers/theme_mode_provider.dart';
import '../../providers/wallet_provider.dart';
import '../../wallet_storage.dart';
import '../widgets/app_ui.dart';

class SettingScreen extends StatelessWidget {
  const SettingScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return AppGradientScaffold(
      appBar: AppBar(title: const Text('Settings'), centerTitle: true),
      body: ListView(
        padding: const EdgeInsets.all(20),
        children: [
          const AppSectionTitle('Appearance'),
          const SizedBox(height: 12),
          _ThemeModeCard(),
          const SizedBox(height: 24),
          const AppSectionTitle('Security'),
          const SizedBox(height: 12),
          _SettingsTile(
            icon: Icons.pin_rounded,
            title: 'Change PIN',
            subtitle: 'Update the 6-digit PIN for this device',
            onTap: () => _showChangePinDialog(context),
          ),
          const SizedBox(height: 12),
          const _BiometricSettingsTile(),
          const SizedBox(height: 12),
          _SettingsTile(
            icon: Icons.shield_moon_rounded,
            title: 'Set Up Two-Factor Authentication',
            subtitle:
                'Scan QR code and protect login with an authenticator app',
            onTap: () => _showSetup2faDialog(context),
          ),
          const SizedBox(height: 12),
          _SettingsTile(
            icon: Icons.shield_outlined,
            title: 'Disable Two-Factor Authentication',
            subtitle: 'Remove authenticator-based login protection',
            onTap: () => _showDisable2faDialog(context),
          ),
          const SizedBox(height: 24),
          const AppSectionTitle('Session'),
          const SizedBox(height: 12),
          _SettingsTile(
            icon: Icons.logout_rounded,
            title: 'Logout',
            subtitle: 'Current session only',
            iconColor: colorScheme.error,
            onTap: () => _handleLogout(context, false),
          ),
          const SizedBox(height: 12),
          _SettingsTile(
            icon: Icons.phonelink_erase_rounded,
            title: 'Logout All Devices',
            subtitle: 'All active sessions',
            iconColor: colorScheme.error,
            onTap: () => _handleLogout(context, true),
          ),
        ],
      ),
    );
  }

  Future<void> _showChangePinDialog(BuildContext context) async {
    final state = context.read<WalletState>();

    final currentPin = await showAppPinEntrySheet(
      context: context,
      title: 'Current PIN',
      subtitle: 'Enter your current 6-digit PIN.',
      onBiometricAuthenticate: state.authorizeWithBiometricSession,
      biometricReason: 'Verify your identity to change the Kanari PIN',
    );
    if (!context.mounted || currentPin == null) return;
    final usedBiometric = currentPin == appPinBiometricResult;

    if (!usedBiometric) {
      final currentPinValid = await state.verifyPin(currentPin);
      if (!context.mounted) return;

      if (!currentPinValid) {
        _showSettingsMessage(context, 'Invalid current PIN', isError: true);
        return;
      }
    }

    final newPin = await showAppPinEntrySheet(
      context: context,
      title: 'New PIN',
      subtitle: 'Enter a new 6-digit PIN.',
    );
    if (!context.mounted || newPin == null) return;

    final confirmPin = await showAppPinEntrySheet(
      context: context,
      title: 'Confirm PIN',
      subtitle: 'Enter the new PIN again to confirm.',
    );
    if (!context.mounted || confirmPin == null) return;

    if (newPin != confirmPin) {
      _showSettingsMessage(context, 'PINs do not match', isError: true);
      return;
    }

    final success = usedBiometric
        ? await state.changePinWithSession(newPin)
        : await state.changePin(currentPin, newPin);
    if (!context.mounted) return;

    _showSettingsMessage(
      context,
      success ? 'PIN changed successfully' : 'Failed to change PIN',
      isError: !success,
    );
  }

  void _showSettingsMessage(
    BuildContext context,
    String message, {
    required bool isError,
  }) {
    if (isError) {
      showAppErrorSnackBar(context, message);
    } else {
      showAppSuccessSnackBar(context, message);
    }
  }

  Future<void> _showSetup2faDialog(BuildContext context) async {
    final outerContext = context;
    final authClient = context.read<KanariAuthClient>();
    final email = authClient.userEmail;

    if (email == null || email.isEmpty) {
      showAppErrorSnackBar(context, 'Please login before configuring 2FA');
      return;
    }

    final authorized = await _authorizeSensitiveAction(
      context,
      title: 'Confirm Access',
      subtitle: 'Authenticate before showing 2FA secret and backup codes.',
    );
    if (!context.mounted || !authorized) return;

    final passwordController = TextEditingController();
    final codeController = TextEditingController();
    var isLoading = false;
    String? errorMessage;
    TwoFactorSetupResponse? setupResponse;

    await showAppModalSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (dialogContext) {
        return StatefulBuilder(
          builder: (context, setState) {
            Future<void> startSetup() async {
              setState(() {
                isLoading = true;
                errorMessage = null;
              });

              final response = await authClient.setup2fa(
                email: email,
                password: passwordController.text,
              );

              if (!context.mounted) return;
              setState(() {
                isLoading = false;
                if (response.success) {
                  setupResponse = response.data;
                } else {
                  errorMessage = response.error ?? 'Failed to start 2FA setup';
                }
              });
            }

            Future<void> confirmSetup() async {
              setState(() {
                isLoading = true;
                errorMessage = null;
              });

              final response = await authClient.enable2fa(
                email: email,
                password: passwordController.text,
                code: codeController.text.trim(),
              );

              if (!context.mounted) return;
              setState(() {
                isLoading = false;
                if (!response.success) {
                  errorMessage = response.error ?? 'Failed to enable 2FA';
                }
              });

              if (response.success && outerContext.mounted) {
                Navigator.of(dialogContext).pop();
                showAppSuccessSnackBar(
                  outerContext,
                  'Two-factor authentication enabled',
                );
              }
            }

            return SafeArea(
              child: SingleChildScrollView(
                padding: const EdgeInsets.fromLTRB(20, 12, 20, 28),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    AppPanel(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          Row(
                            children: [
                              Icon(
                                setupResponse == null
                                    ? Icons.shield_moon_rounded
                                    : Icons.verified_user_rounded,
                              ),
                              const SizedBox(width: 12),
                              Expanded(
                                child: Text(
                                  setupResponse == null
                                      ? 'Set Up 2FA'
                                      : 'Confirm 2FA Setup',
                                  style: Theme.of(context).textTheme.titleLarge
                                      ?.copyWith(fontWeight: FontWeight.w800),
                                ),
                              ),
                            ],
                          ),
                          const SizedBox(height: 12),
                          Text(
                            setupResponse == null
                                ? 'Enter your password to generate a QR code and backup codes.'
                                : 'Scan the QR code with your authenticator app, then enter the current 6-digit code.',
                          ),
                          const SizedBox(height: 16),
                          TextField(
                            controller: passwordController,
                            obscureText: true,
                            enabled: !isLoading && setupResponse == null,
                            decoration: const InputDecoration(
                              labelText: 'Password',
                              prefixIcon: Icon(Icons.lock_rounded),
                            ),
                          ),
                          if (setupResponse != null) ...[
                            const SizedBox(height: 20),
                            if (setupResponse!.otpauthUrl != null &&
                                setupResponse!.otpauthUrl!.isNotEmpty)
                              Center(
                                child: Container(
                                  padding: const EdgeInsets.all(14),
                                  decoration: BoxDecoration(
                                    color: Colors.white,
                                    borderRadius: BorderRadius.circular(24),
                                  ),
                                  child: QrImageView(
                                    data: setupResponse!.otpauthUrl!,
                                    size: 220,
                                    backgroundColor: Colors.white,
                                  ),
                                ),
                              ),
                            const SizedBox(height: 16),
                            Text(
                              'Manual secret',
                              style: Theme.of(context).textTheme.titleSmall
                                  ?.copyWith(fontWeight: FontWeight.w700),
                            ),
                            const SizedBox(height: 8),
                            Container(
                              padding: const EdgeInsets.all(12),
                              decoration: BoxDecoration(
                                color: Theme.of(
                                  context,
                                ).colorScheme.surfaceContainerHighest,
                                borderRadius: BorderRadius.circular(16),
                              ),
                              child: SelectableText(
                                setupResponse!.secret ?? '-',
                              ),
                            ),
                            const SizedBox(height: 16),
                            TextField(
                              controller: codeController,
                              keyboardType: TextInputType.number,
                              enabled: !isLoading,
                              decoration: const InputDecoration(
                                labelText: 'Authenticator code',
                                hintText: '123456',
                                prefixIcon: Icon(Icons.shield_rounded),
                              ),
                            ),
                            const SizedBox(height: 16),
                            Text(
                              'Backup codes',
                              style: Theme.of(context).textTheme.titleSmall
                                  ?.copyWith(fontWeight: FontWeight.w700),
                            ),
                            const SizedBox(height: 8),
                            Container(
                              padding: const EdgeInsets.all(12),
                              decoration: BoxDecoration(
                                color: Theme.of(
                                  context,
                                ).colorScheme.surfaceContainerHighest,
                                borderRadius: BorderRadius.circular(16),
                              ),
                              child: SelectableText(
                                (setupResponse!.backupCodes ?? const <String>[])
                                    .join('\n'),
                              ),
                            ),
                          ],
                          if (errorMessage != null) ...[
                            const SizedBox(height: 16),
                            AppErrorBanner(message: errorMessage!),
                          ],
                          const SizedBox(height: 20),
                          Row(
                            children: [
                              Expanded(
                                child: OutlinedButton(
                                  onPressed: isLoading
                                      ? null
                                      : () => Navigator.of(dialogContext).pop(),
                                  child: const Text('Cancel'),
                                ),
                              ),
                              const SizedBox(width: 12),
                              Expanded(
                                child: FilledButton(
                                  onPressed: isLoading
                                      ? null
                                      : (setupResponse == null
                                            ? startSetup
                                            : confirmSetup),
                                  child: isLoading
                                      ? const SizedBox(
                                          width: 18,
                                          height: 18,
                                          child: CircularProgressIndicator(
                                            strokeWidth: 2,
                                          ),
                                        )
                                      : Text(
                                          setupResponse == null
                                              ? 'Generate QR'
                                              : 'Enable 2FA',
                                        ),
                                ),
                              ),
                            ],
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            );
          },
        );
      },
    );
  }

  Future<void> _showDisable2faDialog(BuildContext context) async {
    final outerContext = context;
    final authClient = context.read<KanariAuthClient>();
    final email = authClient.userEmail;

    if (email == null || email.isEmpty) {
      showAppErrorSnackBar(context, 'Please login before disabling 2FA');
      return;
    }

    final authorized = await _authorizeSensitiveAction(
      context,
      title: 'Confirm Access',
      subtitle: 'Authenticate before changing two-factor settings.',
    );
    if (!context.mounted || !authorized) return;

    final passwordController = TextEditingController();
    final disableCodeController = TextEditingController();
    var isLoading = false;
    String? errorMessage;

    await showAppModalSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (dialogContext) {
        return StatefulBuilder(
          builder: (context, setState) {
            Future<void> disable2fa() async {
              setState(() {
                isLoading = true;
                errorMessage = null;
              });

              final response = await authClient.disable2fa(
                email: email,
                password: passwordController.text,
                code: disableCodeController.text.trim(),
              );

              if (!context.mounted) return;
              setState(() {
                isLoading = false;
                if (!response.success) {
                  errorMessage = response.error ?? 'Failed to disable 2FA';
                }
              });

              if (response.success && outerContext.mounted) {
                Navigator.of(dialogContext).pop();
                showAppSuccessSnackBar(
                  outerContext,
                  'Two-factor authentication disabled',
                );
              }
            }

            return SafeArea(
              child: SingleChildScrollView(
                padding: const EdgeInsets.fromLTRB(20, 12, 20, 28),
                child: AppPanel(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Row(
                        children: [
                          const Icon(Icons.shield_outlined),
                          const SizedBox(width: 12),
                          Expanded(
                            child: Text(
                              'Disable 2FA',
                              style: Theme.of(context).textTheme.titleLarge
                                  ?.copyWith(fontWeight: FontWeight.w800),
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 12),
                      const Text(
                        'Enter your password to remove authenticator-based login protection.',
                      ),
                      const SizedBox(height: 16),
                      TextField(
                        controller: passwordController,
                        obscureText: true,
                        enabled: !isLoading,
                        decoration: const InputDecoration(
                          labelText: 'Password',
                          prefixIcon: Icon(Icons.lock_rounded),
                        ),
                      ),
                      const SizedBox(height: 12),
                      TextField(
                        controller: disableCodeController,
                        keyboardType: TextInputType.number,
                        enabled: !isLoading,
                        decoration: const InputDecoration(
                          labelText: 'Current authenticator code',
                          hintText: '123456',
                          prefixIcon: Icon(Icons.shield_rounded),
                        ),
                      ),
                      if (errorMessage != null) ...[
                        const SizedBox(height: 16),
                        AppErrorBanner(message: errorMessage!),
                      ],
                      const SizedBox(height: 20),
                      Row(
                        children: [
                          Expanded(
                            child: OutlinedButton(
                              onPressed: isLoading
                                  ? null
                                  : () => Navigator.of(dialogContext).pop(),
                              child: const Text('Cancel'),
                            ),
                          ),
                          const SizedBox(width: 12),
                          Expanded(
                            child: FilledButton(
                              onPressed: isLoading ? null : disable2fa,
                              child: isLoading
                                  ? const SizedBox(
                                      width: 18,
                                      height: 18,
                                      child: CircularProgressIndicator(
                                        strokeWidth: 2,
                                      ),
                                    )
                                  : const Text('Disable'),
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
        );
      },
    );
  }

  Future<void> _handleLogout(BuildContext context, bool logoutAll) async {
    final authClient = context.read<KanariAuthClient>();
    final walletState = context.read<WalletState>();

    final confirmed = await showDialog<bool>(
      context: context,
      builder: (_) => AppConfirmationDialog(
        icon: logoutAll ? Icons.phonelink_erase_rounded : Icons.logout_rounded,
        title: logoutAll ? 'Logout All Devices?' : 'Logout?',
        content: logoutAll
            ? 'This will log out all active sessions on all devices.'
            : 'This will log out your current session.',
        confirmLabel: 'Logout',
        isDestructive: true,
      ),
    );

    if (confirmed != true || !context.mounted) return;

    Object? remoteLogoutError;
    try {
      if (logoutAll) {
        await authClient.logoutAll();
      } else {
        await authClient.logout();
      }
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
          ? (logoutAll
                ? 'Logged out from all devices'
                : 'Logged out successfully')
          : 'Logged out locally. Server session will expire automatically.',
    );
  }

  Future<bool> _authorizeSensitiveAction(
    BuildContext context, {
    required String title,
    required String subtitle,
  }) async {
    final walletState = context.read<WalletState>();
    return showAppPinVerificationSheet(
      context: context,
      onVerify: walletState.verifyPin,
      lockRemaining: walletState.pinLockRemaining,
      title: title,
      subtitle: subtitle,
    );
  }
}

class _ThemeModeCard extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final themeModeProvider = context.watch<ThemeModeProvider>();

    return AppPanel(
      padding: EdgeInsets.zero,
      child: RadioGroup<ThemeMode>(
        groupValue: themeModeProvider.themeMode,
        onChanged: (value) {
          if (value != null) {
            themeModeProvider.setThemeMode(value);
          }
        },
        child: Column(
          children: const [
            RadioListTile<ThemeMode>(
              value: ThemeMode.system,
              title: Text('System'),
              subtitle: Text('Follow device appearance'),
            ),
            Divider(height: 1),
            RadioListTile<ThemeMode>(
              value: ThemeMode.light,
              title: Text('Light'),
              subtitle: Text('Always use the light theme'),
            ),
            Divider(height: 1),
            RadioListTile<ThemeMode>(
              value: ThemeMode.dark,
              title: Text('Dark'),
              subtitle: Text('Always use the dark theme'),
            ),
          ],
        ),
      ),
    );
  }
}

class _BiometricSettingsTile extends StatefulWidget {
  const _BiometricSettingsTile();

  @override
  State<_BiometricSettingsTile> createState() => _BiometricSettingsTileState();
}

class _BiometricSettingsTileState extends State<_BiometricSettingsTile> {
  final LocalAuthentication _localAuth = LocalAuthentication();
  bool _enabled = false;
  bool _supported = false;
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    var supported = false;
    try {
      supported =
          await _localAuth.isDeviceSupported() &&
          await _localAuth.canCheckBiometrics;
    } catch (_) {
      supported = false;
    }

    final enabled = supported && await WalletStorage.isBiometricEnabled();

    if (!mounted) return;
    setState(() {
      _supported = supported;
      _enabled = enabled;
      _loading = false;
    });
  }

  Future<void> _setEnabled(bool value) async {
    if (_loading) return;
    final walletState = context.read<WalletState>();

    if (value && !_supported) {
      _showMessage('Biometric authentication is not available', isError: true);
      return;
    }

    if (!value) {
      await walletState.disableBiometricUnlock();
      if (!mounted) return;
      setState(() => _enabled = false);
      _showMessage('Biometric authentication disabled', isError: false);
      return;
    }

    try {
      final verified = await _localAuth.authenticate(
        localizedReason: 'Enable biometric authentication for Kanari Wallet',
        biometricOnly: true,
        persistAcrossBackgrounding: true,
      );

      if (!mounted) return;

      if (!verified) {
        _showMessage('Biometric setup was cancelled', isError: true);
        return;
      }

      final enabled = await walletState.enableBiometricUnlock();
      if (!mounted) return;

      if (!enabled) {
        setState(() => _enabled = false);
        _showMessage(
          'Biometric unlock needs an active wallet session on this device',
          isError: true,
        );
        return;
      }

      setState(() => _enabled = true);
      _showMessage('Biometric authentication enabled', isError: false);
    } catch (_) {
      if (!mounted) return;
      await walletState.disableBiometricUnlock();
      setState(() => _enabled = false);
      _showMessage('Biometric authentication failed', isError: true);
    }
  }

  void _showMessage(String message, {required bool isError}) {
    if (isError) {
      showAppErrorSnackBar(context, message);
    } else {
      showAppSuccessSnackBar(context, message);
    }
  }

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final subtitle = _supported
        ? 'Use fingerprint or face unlock for wallet actions'
        : 'Biometric authentication is not available on this device';

    return Material(
      color: colorScheme.surfaceContainerHigh,
      borderRadius: BorderRadius.circular(20),
      child: Padding(
        padding: const EdgeInsets.all(18),
        child: Row(
          children: [
            Icon(
              Icons.fingerprint_rounded,
              color: _supported
                  ? colorScheme.primary
                  : colorScheme.onSurfaceVariant,
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Biometric Authentication',
                    style: Theme.of(context).textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    subtitle,
                    style: Theme.of(context).textTheme.bodySmall?.copyWith(
                      color: colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            if (_loading)
              const SizedBox.square(
                dimension: 24,
                child: CircularProgressIndicator(strokeWidth: 2),
              )
            else
              Switch(
                value: _enabled,
                onChanged: _supported ? _setEnabled : null,
              ),
          ],
        ),
      ),
    );
  }
}

class _SettingsTile extends StatelessWidget {
  final IconData icon;
  final String title;
  final String subtitle;
  final Color? iconColor;
  final VoidCallback onTap;

  const _SettingsTile({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
    this.iconColor,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Material(
      color: colorScheme.surfaceContainerHigh,
      borderRadius: BorderRadius.circular(20),
      child: InkWell(
        borderRadius: BorderRadius.circular(20),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.all(18),
          child: Row(
            children: [
              Icon(icon, color: iconColor ?? colorScheme.primary),
              const SizedBox(width: 16),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      title,
                      style: Theme.of(context).textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      subtitle,
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color: colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              Icon(
                Icons.chevron_right_rounded,
                color: colorScheme.onSurfaceVariant,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
