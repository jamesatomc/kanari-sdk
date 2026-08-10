import 'package:flutter/material.dart';

import '../../kanaricurve.dart';
import '../../kanari_wallet.dart';
import 'app_ui.dart';

class AppCurveSelectionSheet extends StatefulWidget {
  final String title;
  final String subtitle;
  final String confirmLabel;
  final void Function(KanariCurve curve, String derivationPath) onConfirm;

  const AppCurveSelectionSheet({
    super.key,
    required this.onConfirm,
    this.title = 'Wallet Options',
    this.subtitle = 'Select the cryptographic curve for your new wallet.',
    this.confirmLabel = 'Generate Wallet',
  });

  @override
  State<AppCurveSelectionSheet> createState() => _AppCurveSelectionSheetState();
}

class _AppCurveSelectionSheetState extends State<AppCurveSelectionSheet> {
  KanariCurve _selectedCurve = KanariCurve.ed25519;
  final _derivationPathController = TextEditingController(
    text: KanariWallet.defaultDerivationPath,
  );

  @override
  void dispose() {
    _derivationPathController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Padding(
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
            widget.title,
            style: theme.textTheme.headlineSmall?.copyWith(
              fontWeight: FontWeight.bold,
            ),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 24),
          AppFormSection(
            title: 'Wallet Options',
            subtitle: widget.subtitle,
            children: [
              AppDropdownField<KanariCurve>(
                initialValue: _selectedCurve,
                label: 'Curve Type',
                items: KanariCurve.values.map((curve) {
                  return DropdownMenuItem(
                    value: curve,
                    child: Text(curve.name, overflow: TextOverflow.ellipsis),
                  );
                }).toList(),
                onChanged: (val) => setState(() => _selectedCurve = val!),
              ),
              if (!_selectedCurve.isPostQuantum) ...[
                const SizedBox(height: AppUiTokens.cardPadding),
                AppTextInput(
                  controller: _derivationPathController,
                  label: 'HD Path',
                  hintText: KanariWallet.defaultDerivationPath,
                ),
              ],
            ],
          ),
          const SizedBox(height: 24),
          FilledButton(
            style: FilledButton.styleFrom(
              minimumSize: const Size(double.infinity, 56),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(20),
              ),
            ),
            onPressed: () {
              final derivationPath = _selectedCurve.isPostQuantum
                  ? KanariWallet.defaultDerivationPath
                  : _derivationPathController.text.trim();
              if (!_selectedCurve.isPostQuantum &&
                  !KanariWallet.isValidDerivationPath(derivationPath)) {
                showAppErrorSnackBar(
                  context,
                  "Invalid HD path. Example: ${KanariWallet.defaultDerivationPath}",
                );
                return;
              }
              widget.onConfirm(_selectedCurve, derivationPath);
            },
            child: Text(
              widget.confirmLabel,
              style: const TextStyle(fontSize: 16, fontWeight: FontWeight.bold),
            ),
          ),
        ],
      ),
    );
  }
}

class AppImportWalletSheet extends StatefulWidget {
  final void Function(
    String data,
    KanariCurve curve,
    bool isMnemonic,
    String derivationPath,
  )
  onContinue;

  const AppImportWalletSheet({super.key, required this.onContinue});

  @override
  State<AppImportWalletSheet> createState() => _AppImportWalletSheetState();
}

class _AppImportWalletSheetState extends State<AppImportWalletSheet>
    with SingleTickerProviderStateMixin {
  late TabController _tabController;
  final _dataController = TextEditingController();
  final _derivationPathController = TextEditingController(
    text: KanariWallet.defaultDerivationPath,
  );
  KanariCurve _curve = KanariCurve.ed25519;

  @override
  void initState() {
    super.initState();
    _tabController = TabController(length: 2, vsync: this);
  }

  @override
  void dispose() {
    _tabController.dispose();
    _dataController.dispose();
    _derivationPathController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Padding(
      padding: EdgeInsets.fromLTRB(
        24,
        8,
        24,
        MediaQuery.of(context).viewInsets.bottom + 24,
      ),
      child: SizedBox(
        height: 460,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              'Import Wallet',
              style: theme.textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.bold,
              ),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 16),
            Expanded(
              child: AppTabPageSection(
                controller: _tabController,
                tabs: const [Text('Private Key'), Text('Mnemonic')],
                viewPadding: const EdgeInsets.only(top: 24),
                children: [
                  _buildImportTab(
                    context,
                    isMnemonic: false,
                    hintText: 'Enter your private key',
                  ),
                  _buildImportTab(
                    context,
                    isMnemonic: true,
                    hintText: 'Enter your 12-word mnemonic phrase',
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildImportTab(
    BuildContext context, {
    required bool isMnemonic,
    required String hintText,
  }) {
    return SingleChildScrollView(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          AppFormSection(
            title: isMnemonic
                ? 'Import From Mnemonic'
                : 'Import From Private Key',
            subtitle: isMnemonic
                ? 'Paste the recovery phrase for the wallet you want to restore.'
                : 'Paste the private key for the wallet you want to restore.',
            children: [
              AppDropdownField<KanariCurve>(
                initialValue: _curve,
                label: 'Curve Type',
                items: KanariCurve.values
                    .map((c) => DropdownMenuItem(value: c, child: Text(c.name)))
                    .toList(),
                onChanged: (v) => setState(() => _curve = v!),
              ),
              const SizedBox(height: AppUiTokens.cardPadding),
              AppTextInput(
                controller: _dataController,
                label: isMnemonic ? 'Mnemonic Phrase' : 'Private Key',
                hintText: hintText,
                maxLines: 3,
              ),
              if (isMnemonic && !_curve.isPostQuantum) ...[
                const SizedBox(height: AppUiTokens.cardPadding),
                AppTextInput(
                  controller: _derivationPathController,
                  label: 'HD Path',
                  hintText: KanariWallet.defaultDerivationPath,
                ),
              ],
            ],
          ),
          const SizedBox(height: 24),
          AppWideButton(
            onPressed: () => _continueImport(context, isMnemonic),
            icon: isMnemonic ? Icons.key_rounded : Icons.vpn_key_rounded,
            label: 'Continue',
            style: AppWideButtonStyle.primary,
          ),
        ],
      ),
    );
  }

  void _continueImport(BuildContext context, bool isMnemonic) {
    if (_dataController.text.trim().isEmpty) {
      showAppErrorSnackBar(
        context,
        isMnemonic
            ? 'Please enter your mnemonic phrase'
            : 'Please enter your private key',
      );
      return;
    }

    if (isMnemonic && _curve.isPostQuantum) {
      showAppErrorSnackBar(
        context,
        'Post-quantum curves do not support mnemonic HD paths yet.',
      );
      return;
    }

    if (isMnemonic &&
        !KanariWallet.isValidDerivationPath(_derivationPathController.text)) {
      showAppErrorSnackBar(
        context,
        "Invalid HD path. Example: ${KanariWallet.defaultDerivationPath}",
      );
      return;
    }

    Navigator.pop(context);
    widget.onContinue(
      _dataController.text.trim(),
      _curve,
      isMnemonic,
      _derivationPathController.text.trim(),
    );
  }
}
