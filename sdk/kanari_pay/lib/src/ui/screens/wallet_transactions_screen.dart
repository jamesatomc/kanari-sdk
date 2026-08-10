import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:kanari_pay/kanari_pay.dart';
import 'package:kanari_pay/src/providers/wallet_provider.dart';
import 'package:provider/provider.dart';

import '../widgets/app_ui.dart';

class WalletTransactionsScreen extends StatefulWidget {
  final String walletName;
  final String walletAddress;

  const WalletTransactionsScreen({
    super.key,
    required this.walletName,
    required this.walletAddress,
  });

  @override
  State<WalletTransactionsScreen> createState() =>
      _WalletTransactionsScreenState();
}

class _WalletTransactionsScreenState extends State<WalletTransactionsScreen> {
  late Future<List<TransactionDetails>> _future;

  @override
  void initState() {
    super.initState();
    _future = _loadTransactions();
  }

  Future<List<TransactionDetails>> _loadTransactions() async {
    final client = context.read<WalletState>().client;
    if (client == null) {
      throw Exception('Kanari RPC client is not initialized');
    }
    return client.getAllTransactions(account: widget.walletAddress, limit: 50);
  }

  Future<void> _refresh() async {
    setState(() {
      _future = _loadTransactions();
    });
    await _future;
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = theme.colorScheme;

    return Scaffold(
      appBar: AppBar(
        title: Text(widget.walletName),
        actions: [
          IconButton(
            onPressed: () {
              Clipboard.setData(ClipboardData(text: widget.walletAddress));
              showAppInfoSnackBar(context, 'Address copied');
            },
            icon: const Icon(Icons.copy_rounded),
            tooltip: 'Copy address',
          ),
        ],
      ),
      body: RefreshIndicator(
        onRefresh: _refresh,
        child: FutureBuilder<List<TransactionDetails>>(
          future: _future,
          builder: (context, snapshot) {
            if (snapshot.connectionState == ConnectionState.waiting) {
              return const Center(child: CircularProgressIndicator());
            }

            if (snapshot.hasError) {
              return _TxMessageState(
                icon: Icons.sync_problem_rounded,
                title: 'Could not load transactions',
                message: snapshot.error.toString(),
                actionLabel: 'Try Again',
                onAction: () => setState(() => _future = _loadTransactions()),
              );
            }

            final transactions = snapshot.data ?? const <TransactionDetails>[];
            if (transactions.isEmpty) {
              return _TxMessageState(
                icon: Icons.history_rounded,
                title: 'No transactions yet',
                message:
                    'This wallet does not have indexed transactions on the current network.',
                actionLabel: 'Refresh',
                onAction: () => setState(() => _future = _loadTransactions()),
              );
            }

            return ListView(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
              children: [
                _WalletAddressHeader(address: widget.walletAddress),
                const SizedBox(height: 16),
                Text(
                  'RECENT ACTIVITY',
                  style: theme.textTheme.labelMedium?.copyWith(
                    color: colors.secondary,
                  ),
                ),
                const SizedBox(height: 10),
                ...transactions.map(
                  (tx) => Padding(
                    padding: const EdgeInsets.only(bottom: 10),
                    child: _TransactionTile(
                      transaction: tx,
                      walletAddress: widget.walletAddress,
                      onTap: () => _showTransactionDetails(context, tx),
                    ),
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
  }

  void _showTransactionDetails(BuildContext context, TransactionDetails tx) {
    showModalBottomSheet(
      context: context,
      showDragHandle: true,
      isScrollControlled: true,
      builder: (context) => _TransactionDetailsSheet(transaction: tx),
    );
  }
}

class _WalletAddressHeader extends StatelessWidget {
  final String address;

  const _WalletAddressHeader({required this.address});

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return Container(
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: colors.surfaceContainerLowest,
        border: Border.all(color: colors.outlineVariant),
        borderRadius: BorderRadius.circular(18),
      ),
      child: Row(
        children: [
          Icon(Icons.account_balance_wallet_rounded, color: colors.secondary),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              _short(address),
              style: Theme.of(context).textTheme.titleMedium,
            ),
          ),
        ],
      ),
    );
  }
}

class _TransactionTile extends StatelessWidget {
  final TransactionDetails transaction;
  final String walletAddress;
  final VoidCallback onTap;

  const _TransactionTile({
    required this.transaction,
    required this.walletAddress,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    final isSuccess =
        transaction.status.toLowerCase().contains('success') ||
        transaction.status.toLowerCase() == 'executed';
    final sender = transaction.senderAddress ?? transaction.sender;
    final outgoing =
        sender.trim().toLowerCase() == walletAddress.trim().toLowerCase();

    return Material(
      color: colors.surfaceContainerLowest,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(18),
        side: BorderSide(color: colors.outlineVariant),
      ),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(18),
        child: Padding(
          padding: const EdgeInsets.all(14),
          child: Row(
            children: [
              CircleAvatar(
                backgroundColor: outgoing
                    ? colors.primaryContainer
                    : colors.secondaryContainer,
                foregroundColor: outgoing
                    ? colors.onPrimaryContainer
                    : colors.onSecondaryContainer,
                child: Icon(
                  outgoing
                      ? Icons.north_east_rounded
                      : Icons.south_west_rounded,
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      transaction.txType.isEmpty
                          ? 'Transaction'
                          : transaction.txType,
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                    const SizedBox(height: 4),
                    Text(
                      _short(transaction.hash),
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color: colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              Column(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [
                  _StatusPill(success: isSuccess, label: transaction.status),
                  const SizedBox(height: 6),
                  Text(
                    transaction.checkpointHeight == null
                        ? 'Pending'
                        : '#${transaction.checkpointHeight}',
                    style: Theme.of(context).textTheme.bodySmall?.copyWith(
                      color: colors.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _StatusPill extends StatelessWidget {
  final bool success;
  final String label;

  const _StatusPill({required this.success, required this.label});

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 5),
      decoration: BoxDecoration(
        color: success ? colors.primaryContainer : colors.errorContainer,
        borderRadius: BorderRadius.circular(99),
      ),
      child: Text(
        label.isEmpty ? (success ? 'Success' : 'Failed') : label,
        style: Theme.of(context).textTheme.labelSmall?.copyWith(
          color: success ? colors.onPrimaryContainer : colors.onErrorContainer,
        ),
      ),
    );
  }
}

class _TransactionDetailsSheet extends StatelessWidget {
  final TransactionDetails transaction;

  const _TransactionDetailsSheet({required this.transaction});

  @override
  Widget build(BuildContext context) {
    final maxHeight = MediaQuery.sizeOf(context).height * 0.82;

    return SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: maxHeight),
        child: SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(20, 0, 20, 24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                'Transaction Details',
                style: Theme.of(context).textTheme.headlineSmall,
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: 18),
              _DetailRow(
                label: 'Hash',
                value: transaction.hash,
                copyable: true,
                compactLongValue: true,
              ),
              _DetailRow(label: 'Status', value: transaction.status),
              _DetailRow(label: 'Type', value: transaction.txType),
              _DetailRow(
                label: 'Sender',
                value: transaction.senderAddress ?? transaction.sender,
                copyValue: transaction.sender,
                copyable: true,
                compactLongValue: true,
              ),
              if (transaction.senderAddress != null &&
                  transaction.senderAddress != transaction.sender)
                _DetailRow(
                  label: 'Signer',
                  value: transaction.sender,
                  copyable: true,
                  compactLongValue: true,
                ),
              if (transaction.checkpointHeight != null)
                _DetailRow(
                  label: 'Checkpoint',
                  value: '${transaction.checkpointHeight}',
                ),
              if (transaction.gasUsed != null)
                _DetailRow(label: 'Gas Used', value: '${transaction.gasUsed}'),
              _DetailRow(label: 'Nonce', value: '${transaction.nonce}'),
              _DetailRow(label: 'Gas Limit', value: '${transaction.gasLimit}'),
              _DetailRow(label: 'Gas Price', value: '${transaction.gasPrice}'),
              if (transaction.module != null)
                _DetailRow(label: 'Module', value: transaction.module!),
              if (transaction.function != null)
                _DetailRow(label: 'Function', value: transaction.function!),
            ],
          ),
        ),
      ),
    );
  }
}

class _DetailRow extends StatelessWidget {
  final String label;
  final String value;
  final String? copyValue;
  final bool copyable;
  final bool compactLongValue;

  const _DetailRow({
    required this.label,
    required this.value,
    this.copyValue,
    this.copyable = false,
    this.compactLongValue = false,
  });

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    final displayValue = compactLongValue ? _middleShort(value) : value;

    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 92,
            child: Text(
              label,
              style: Theme.of(
                context,
              ).textTheme.labelMedium?.copyWith(color: colors.onSurfaceVariant),
            ),
          ),
          Expanded(
            child: compactLongValue
                ? SelectableText(
                    displayValue,
                    style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                      fontFeatures: const [FontFeature.tabularFigures()],
                    ),
                    maxLines: 3,
                  )
                : SelectableText(
                    displayValue,
                    style: Theme.of(context).textTheme.bodyMedium,
                  ),
          ),
          if (copyable)
            IconButton(
              visualDensity: VisualDensity.compact,
              onPressed: () {
                Clipboard.setData(ClipboardData(text: copyValue ?? value));
                showAppInfoSnackBar(context, '$label copied');
              },
              icon: const Icon(Icons.copy_rounded, size: 18),
            ),
        ],
      ),
    );
  }
}

class _TxMessageState extends StatelessWidget {
  final IconData icon;
  final String title;
  final String message;
  final String actionLabel;
  final VoidCallback onAction;

  const _TxMessageState({
    required this.icon,
    required this.title,
    required this.message,
    required this.actionLabel,
    required this.onAction,
  });

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return ListView(
      padding: const EdgeInsets.all(24),
      children: [
        const SizedBox(height: 90),
        Icon(icon, size: 48, color: colors.secondary),
        const SizedBox(height: 18),
        Text(
          title,
          textAlign: TextAlign.center,
          style: Theme.of(context).textTheme.headlineSmall,
        ),
        const SizedBox(height: 8),
        Text(
          message,
          textAlign: TextAlign.center,
          style: Theme.of(
            context,
          ).textTheme.bodyMedium?.copyWith(color: colors.onSurfaceVariant),
        ),
        const SizedBox(height: 22),
        Center(
          child: FilledButton.icon(
            onPressed: onAction,
            icon: const Icon(Icons.refresh_rounded),
            label: Text(actionLabel),
          ),
        ),
      ],
    );
  }
}

String _short(String value) {
  if (value.length <= 18) return value;
  return '${value.substring(0, 10)}...${value.substring(value.length - 6)}';
}

String _middleShort(String value, {int head = 22, int tail = 18}) {
  if (value.length <= head + tail + 3) return value;
  return '${value.substring(0, head)}...${value.substring(value.length - tail)}';
}
