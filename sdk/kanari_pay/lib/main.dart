import 'dart:async';

import 'package:flutter/material.dart';
import 'package:kanari_crypto/kanari_crypto.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'theme.dart';
import 'src/client/auth_client.dart';
import 'src/models/environment.dart';
import 'src/providers/theme_mode_provider.dart';
import 'src/providers/wallet_provider.dart';
import 'src/ui/screens/login_screen.dart';
import 'src/ui/screens/register_screen.dart';
import 'src/ui/screens/setting_screen.dart';
import 'src/ui/screens/kanari_welcome_screen.dart';
import 'src/ui/widgets/kanari_bottom_nav.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  try {
    await initKanariCrypto();
    debugPrint('Kanari Crypto initialized');
  } catch (e) {
    debugPrint('Kanari Crypto init error: $e');
  }

  final authClient = KanariAuthClient(KanariEnvironment.dev.authUrl);
  await _restoreSession(authClient);

  final themeModeProvider = ThemeModeProvider();
  await themeModeProvider.initialize();

  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(create: (_) => authClient),
        ChangeNotifierProvider(create: (_) => themeModeProvider),
        ChangeNotifierProvider(create: (_) => WalletState()..initialize()),
      ],
      child: const KanariApp(),
    ),
  );
}

Future<void> _restoreSession(KanariAuthClient authClient) async {
  try {
    final prefs = await SharedPreferences.getInstance();

    final sessionId = prefs.getString('session_id');
    final userEmail = prefs.getString('user_email');
    final walletAddress = prefs.getString('wallet_address');

    if (sessionId != null &&
        sessionId.isNotEmpty &&
        userEmail != null &&
        userEmail.isNotEmpty &&
        walletAddress != null &&
        walletAddress.isNotEmpty) {
      debugPrint('Restoring session from SharedPreferences...');
      debugPrint('User: $userEmail');
      debugPrint('Wallet: $walletAddress');

      authClient.setSession(
        sessionId: sessionId,
        userEmail: userEmail,
        walletAddress: walletAddress,
      );

      debugPrint('Session restored locally for $userEmail');
      _validateSessionInBackground(authClient);
    } else {
      debugPrint('No saved session found in SharedPreferences');
    }
  } catch (e) {
    debugPrint('Session restore error: $e');
  }
}

Future<void> _validateSessionInBackground(KanariAuthClient authClient) async {
  try {
    debugPrint('Validating session in background...');
    final response = await authClient.validateSession();

    if (!response.success || !(response.data?.valid ?? false)) {
      debugPrint('Session validation failed - session may be expired');
      debugPrint('User can still access app, but may need to re-login later');
    } else {
      debugPrint('Session validated successfully');
    }
  } catch (e) {
    debugPrint('Background validation skipped: $e');
  }
}

Future<void> _saveSession(KanariAuthClient authClient) async {
  try {
    final prefs = await SharedPreferences.getInstance();

    if (authClient.sessionId != null &&
        authClient.userEmail != null &&
        authClient.walletAddress != null) {
      await prefs.setString('session_id', authClient.sessionId!);
      await prefs.setString('user_email', authClient.userEmail!);
      await prefs.setString('wallet_address', authClient.walletAddress!);
      debugPrint('Session saved for: ${authClient.userEmail}');
    } else {
      debugPrint('Cannot save session: missing required fields');
    }
  } catch (e) {
    debugPrint('Session save error: $e');
  }
}

class KanariApp extends StatefulWidget {
  const KanariApp({super.key});

  @override
  State<KanariApp> createState() => _KanariAppState();
}

class _KanariAppState extends State<KanariApp> with WidgetsBindingObserver {
  static const _backgroundLockDelay = Duration(seconds: 30);
  Timer? _lockTimer;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _lockTimer?.cancel();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (!mounted) return;

    switch (state) {
      case AppLifecycleState.inactive:
      case AppLifecycleState.paused:
      case AppLifecycleState.hidden:
        _lockTimer?.cancel();
        _lockTimer = Timer(_backgroundLockDelay, () {
          if (!mounted) return;
          final walletState = context.read<WalletState>();
          if (walletState.isUnlocked) {
            walletState.lockSession();
          }
        });
        break;
      case AppLifecycleState.resumed:
        _lockTimer?.cancel();
        break;
      case AppLifecycleState.detached:
        _lockTimer?.cancel();
        final walletState = context.read<WalletState>();
        if (walletState.isUnlocked) {
          walletState.lockSession();
        }
        break;
    }
  }

  @override
  Widget build(BuildContext context) {
    final themeMode = context.watch<ThemeModeProvider>().themeMode;
    final textTheme = createTextTheme(context, 'Inter', 'Inter');

    return MaterialApp(
      title: 'Kanari Wallet',
      debugShowCheckedModeBanner: false,
      themeMode: themeMode,
      theme: MaterialTheme(textTheme).light(),
      darkTheme: MaterialTheme(textTheme).dark(),
      initialRoute: '/',
      routes: {
        '/': (context) {
          final authClient = context.watch<KanariAuthClient>();
          final walletState = context.watch<WalletState>();
          final canEnterHome = walletState.hasWallet && walletState.isUnlocked;

          debugPrint(
            "Route '/' check: isAuthenticated=${authClient.isAuthenticated}, isUnlocked=${walletState.isUnlocked}, hasWallet=${walletState.hasWallet}",
          );

          if (canEnterHome) {
            debugPrint('Unlocked wallet -> Navigate to KanariBottomNav');
            return const KanariBottomNav();
          }

          debugPrint('Stay on WelcomeScreen');
          return const KanariWelcomeScreen();
        },
        '/login': (context) {
          final authClient = context.read<KanariAuthClient>();
          return KanariLoginScreen(
            authClient: authClient,
            onLoginSuccess: () async {
              await _saveSession(authClient);
              if (context.mounted) {
                Navigator.of(
                  context,
                ).pushNamedAndRemoveUntil('/', (route) => false);
              }
            },
          );
        },
        '/register': (context) {
          final authClient = context.read<KanariAuthClient>();
          return KanariRegisterScreen(
            authClient: authClient,
            onRegistrationSuccess: () async {
              await _saveSession(authClient);
              if (context.mounted) {
                Navigator.of(
                  context,
                ).pushNamedAndRemoveUntil('/', (route) => false);
              }
            },
          );
        },
        '/home': (context) => const KanariBottomNav(),
        '/settings': (context) => const SettingScreen(),
        '/escrow': (context) => const KanariBottomNav(currentIndex: 1),
      },
    );
  }
}
