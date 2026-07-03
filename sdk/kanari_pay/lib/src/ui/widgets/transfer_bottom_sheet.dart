import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:provider/provider.dart';

import '../../core/token_utils.dart' as token_utils;
import '../../models/account.dart';
import '../../modules/transactions/constants.dart';
import '../../providers/wallet_provider.dart';
import 'app_ui.dart';
import 'token_logo.dart';

class TransferBottomSheet extends StatefulWidget {
  final String? prefilledAddress;

  const TransferBottomSheet({super.key, this.prefilledAddress});

  @override
  State<TransferBottomSheet> createState() => _TransferBottomSheetState();
}

class _TransferBottomSheetState extends State<TransferBottomSheet> {
  late final TextEditingController _recipientController;
  late final TextEditingController _amountController;
  String _selectedTokenType = '';

  @override
  void initState() {
    super.initState();
    _recipientController = TextEditingController(text: widget.prefilledAddress);
    _amountController = TextEditingController();
  }

  @override
  void dispose() {
    _recipientController.dispose();
    _amountController.dispose();
    super.dispose();
  }

  Future<String?> _scanQrCode(BuildContext context) async {
    return Navigator.push<String>(
      context,
      MaterialPageRoute(builder: (context) => const QRScannerScreen()),
    );
  }

  DropdownMenuItem<String> _buildTokenItem(TokenBalance token) {
    return DropdownMenuItem(
      value: token.tokenType,
      child: Row(
        children: [
          TokenLogo(
            tokenType: token.tokenType,
            symbol: token.symbol,
            size: 28,
            logoUrl: token.iconUrl,
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              '${token.symbol} (${token_utils.formatDisplayAmount(token.amount, token.decimals)})',
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }

  int get _nativeGasReserveBaseUnits =>
      TransactionConstants.defaultGasLimit *
      TransactionConstants.defaultGasPrice;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;
    final isSmallScreen = MediaQuery.of(context).size.width < 360;
    final walletState = context.watch<WalletState>();
    final selectedTokenValue = _selectedTokenType.isEmpty
        ? WalletState.kanariTokenType
        : _selectedTokenType;

    final kanariToken =
        walletState.kanariTokenBalance ??
        token_utils.buildKanariTokenBalance(walletState.kanariBalance);

    final tokenItems = [
      DropdownMenuItem(
        value: WalletState.kanariTokenType,
        child: Row(
          children: [
            TokenLogo(
              tokenType: kanariToken.tokenType,
              symbol: kanariToken.symbol,
              size: 28,
              logoUrl: kanariToken.iconUrl,
            ),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                'KANARI (${token_utils.formatDisplayAmount(walletState.kanariBalance, token_utils.kanariDecimals)})',
                style: const TextStyle(fontWeight: FontWeight.bold),
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
        ),
      ),
      ...walletState.tokenBalances
          .where((token) => !token_utils.isKanariToken(token))
          .map(_buildTokenItem),
    ];

    return Material(
      borderRadius: const BorderRadius.vertical(top: Radius.circular(24)),
      color: colorScheme.surface,
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          24,
          8,
          24,
          MediaQuery.of(context).viewInsets.bottom + 24,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              'Transfer Assets',
              style: theme.textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.bold,
                color: colorScheme.onSurface,
              ),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 24),
            TextField(
              controller: _recipientController,
              style: const TextStyle(fontFamily: 'monospace', fontSize: 13),
              decoration: InputDecoration(
                labelText: 'Recipient Address',
                hintText: '0x...',
                helperText: 'Must be exactly 64 hex characters',
                suffixIcon: IconButton(
                  icon: const Icon(Icons.qr_code_scanner_rounded),
                  onPressed: () async {
                    final scannedAddress = await _scanQrCode(context);
                    if (scannedAddress != null) {
                      setState(
                        () => _recipientController.text = scannedAddress,
                      );
                    }
                  },
                ),
              ),
            ),
            SizedBox(height: isSmallScreen ? 12 : 16),
            DropdownButtonFormField<String>(
              initialValue: selectedTokenValue,
              isExpanded: true,
              decoration: const InputDecoration(labelText: 'Asset to send'),
              items: tokenItems,
              onChanged: (value) {
                if (value == null) return;
                setState(() {
                  _selectedTokenType = value;
                  _amountController.clear();
                });
              },
            ),
            SizedBox(height: isSmallScreen ? 12 : 16),
            TextField(
              controller: _amountController,
              keyboardType: const TextInputType.numberWithOptions(
                decimal: true,
              ),
              decoration: InputDecoration(
                labelText: 'Amount',
                prefixIcon: const Icon(Icons.account_balance_wallet_rounded),
                suffixIcon: TextButton(
                  onPressed: () =>
                      _fillMaxAmount(walletState, selectedTokenValue),
                  child: const Text('MAX'),
                ),
              ),
            ),
            const SizedBox(height: 32),
            FilledButton(
              style: FilledButton.styleFrom(
                minimumSize: const Size(double.infinity, 56),
                shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(20),
                ),
              ),
              onPressed: () async => _handleTransfer(context, walletState),
              child: const Text(
                'Send Assets',
                style: TextStyle(fontSize: 16, fontWeight: FontWeight.bold),
              ),
            ),
          ],
        ),
      ),
    );
  }

  void _fillMaxAmount(WalletState walletState, String selectedTokenValue) {
    if (selectedTokenValue == WalletState.kanariTokenType) {
      final spendable = walletState.kanariBalance - _nativeGasReserveBaseUnits;
      _amountController.text = token_utils
          .displayAmountFromBaseUnits(
            spendable > 0 ? spendable : 0,
            token_utils.kanariDecimals,
          )
          .toStringAsFixed(6);
      return;
    }

    final token = walletState.tokenBalances.firstWhere(
      (item) => item.tokenType == selectedTokenValue,
    );
    final maxAmount = token_utils.displayAmountFromBaseUnits(
      token.amount,
      token.decimals,
    );
    _amountController.text = maxAmount.toStringAsFixed(
      token.decimals < 6 ? token.decimals : 6,
    );
  }

  Future<void> _handleTransfer(
    BuildContext context,
    WalletState walletState,
  ) async {
    final recipient = _recipientController.text.trim();
    final amount = double.tryParse(_amountController.text.trim()) ?? 0;
    if (recipient.isEmpty || amount <= 0) {
      _showMessage(
        context,
        'Enter a valid recipient and amount.',
        isError: true,
      );
      return;
    }

    final cleanAddress = recipient.startsWith('0x')
        ? recipient.substring(2)
        : recipient;
    if (cleanAddress.length != 64 ||
        !RegExp(r'^[0-9a-fA-F]+$').hasMatch(cleanAddress)) {
      _showMessage(context, 'Invalid address format.', isError: true);
      return;
    }

    final selectedTokenValue = _selectedTokenType.isEmpty
        ? WalletState.kanariTokenType
        : _selectedTokenType;
    final selectedToken = selectedTokenValue == WalletState.kanariTokenType
        ? null
        : walletState.tokenBalances.firstWhere(
            (token) => token.tokenType == selectedTokenValue,
          );
    final decimals = selectedToken?.decimals ?? token_utils.kanariDecimals;
    final rawAmount = token_utils.baseUnitsFromDisplayAmount(amount, decimals);
    final availableBaseUnits = _availableBaseUnits(
      walletState,
      selectedTokenValue,
    );
    if (rawAmount > availableBaseUnits) {
      _showMessage(context, 'Amount exceeds available balance.', isError: true);
      return;
    }
    if (selectedTokenValue != WalletState.kanariTokenType &&
        walletState.kanariBalance < _nativeGasReserveBaseUnits) {
      _showMessage(
        context,
        'Insufficient KANARI balance for gas.',
        isError: true,
      );
      return;
    }

    final authorized = await showAppPinVerificationSheet(
      context: context,
      onVerify: walletState.verifyPin,
      lockRemaining: walletState.pinLockRemaining,
      subtitle: 'Enter your 6-digit PIN to send this transaction.',
    );

    if (!context.mounted || !authorized) {
      return;
    }

    Navigator.pop(context);

    String? result;
    if (selectedTokenValue == WalletState.kanariTokenType) {
      result = await walletState.transfer(recipient, rawAmount);
    } else {
      result = await walletState.transferToken(
        recipient,
        selectedTokenValue,
        rawAmount,
      );
    }

    if (!context.mounted) {
      return;
    }

    _showMessage(
      context,
      result?.startsWith('Error:') == true ? result! : 'Transaction successful',
      isError: result?.startsWith('Error:') == true,
    );
  }

  int _availableBaseUnits(WalletState walletState, String selectedTokenValue) {
    if (selectedTokenValue == WalletState.kanariTokenType) {
      final spendable = walletState.kanariBalance - _nativeGasReserveBaseUnits;
      return spendable > 0 ? spendable : 0;
    }

    final token = walletState.tokenBalances.firstWhere(
      (item) => item.tokenType == selectedTokenValue,
    );
    return token.amount;
  }

  void _showMessage(
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
}

class QRScannerScreen extends StatefulWidget {
  const QRScannerScreen({super.key});

  @override
  State<QRScannerScreen> createState() => _QRScannerScreenState();
}

class _QRScannerScreenState extends State<QRScannerScreen> {
  late final MobileScannerController cameraController;
  bool isProcessing = false;

  @override
  void initState() {
    super.initState();
    cameraController = MobileScannerController();
  }

  @override
  void dispose() {
    cameraController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        foregroundColor: Colors.white,
        title: const Text('Scan QR Code'),
        actions: [
          IconButton(
            icon: const Icon(Icons.flashlight_on_rounded, color: Colors.white),
            onPressed: () => cameraController.toggleTorch(),
          ),
          IconButton(
            icon: const Icon(
              Icons.flip_camera_android_rounded,
              color: Colors.white,
            ),
            onPressed: () => cameraController.switchCamera(),
          ),
        ],
      ),
      body: Stack(
        children: [
          MobileScanner(
            controller: cameraController,
            onDetect: (capture) {
              if (isProcessing) {
                return;
              }

              for (final barcode in capture.barcodes) {
                if (barcode.rawValue == null) {
                  continue;
                }
                setState(() => isProcessing = true);
                Navigator.pop(context, barcode.rawValue);
                break;
              }
            },
          ),
          Center(
            child: Container(
              width: 250,
              height: 250,
              decoration: BoxDecoration(
                border: Border.all(color: colorScheme.primary, width: 3),
                borderRadius: BorderRadius.circular(24),
              ),
            ),
          ),
          Positioned(
            bottom: 48,
            left: 0,
            right: 0,
            child: Center(
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 8,
                ),
                decoration: BoxDecoration(
                  color: Colors.black54,
                  borderRadius: BorderRadius.circular(20),
                ),
                child: const Text(
                  'Align QR code within the frame',
                  style: TextStyle(color: Colors.white),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
