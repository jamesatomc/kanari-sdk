import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../../client/auth_client.dart';
import '../../kanaricurve.dart';
import '../../providers/wallet_provider.dart';
import '../widgets/app_ui.dart';

class KanariLoginScreen extends StatefulWidget {
  final KanariAuthClient authClient;
  final VoidCallback? onLoginSuccess;

  const KanariLoginScreen({
    super.key,
    required this.authClient,
    this.onLoginSuccess,
  });

  @override
  State<KanariLoginScreen> createState() => _KanariLoginScreenState();
}

class _KanariLoginScreenState extends State<KanariLoginScreen> {
  static const int _defaultKdfIterations = 120000;

  final _formKey = GlobalKey<FormState>();
  final _emailController = TextEditingController();
  final _passwordController = TextEditingController();
  final _totpController = TextEditingController();
  final _backupCodeController = TextEditingController();

  bool _isLoading = false;
  bool _obscurePassword = true;
  bool _requiresTwoFactor = false;
  String? _errorMessage;

  @override
  void dispose() {
    _emailController.dispose();
    _passwordController.dispose();
    _totpController.dispose();
    _backupCodeController.dispose();
    super.dispose();
  }

  Future<void> _handleLogin() async {
    if (!_formKey.currentState!.validate()) return;

    setState(() {
      _isLoading = true;
      _errorMessage = null;
    });

    if (_requiresTwoFactor &&
        _totpController.text.trim().isEmpty &&
        _backupCodeController.text.trim().isEmpty) {
      setState(() {
        _isLoading = false;
        _errorMessage = 'Please enter an authenticator code or a backup code.';
      });
      return;
    }

    try {
      final response = await widget.authClient.login(
        email: _emailController.text.trim(),
        password: _passwordController.text,
        totpCode: _totpController.text.trim().isEmpty
            ? null
            : _totpController.text.trim(),
        backupCode: _backupCodeController.text.trim().isEmpty
            ? null
            : _backupCodeController.text.trim(),
      );

      if (!mounted) return;

      setState(() {
        _isLoading = false;
      });

      if (response.success) {
        setState(() {
          _requiresTwoFactor = false;
        });
        final walletAddress = response.data?.walletAddress;
        var matchedLocalWallet = false;

        final walletState = context.read<WalletState>();

        if (walletAddress == null || walletAddress.isEmpty) {
          widget.authClient.clearSession();
          setState(() {
            _errorMessage = 'Login succeeded, but no account wallet was found.';
          });
          return;
        }

        if (response.data?.encryptedPrivateKey != null &&
            response.data!.encryptedPrivateKey!.isNotEmpty &&
            response.data?.curveType != null) {
          try {
            final privateKey = await _decryptPrivateKey(
              response.data!.encryptedPrivateKey!,
              _passwordController.text,
            );
            final curve = KanariCurve.fromString(response.data!.curveType!);
            if (!mounted) return;
            final hasPinSet = await walletState.hasPinSet();
            if (!mounted) return;
            String? pin;

            if (hasPinSet && !walletState.isUnlocked) {
              final authorized = await showAppPinVerificationSheet(
                context: context,
                onVerify: walletState.verifyPin,
                lockRemaining: walletState.pinLockRemaining,
                title: 'Unlock Wallet',
                subtitle:
                    'Enter your 6-digit PIN to import and unlock this wallet.',
              );
              if (!mounted || !authorized) {
                setState(() {
                  _errorMessage = 'Wallet import requires PIN verification.';
                });
                return;
              }
            } else if (!hasPinSet) {
              pin = await showAppPinEntrySheet(
                context: context,
                title: 'Set PIN',
                subtitle:
                    'Set a 6-digit PIN to secure your Kanari account wallet.',
              );
              if (!mounted || pin == null) return;
            }

            matchedLocalWallet = await walletState
                .upsertAuthenticatedWalletFromPrivateKey(
                  privateKey,
                  curve: curve,
                  expectedAddress: walletAddress,
                  pin: pin,
                  name: 'Kanari Account Wallet',
                );
          } catch (e) {
            if (!mounted) return;
            setState(() {
              _errorMessage = 'Wallet import failed: $e';
            });
          }
        } else {
          matchedLocalWallet = await walletState.syncWalletWithAddress(
            walletAddress,
          );
        }

        if (!mounted) return;
        if (!matchedLocalWallet) {
          widget.authClient.clearSession();
          setState(() {
            _errorMessage =
                _errorMessage ??
                'Login succeeded, but this wallet could not be loaded on this device.';
          });
          showAppErrorSnackBar(
            context,
            'Wallet not found for this account on this device.',
          );
          return;
        }

        showAppSuccessSnackBar(context, 'Login successful!');
        widget.onLoginSuccess?.call();
      } else {
        final error = response.error ?? 'Login failed';
        setState(() {
          _requiresTwoFactor = error.toLowerCase().contains('two-factor');
          _errorMessage = error;
        });
      }
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _isLoading = false;
        _errorMessage = 'Network error: $e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    Theme.of(context);

    return AppGradientScaffold(
      appBar: AppBar(title: const Text('Login'), centerTitle: true),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 520),
            child: Form(
              key: _formKey,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const SizedBox(height: 16),
                  const AuthHero(
                    icon: Icons.login_rounded,
                    title: 'Welcome Back',
                    subtitle:
                        'Sign in to access your Kanari wallet and synced sessions.',
                  ),
                  const SizedBox(height: 16),
                  AppPanel(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        const AppSectionTitle('Account'),
                        const SizedBox(height: 12),
                        TextFormField(
                          controller: _emailController,
                          keyboardType: TextInputType.emailAddress,
                          enabled: !_isLoading,
                          decoration: const InputDecoration(
                            labelText: 'Email',
                            hintText: 'user@example.com',
                            prefixIcon: Icon(Icons.alternate_email_rounded),
                          ),
                          validator: (value) {
                            if (value == null || value.isEmpty) {
                              return 'Please enter your email';
                            }
                            if (!value.contains('@')) {
                              return 'Please enter a valid email';
                            }
                            return null;
                          },
                        ),
                        const SizedBox(height: 16),
                        TextFormField(
                          controller: _passwordController,
                          obscureText: _obscurePassword,
                          enabled: !_isLoading,
                          decoration: InputDecoration(
                            labelText: 'Password',
                            hintText: 'Enter your password',
                            prefixIcon: const Icon(Icons.lock_rounded),
                            suffixIcon: IconButton(
                              icon: Icon(
                                _obscurePassword
                                    ? Icons.visibility_rounded
                                    : Icons.visibility_off_rounded,
                              ),
                              onPressed: () {
                                setState(() {
                                  _obscurePassword = !_obscurePassword;
                                });
                              },
                            ),
                          ),
                          validator: (value) {
                            if (value == null || value.isEmpty) {
                              return 'Please enter your password';
                            }
                            if (value.length < 8) {
                              return 'Password must be at least 8 characters';
                            }
                            return null;
                          },
                        ),
                        if (_requiresTwoFactor) ...[
                          const SizedBox(height: 16),
                          TextFormField(
                            controller: _totpController,
                            keyboardType: TextInputType.number,
                            enabled: !_isLoading,
                            decoration: const InputDecoration(
                              labelText: 'Authenticator code',
                              hintText: '123456',
                              prefixIcon: Icon(Icons.shield_rounded),
                            ),
                          ),
                          const SizedBox(height: 16),
                          TextFormField(
                            controller: _backupCodeController,
                            textCapitalization: TextCapitalization.characters,
                            enabled: !_isLoading,
                            decoration: const InputDecoration(
                              labelText: 'Backup code',
                              hintText: 'Optional backup code',
                              prefixIcon: Icon(Icons.key_rounded),
                            ),
                          ),
                          const SizedBox(height: 8),
                          Text(
                            'Enter either a 6-digit authenticator code or a backup code.',
                            style: Theme.of(context).textTheme.bodySmall,
                          ),
                        ],
                        if (_errorMessage != null) ...[
                          const SizedBox(height: 16),
                          AppErrorBanner(message: _errorMessage!),
                        ],
                        const SizedBox(height: 24),
                        AppWideButton(
                          onPressed: _isLoading ? null : _handleLogin,
                          icon: Icons.login_rounded,
                          label: 'Login',
                          child: _isLoading
                              ? const SizedBox(
                                  width: 20,
                                  height: 20,
                                  child: CircularProgressIndicator(
                                    strokeWidth: 2,
                                  ),
                                )
                              : const Text('Login'),
                        ),
                        const SizedBox(height: 12),
                        AppWideButton(
                          onPressed: _isLoading
                              ? null
                              : () => Navigator.pushNamed(context, '/register'),
                          icon: Icons.person_add_rounded,
                          label: "Don't have an account? Register",
                          style: AppWideButtonStyle.text,
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  Future<String> _decryptPrivateKey(
    String encryptedPayload,
    String password,
  ) async {
    final payload = jsonDecode(encryptedPayload) as Map<String, dynamic>;
    final encryptedBytes = base64Decode(payload['ciphertext'] as String);
    final nonce = base64Decode(payload['nonce'] as String);
    final salt = base64Decode(payload['salt'] as String);
    final version = (payload['version'] as num?)?.toInt() ?? 1;

    if (encryptedBytes.length < 16) {
      throw Exception('Encrypted payload is invalid');
    }

    final keyBytes = version == 2
        ? await _deriveArgon2idKey(password, salt, payload)
        : await _deriveLegacyKey(
            password,
            salt,
            (payload['iterations'] as num?)?.toInt() ?? _defaultKdfIterations,
          );
    final algorithm = AesGcm.with256bits();
    final cipherText = encryptedBytes.sublist(0, encryptedBytes.length - 16);
    final macBytes = encryptedBytes.sublist(encryptedBytes.length - 16);
    final secretBox = SecretBox(cipherText, nonce: nonce, mac: Mac(macBytes));

    final clearText = await algorithm.decrypt(
      secretBox,
      secretKey: SecretKey(keyBytes),
      aad: version == 2 ? _argon2idAad(payload) : const <int>[],
    );

    return utf8.decode(clearText);
  }

  Future<List<int>> _deriveArgon2idKey(
    String password,
    List<int> salt,
    Map<String, dynamic> payload,
  ) async {
    final kdf = payload['kdf'] as String?;
    if (kdf != 'argon2id') {
      throw Exception('Unsupported encrypted wallet KDF');
    }

    final memoryKiB = (payload['memory_kib'] as num?)?.toInt() ?? 65536;
    final timeCost = (payload['time_cost'] as num?)?.toInt() ?? 3;
    final parallelism = (payload['parallelism'] as num?)?.toInt() ?? 1;

    final key = await Argon2id(
      memory: memoryKiB,
      iterations: timeCost,
      parallelism: parallelism,
      hashLength: 32,
    ).deriveKey(secretKey: SecretKey(utf8.encode(password)), nonce: salt);

    return key.extractBytes();
  }

  List<int> _argon2idAad(Map<String, dynamic> payload) {
    final memoryKiB = (payload['memory_kib'] as num?)?.toInt() ?? 65536;
    final timeCost = (payload['time_cost'] as num?)?.toInt() ?? 3;
    final parallelism = (payload['parallelism'] as num?)?.toInt() ?? 1;
    return utf8.encode(
      'kanari-auth-key-v2:argon2id:m=$memoryKiB:t=$timeCost:p=$parallelism',
    );
  }

  Future<List<int>> _deriveLegacyKey(
    String password,
    List<int> salt,
    int iterations,
  ) async {
    final sha256 = Sha256();
    var block = await sha256.hash([...utf8.encode(password), ...salt]);

    for (var i = 1; i < iterations; i++) {
      block = await sha256.hash([
        ...block.bytes,
        ...utf8.encode(password),
        ...salt,
      ]);
    }

    return block.bytes;
  }
}
