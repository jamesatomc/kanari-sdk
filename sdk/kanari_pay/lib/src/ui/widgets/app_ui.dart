import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:kanari_pay/src/wallet_storage.dart';
import 'package:kanari_pay/theme.dart';
import 'package:local_auth/local_auth.dart';

const String appPinBiometricResult = '__biometric_authenticated__';

abstract final class AppUiTokens {
  static const double panelRadius = 18;
  static const double cardRadius = 16;
  static const double pillRadius = 999;

  static const double cardPadding = 16;
  static const double compactSpacing = 4;
  static const double contentSpacing = 8;
  static const double sectionSpacing = 12;

  static const double badgeHorizontalPadding = 12;
  static const double badgeVerticalPadding = 6;

  static const double subtleBorderAlpha = 0.16;
  static const double selectedBorderAlpha = 0.28;
  static const double selectedFillAlpha = 0.55;
}

Future<T?> showAppModalSheet<T>({
  required BuildContext context,
  required WidgetBuilder builder,
  bool isScrollControlled = true,
  bool useSafeArea = true,
  bool showDragHandle = false,
  bool isDismissible = true,
  bool enableDrag = true,
  ShapeBorder? shape,
}) {
  return showModalBottomSheet<T>(
    context: context,
    isScrollControlled: isScrollControlled,
    useSafeArea: useSafeArea,
    showDragHandle: showDragHandle,
    isDismissible: isDismissible,
    enableDrag: enableDrag,
    backgroundColor: Theme.of(context).colorScheme.surfaceContainerLowest,
    shape:
        shape ??
        (showDragHandle
            ? const RoundedRectangleBorder(
                borderRadius: BorderRadius.vertical(top: Radius.circular(24)),
              )
            : null),
    builder: builder,
  );
}

class AppGradientScaffold extends StatelessWidget {
  final PreferredSizeWidget? appBar;
  final Widget body;
  final Color? backgroundColor;

  const AppGradientScaffold({
    super.key,
    this.appBar,
    required this.body,
    this.backgroundColor,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Scaffold(
      backgroundColor: backgroundColor ?? colorScheme.surface,
      appBar: appBar,
      body: SafeArea(child: body),
    );
  }
}

class AppPanel extends StatelessWidget {
  final Widget child;
  final EdgeInsetsGeometry padding;

  const AppPanel({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.all(16),
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Container(
      padding: padding,
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerLowest,
        borderRadius: BorderRadius.circular(AppUiTokens.panelRadius),
        border: Border.all(color: colorScheme.outlineVariant),
      ),
      child: child,
    );
  }
}

class AppSectionTitle extends StatelessWidget {
  final String title;

  const AppSectionTitle(this.title, {super.key});

  @override
  Widget build(BuildContext context) {
    return Text(
      title,
      style: Theme.of(
        context,
      ).textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w700),
    );
  }
}

class AuthHero extends StatelessWidget {
  final IconData icon;
  final String title;
  final String subtitle;

  const AuthHero({
    super.key,
    required this.icon,
    required this.title,
    required this.subtitle,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return Column(
      children: [
        Container(
          padding: const EdgeInsets.all(20),
          decoration: BoxDecoration(
            color: colorScheme.primaryContainer,
            shape: BoxShape.circle,
            border: Border.all(color: colorScheme.outlineVariant),
          ),
          child: Icon(icon, size: 40, color: colorScheme.onPrimaryContainer),
        ),
        const SizedBox(height: 16),
        Text(
          title,
          style: theme.textTheme.headlineMedium?.copyWith(
            fontWeight: FontWeight.w700,
            letterSpacing: 0,
          ),
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 6),
        Text(
          subtitle,
          style: theme.textTheme.bodyLarge?.copyWith(
            color: colorScheme.onSurfaceVariant,
          ),
          textAlign: TextAlign.center,
        ),
      ],
    );
  }
}

class AppErrorBanner extends StatelessWidget {
  final String message;

  const AppErrorBanner({super.key, required this.message});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: colorScheme.errorContainer,
        borderRadius: BorderRadius.circular(12),
      ),
      child: Row(
        children: [
          Icon(
            Icons.error_outline_rounded,
            color: colorScheme.onErrorContainer,
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              message,
              style: TextStyle(color: colorScheme.onErrorContainer),
            ),
          ),
        ],
      ),
    );
  }
}

enum AppStatusTone { success, warning, info }

class AppStatusBanner extends StatelessWidget {
  final String message;
  final AppStatusTone tone;
  final IconData? icon;
  final VoidCallback? onDismiss;

  const AppStatusBanner({
    super.key,
    required this.message,
    this.tone = AppStatusTone.info,
    this.icon,
    this.onDismiss,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    late final Color backgroundColor;
    late final Color foregroundColor;
    late final Color borderColor;
    late final IconData resolvedIcon;

    switch (tone) {
      case AppStatusTone.success:
        backgroundColor = Colors.green.withValues(alpha: 0.12);
        foregroundColor = Colors.green.shade700;
        borderColor = Colors.green.withValues(alpha: 0.24);
        resolvedIcon = Icons.check_circle_outline_rounded;
        break;
      case AppStatusTone.warning:
        backgroundColor = colorScheme.errorContainer.withValues(alpha: 0.32);
        foregroundColor = colorScheme.onErrorContainer;
        borderColor = colorScheme.error.withValues(alpha: 0.24);
        resolvedIcon = Icons.warning_amber_rounded;
        break;
      case AppStatusTone.info:
        backgroundColor = colorScheme.primaryContainer.withValues(alpha: 0.32);
        foregroundColor = colorScheme.onPrimaryContainer;
        borderColor = colorScheme.primary.withValues(alpha: 0.18);
        resolvedIcon = Icons.info_outline_rounded;
        break;
    }

    return Container(
      margin: const EdgeInsets.only(bottom: 16),
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: backgroundColor,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: borderColor),
      ),
      child: Row(
        children: [
          Icon(icon ?? resolvedIcon, color: foregroundColor, size: 20),
          const SizedBox(width: 10),
          Expanded(
            child: Text(message, style: TextStyle(color: foregroundColor)),
          ),
          if (onDismiss != null)
            IconButton(
              onPressed: onDismiss,
              icon: Icon(Icons.close_rounded, color: foregroundColor),
              tooltip: 'Dismiss',
            ),
        ],
      ),
    );
  }
}

SnackBar _buildAppSnackBar(
  BuildContext context, {
  required String message,
  required AppStatusTone tone,
}) {
  final colorScheme = Theme.of(context).colorScheme;
  late final Color backgroundColor;
  late final Color foregroundColor;
  late final IconData icon;

  switch (tone) {
    case AppStatusTone.success:
      backgroundColor = Colors.green.shade700;
      foregroundColor = Colors.white;
      icon = Icons.check_circle_outline_rounded;
      break;
    case AppStatusTone.warning:
      backgroundColor = colorScheme.error;
      foregroundColor = colorScheme.onError;
      icon = Icons.warning_amber_rounded;
      break;
    case AppStatusTone.info:
      backgroundColor = colorScheme.primary;
      foregroundColor = colorScheme.onPrimary;
      icon = Icons.info_outline_rounded;
      break;
  }

  return SnackBar(
    behavior: SnackBarBehavior.floating,
    backgroundColor: backgroundColor,
    content: Row(
      children: [
        Icon(icon, color: foregroundColor, size: 18),
        const SizedBox(width: 10),
        Expanded(
          child: Text(
            message,
            style: TextStyle(
              color: foregroundColor,
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
      ],
    ),
    shape: RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(AppUiTokens.cardRadius),
    ),
  );
}

void showAppSnackBar(
  BuildContext context, {
  required String message,
  AppStatusTone tone = AppStatusTone.info,
}) {
  final messenger = ScaffoldMessenger.of(context);
  messenger
    ..hideCurrentSnackBar()
    ..showSnackBar(_buildAppSnackBar(context, message: message, tone: tone));
}

void showAppSuccessSnackBar(BuildContext context, String message) {
  showAppSnackBar(context, message: message, tone: AppStatusTone.success);
}

void showAppInfoSnackBar(BuildContext context, String message) {
  showAppSnackBar(context, message: message, tone: AppStatusTone.info);
}

void showAppErrorSnackBar(BuildContext context, String message) {
  showAppSnackBar(context, message: message, tone: AppStatusTone.warning);
}

class AppFormSection extends StatelessWidget {
  final String title;
  final String? subtitle;
  final Widget? child;
  final List<Widget> children;
  final EdgeInsetsGeometry padding;
  final double spacing;

  AppFormSection({
    super.key,
    required this.title,
    this.subtitle,
    this.child,
    this.children = const [],
    this.padding = const EdgeInsets.all(16),
    this.spacing = AppUiTokens.sectionSpacing,
  }) : assert(
         child == null || children.isEmpty,
         'Use either child or children in AppFormSection.',
       );

  @override
  Widget build(BuildContext context) {
    final subtitleStyle = Theme.of(context).textTheme.bodySmall?.copyWith(
      color: Theme.of(context).colorScheme.onSurfaceVariant,
    );
    final bodyChildren = child == null ? children : <Widget>[child!];

    return AppPanel(
      padding: padding,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          AppSectionTitle(title),
          if (subtitle != null) ...[
            const SizedBox(height: AppUiTokens.compactSpacing),
            Text(subtitle!, style: subtitleStyle),
          ],
          if (bodyChildren.isNotEmpty) ...[
            SizedBox(height: spacing),
            ...bodyChildren,
          ],
        ],
      ),
    );
  }
}

class AppLabeledDivider extends StatelessWidget {
  final String label;

  const AppLabeledDivider({super.key, this.label = 'or'});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Row(
      children: [
        Expanded(child: Divider(color: colorScheme.outlineVariant)),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: Text(
            label,
            style: TextStyle(color: colorScheme.onSurfaceVariant, fontSize: 12),
          ),
        ),
        Expanded(child: Divider(color: colorScheme.outlineVariant)),
      ],
    );
  }
}

class AppAccountSummaryPanel extends StatelessWidget {
  final String title;
  final String? subtitle;
  final Widget? trailing;

  const AppAccountSummaryPanel({
    super.key,
    required this.title,
    this.subtitle,
    this.trailing,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return AppPanel(
      padding: const EdgeInsets.all(12),
      child: Row(
        children: [
          CircleAvatar(
            backgroundColor: colorScheme.secondaryContainer,
            radius: 20,
            child: Icon(
              Icons.account_circle,
              color: colorScheme.onSecondaryContainer,
              size: 24,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.w600,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
                if (subtitle != null) ...[
                  const SizedBox(height: 2),
                  Text(
                    subtitle!,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ],
            ),
          ),
          if (trailing case final trailingWidget?) ...<Widget>[trailingWidget],
        ],
      ),
    );
  }
}

class AppActionTextField extends StatelessWidget {
  final TextEditingController controller;
  final String label;
  final String? hintText;
  final IconData prefixIcon;
  final String? helperText;
  final TextInputType? keyboardType;
  final int maxLines;
  final bool enabled;
  final ValueChanged<String>? onChanged;
  final VoidCallback? onAction;
  final IconData actionIcon;
  final String actionTooltip;

  const AppActionTextField({
    super.key,
    required this.controller,
    required this.label,
    required this.prefixIcon,
    this.hintText,
    this.helperText,
    this.keyboardType,
    this.maxLines = 1,
    this.enabled = true,
    this.onChanged,
    this.onAction,
    this.actionIcon = Icons.auto_fix_high_rounded,
    this.actionTooltip = 'Action',
  });

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: TextFormField(
            controller: controller,
            enabled: enabled,
            onChanged: onChanged,
            keyboardType: keyboardType,
            maxLines: maxLines,
            decoration: InputDecoration(
              labelText: label,
              hintText: hintText,
              prefixIcon: Icon(prefixIcon, size: 20),
              prefixIconConstraints: const BoxConstraints(
                minWidth: 40,
                minHeight: 40,
              ),
              helperText: helperText,
              contentPadding: const EdgeInsets.symmetric(
                horizontal: 12,
                vertical: 16,
              ),
            ),
          ),
        ),
        const SizedBox(width: 8),
        SizedBox(
          width: 44,
          height: 44,
          child: IconButton(
            onPressed: enabled ? onAction : null,
            icon: Icon(actionIcon, size: 20),
            tooltip: actionTooltip,
            padding: EdgeInsets.zero,
          ),
        ),
      ],
    );
  }
}

InputDecoration appInputDecoration({
  required String label,
  String? hintText,
  String? helperText,
  IconData? prefixIcon,
}) {
  return InputDecoration(
    labelText: label,
    hintText: hintText,
    helperText: helperText,
    prefixIcon: prefixIcon == null ? null : Icon(prefixIcon, size: 20),
    prefixIconConstraints: prefixIcon == null
        ? null
        : const BoxConstraints(minWidth: 40, minHeight: 40),
    contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 16),
    border: OutlineInputBorder(
      borderRadius: BorderRadius.circular(AppUiTokens.cardRadius),
    ),
  );
}

class AppTextInput extends StatelessWidget {
  final TextEditingController controller;
  final String label;
  final String? hintText;
  final String? helperText;
  final IconData? prefixIcon;
  final TextInputType? keyboardType;
  final int maxLines;
  final bool enabled;
  final ValueChanged<String>? onChanged;

  const AppTextInput({
    super.key,
    required this.controller,
    required this.label,
    this.hintText,
    this.helperText,
    this.prefixIcon,
    this.keyboardType,
    this.maxLines = 1,
    this.enabled = true,
    this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    return TextFormField(
      controller: controller,
      enabled: enabled,
      onChanged: onChanged,
      keyboardType: keyboardType,
      maxLines: maxLines,
      decoration: appInputDecoration(
        label: label,
        hintText: hintText,
        helperText: helperText,
        prefixIcon: prefixIcon,
      ),
    );
  }
}

class AppDropdownField<T> extends StatelessWidget {
  final T? initialValue;
  final String label;
  final String? hintText;
  final String? helperText;
  final IconData? prefixIcon;
  final bool isExpanded;
  final ValueChanged<T?>? onChanged;
  final List<DropdownMenuItem<T>> items;

  const AppDropdownField({
    super.key,
    required this.initialValue,
    required this.label,
    required this.items,
    this.hintText,
    this.helperText,
    this.prefixIcon,
    this.isExpanded = true,
    this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    return DropdownButtonFormField<T>(
      initialValue: initialValue,
      isExpanded: isExpanded,
      decoration: appInputDecoration(
        label: label,
        hintText: hintText,
        helperText: helperText,
        prefixIcon: prefixIcon,
      ),
      items: items,
      onChanged: onChanged,
    );
  }
}

class AppDetailRow extends StatelessWidget {
  final String label;
  final String value;

  const AppDetailRow({super.key, required this.label, required this.value});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 72,
          child: Text(
            label,
            style: TextStyle(
              fontWeight: FontWeight.w600,
              color: colorScheme.onSurfaceVariant,
            ),
          ),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Text(
            value,
            style: const TextStyle(fontWeight: FontWeight.w600),
            overflow: TextOverflow.ellipsis,
          ),
        ),
      ],
    );
  }
}

class AppSegmentedTabBar extends StatelessWidget {
  final TabController controller;
  final List<Widget> tabs;
  final EdgeInsetsGeometry? margin;

  const AppSegmentedTabBar({
    super.key,
    required this.controller,
    required this.tabs,
    this.margin,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return Container(
      margin: margin,
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(20),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: 0.03),
            blurRadius: 8,
            offset: const Offset(0, 2),
          ),
        ],
      ),
      child: TabBar(
        controller: controller,
        labelColor: colorScheme.onSurface,
        unselectedLabelColor: colorScheme.onSurfaceVariant,
        indicator: BoxDecoration(
          color: colorScheme.surface,
          borderRadius: BorderRadius.circular(16),
          border: Border.all(
            color: colorScheme.outline.withValues(alpha: 0.15),
            width: 1.5,
          ),
          boxShadow: [
            BoxShadow(
              color: Colors.black.withValues(alpha: 0.04),
              blurRadius: 4,
              offset: const Offset(0, 1),
            ),
          ],
        ),
        indicatorWeight: 0,
        dividerColor: Colors.transparent,
        labelPadding: EdgeInsets.zero,
        tabAlignment: TabAlignment.fill,
        isScrollable: false,
        labelStyle: theme.textTheme.labelLarge?.copyWith(
          fontWeight: FontWeight.w700,
          fontSize: 13,
        ),
        unselectedLabelStyle: theme.textTheme.labelLarge?.copyWith(
          fontWeight: FontWeight.w500,
          fontSize: 13,
        ),
        overlayColor: WidgetStateProperty.resolveWith((states) {
          if (states.contains(WidgetState.pressed)) {
            return colorScheme.primary.withValues(alpha: 0.08);
          }
          return null;
        }),
        splashFactory: InkSplash.splashFactory,
        tabs: tabs
            .map((tab) => Tab(height: 48, child: Center(child: tab)))
            .toList(),
      ),
    );
  }
}

class AppTabPageSection extends StatelessWidget {
  final TabController controller;
  final List<Widget> tabs;
  final List<Widget> children;
  final EdgeInsetsGeometry? tabBarMargin;
  final EdgeInsetsGeometry? viewPadding;

  const AppTabPageSection({
    super.key,
    required this.controller,
    required this.tabs,
    required this.children,
    this.tabBarMargin,
    this.viewPadding,
  }) : assert(
         tabs.length == children.length,
         'tabs and children must have the same length',
       );

  @override
  Widget build(BuildContext context) {
    final tabView = TabBarView(controller: controller, children: children);

    return Column(
      children: [
        AppSegmentedTabBar(
          controller: controller,
          margin: tabBarMargin,
          tabs: tabs,
        ),
        Expanded(
          child: Padding(
            padding: viewPadding ?? EdgeInsets.zero,
            child: tabView,
          ),
        ),
      ],
    );
  }
}

class AppWideButton extends StatelessWidget {
  final VoidCallback? onPressed;
  final IconData icon;
  final String label;
  final AppWideButtonStyle style;
  final Widget? child;

  const AppWideButton({
    super.key,
    required this.onPressed,
    required this.icon,
    required this.label,
    this.style = AppWideButtonStyle.primary,
    this.child,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final buttonChild =
        child ??
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              icon,
              color: style == AppWideButtonStyle.text
                  ? colorScheme.primary
                  : null,
            ),
            const SizedBox(width: 8),
            Text(
              label,
              style: style == AppWideButtonStyle.text
                  ? TextStyle(
                      color: colorScheme.primary,
                      fontWeight: FontWeight.bold,
                    )
                  : null,
            ),
          ],
        );

    final Widget button;
    switch (style) {
      case AppWideButtonStyle.primary:
        button = child == null
            ? FilledButton.icon(
                onPressed: onPressed,
                icon: Icon(icon),
                label: Text(label),
                style: FilledButton.styleFrom(
                  minimumSize: const Size(double.infinity, 56),
                ),
              )
            : FilledButton(
                onPressed: onPressed,
                style: FilledButton.styleFrom(
                  minimumSize: const Size(double.infinity, 56),
                ),
                child: child,
              );
        break;
      case AppWideButtonStyle.tonal:
        button = child == null
            ? FilledButton.tonalIcon(
                onPressed: onPressed,
                icon: Icon(icon),
                label: Text(label),
                style: FilledButton.styleFrom(
                  minimumSize: const Size(double.infinity, 56),
                  backgroundColor: KanariColors.lime,
                  foregroundColor: KanariColors.ink,
                ),
              )
            : FilledButton.tonal(
                onPressed: onPressed,
                style: FilledButton.styleFrom(
                  minimumSize: const Size(double.infinity, 56),
                  backgroundColor: KanariColors.lime,
                  foregroundColor: KanariColors.ink,
                ),
                child: child,
              );
        break;
      case AppWideButtonStyle.outlined:
        button = child == null
            ? OutlinedButton.icon(
                onPressed: onPressed,
                icon: Icon(icon),
                label: Text(label),
                style: OutlinedButton.styleFrom(
                  minimumSize: const Size(double.infinity, 56),
                ),
              )
            : OutlinedButton(
                onPressed: onPressed,
                style: OutlinedButton.styleFrom(
                  minimumSize: const Size(double.infinity, 56),
                ),
                child: child,
              );
        break;
      case AppWideButtonStyle.text:
        button = TextButton(
          onPressed: onPressed,
          style: TextButton.styleFrom(
            minimumSize: const Size(double.infinity, 56),
          ),
          child: buttonChild,
        );
        break;
    }

    return _HoverLift(enabled: onPressed != null, child: button);
  }
}

enum AppWideButtonStyle { primary, tonal, outlined, text }

class _HoverLift extends StatefulWidget {
  final bool enabled;
  final Widget child;

  const _HoverLift({required this.enabled, required this.child});

  @override
  State<_HoverLift> createState() => _HoverLiftState();
}

class _HoverLiftState extends State<_HoverLift> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    return MouseRegion(
      onEnter: widget.enabled ? (_) => setState(() => _hovered = true) : null,
      onExit: widget.enabled ? (_) => setState(() => _hovered = false) : null,
      child: AnimatedSlide(
        duration: reduceMotion
            ? Duration.zero
            : const Duration(milliseconds: 180),
        curve: Curves.easeOutCubic,
        offset: Offset(0, _hovered ? -.045 : 0),
        child: AnimatedScale(
          duration: reduceMotion
              ? Duration.zero
              : const Duration(milliseconds: 180),
          curve: Curves.easeOutCubic,
          scale: _hovered ? 1.01 : 1,
          child: widget.child,
        ),
      ),
    );
  }
}

class AppConfirmationDialog extends StatelessWidget {
  final IconData? icon;
  final Color? iconColor;
  final String title;
  final String content;
  final String confirmLabel;
  final bool isDestructive;

  const AppConfirmationDialog({
    super.key,
    this.icon,
    this.iconColor,
    required this.title,
    required this.content,
    this.confirmLabel = 'Confirm',
    this.isDestructive = false,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return AlertDialog(
      icon: icon != null
          ? Icon(
              icon,
              color: iconColor ?? (isDestructive ? colorScheme.error : null),
            )
          : null,
      title: Text(title),
      content: Text(content),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          style: isDestructive
              ? FilledButton.styleFrom(backgroundColor: colorScheme.error)
              : null,
          onPressed: () => Navigator.pop(context, true),
          child: Text(confirmLabel),
        ),
      ],
    );
  }
}

class AppPinChangeDialog extends StatelessWidget {
  final Future<bool> Function(String oldPin, String newPin) onSubmit;
  final String title;
  final String successMessage;
  final String failureMessage;

  const AppPinChangeDialog({
    super.key,
    required this.onSubmit,
    this.title = 'Change PIN',
    this.successMessage = 'PIN changed successfully',
    this.failureMessage = 'Failed to change PIN (Old PIN incorrect?)',
  });

  @override
  Widget build(BuildContext context) {
    final oldPinController = TextEditingController();
    final newPinController = TextEditingController();
    final confirmPinController = TextEditingController();

    return AlertDialog(
      icon: const Icon(Icons.pin_rounded),
      title: Text(title),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _pinField(oldPinController, 'Current PIN'),
            const SizedBox(height: 12),
            _pinField(newPinController, 'New PIN'),
            const SizedBox(height: 12),
            _pinField(confirmPinController, 'Confirm New PIN'),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () async {
            final oldPin = oldPinController.text;
            final newPin = newPinController.text;
            final confirmPin = confirmPinController.text;

            if (oldPin.length != 6 ||
                newPin.length != 6 ||
                newPin != confirmPin) {
              showAppErrorSnackBar(context, 'Invalid PIN or PINs do not match');
              return;
            }

            Navigator.pop(context);
            final success = await onSubmit(oldPin, newPin);

            if (!context.mounted) return;

            if (success) {
              showAppSuccessSnackBar(context, successMessage);
            } else {
              showAppErrorSnackBar(context, failureMessage);
            }
          },
          child: const Text('Update'),
        ),
      ],
    );
  }

  Widget _pinField(TextEditingController controller, String label) {
    return TextField(
      controller: controller,
      obscureText: true,
      keyboardType: TextInputType.number,
      inputFormatters: [FilteringTextInputFormatter.digitsOnly],
      maxLength: 6,
      decoration: InputDecoration(labelText: label),
    );
  }
}

class AppPinEntrySheet extends StatefulWidget {
  final String title;
  final String subtitle;
  final ValueChanged<String>? onComplete;
  final Future<bool> Function(String pin)? onValidate;
  final int pinLength;
  final Future<bool> Function()? onBiometricAuthenticate;
  final String biometricReason;
  final bool biometricHandlesPrompt;

  const AppPinEntrySheet({
    super.key,
    required this.title,
    required this.subtitle,
    this.onComplete,
    this.onValidate,
    this.pinLength = 6,
    this.onBiometricAuthenticate,
    this.biometricReason = 'Authenticate with biometrics',
    this.biometricHandlesPrompt = false,
  });

  @override
  State<AppPinEntrySheet> createState() => _AppPinEntrySheetState();
}

Future<String?> showAppPinEntrySheet({
  required BuildContext context,
  required String title,
  required String subtitle,
  int pinLength = 6,
  Future<bool> Function()? onBiometricAuthenticate,
  String biometricReason = 'Authenticate with biometrics',
  bool biometricHandlesPrompt = false,
}) {
  return showAppModalSheet<String>(
    context: context,
    builder: (_) => AppPinEntrySheet(
      title: title,
      subtitle: subtitle,
      pinLength: pinLength,
      onBiometricAuthenticate: onBiometricAuthenticate,
      biometricReason: biometricReason,
      biometricHandlesPrompt: biometricHandlesPrompt,
    ),
  );
}

class _AppPinEntrySheetState extends State<AppPinEntrySheet> {
  final LocalAuthentication _localAuth = LocalAuthentication();
  String _enteredPin = '';
  String? _errorText;
  bool _isChecking = false;
  bool _canUseBiometric = false;

  @override
  void initState() {
    super.initState();
    _loadBiometricAvailability();
  }

  Future<void> _loadBiometricAvailability() async {
    if (widget.onBiometricAuthenticate == null) return;

    try {
      final biometricEnabled = await WalletStorage.isBiometricEnabled();
      if (!biometricEnabled) return;

      if (widget.biometricHandlesPrompt) {
        if (mounted) {
          setState(() => _canUseBiometric = true);
        }
        return;
      }

      final canUseBiometric =
          await _localAuth.isDeviceSupported() &&
          await _localAuth.canCheckBiometrics;

      if (mounted) {
        setState(() => _canUseBiometric = canUseBiometric);
      }
    } catch (_) {
      if (mounted) {
        setState(() => _canUseBiometric = false);
      }
    }
  }

  Future<void> _handleNumberPressed(String number) async {
    if (_isChecking || _enteredPin.length >= widget.pinLength) return;

    final pin = (_enteredPin + number);
    setState(() {
      _errorText = null;
      _enteredPin = pin;
    });

    if (pin.length != widget.pinLength) return;

    final validator = widget.onValidate;
    if (validator == null) {
      Future.delayed(const Duration(milliseconds: 200), () {
        if (!mounted) return;
        Navigator.pop(context, pin);
        widget.onComplete?.call(pin);
      });
      return;
    }

    setState(() => _isChecking = true);

    try {
      final isValid = await validator(pin);
      if (!mounted) return;

      if (isValid) {
        Navigator.pop(context, pin);
        widget.onComplete?.call(pin);
        return;
      }

      setState(() {
        _enteredPin = '';
        _isChecking = false;
        _errorText = 'Invalid PIN';
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _enteredPin = '';
        _isChecking = false;
        _errorText = 'Unable to verify PIN';
      });
    }
  }

  void _handleBackspace() {
    if (_isChecking || _enteredPin.isEmpty) return;

    setState(() {
      _errorText = null;
      _enteredPin = _enteredPin.substring(0, _enteredPin.length - 1);
    });
  }

  Future<void> _authenticateWithBiometric() async {
    final callback = widget.onBiometricAuthenticate;
    if (_isChecking || callback == null) return;

    setState(() {
      _isChecking = true;
      _errorText = null;
    });

    try {
      final biometricSuccess = widget.biometricHandlesPrompt
          ? true
          : await _localAuth.authenticate(
              localizedReason: widget.biometricReason,
              biometricOnly: true,
              persistAcrossBackgrounding: true,
            );

      if (!mounted) return;

      if (!biometricSuccess) {
        setState(() {
          _isChecking = false;
          _errorText = 'Biometric authentication was cancelled';
        });
        return;
      }

      final authorized = await callback();
      if (!mounted) return;

      if (authorized) {
        Navigator.pop(context, appPinBiometricResult);
        return;
      }

      setState(() {
        _isChecking = false;
        _errorText = 'Biometric authentication could not complete this action';
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _isChecking = false;
        _errorText = 'Biometric authentication is unavailable';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return Scaffold(
      backgroundColor: Colors.transparent,
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        elevation: 0,
        leading: CloseButton(color: colorScheme.onSurface),
      ),
      body: SafeArea(
        child: Column(
          children: [
            const SizedBox(height: 16),
            Text(
              widget.title,
              style: theme.textTheme.headlineMedium?.copyWith(
                fontWeight: FontWeight.bold,
                color: colorScheme.onSurface,
              ),
            ),
            const SizedBox(height: 12),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 32.0),
              child: Text(
                widget.subtitle,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyLarge?.copyWith(
                  color: colorScheme.onSurfaceVariant,
                ),
              ),
            ),
            const Spacer(),
            _AppPinCirclesDisplay(
              length: _enteredPin.length,
              totalLength: widget.pinLength,
            ),
            AnimatedSwitcher(
              duration: const Duration(milliseconds: 160),
              child: _errorText == null
                  ? const SizedBox(height: 28)
                  : Padding(
                      padding: const EdgeInsets.only(top: 10),
                      child: Text(
                        _errorText!,
                        key: ValueKey(_errorText),
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: colorScheme.error,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
            ),
            if (_isChecking)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: SizedBox(
                  width: 22,
                  height: 22,
                  child: CircularProgressIndicator(
                    strokeWidth: 2.2,
                    color: colorScheme.tertiary,
                  ),
                ),
              )
            else
              const SizedBox(height: 30),
            const Spacer(),
            _AppCustomNumberPad(
              onNumberPressed: _handleNumberPressed,
              onBackspacePressed: _handleBackspace,
              biometricEnabled: _canUseBiometric,
              onBiometricPressed: _authenticateWithBiometric,
            ),
            const SizedBox(height: 16),
          ],
        ),
      ),
    );
  }
}

Future<bool> showAppPinVerificationSheet({
  required BuildContext context,
  required Future<bool> Function(String pin) onVerify,
  Future<Duration?> Function()? lockRemaining,
  String title = 'Confirm Transaction',
  String subtitle = 'Enter your 6-digit PIN to authorize this transaction.',
}) async {
  final result = await showAppModalSheet<bool>(
    context: context,
    isDismissible: true,
    enableDrag: true,
    builder: (_) => _AppPinVerificationSheet(
      title: title,
      subtitle: subtitle,
      onVerify: onVerify,
      lockRemaining: lockRemaining,
    ),
  );

  return result ?? false;
}

class _AppPinVerificationSheet extends StatefulWidget {
  final String title;
  final String subtitle;
  final Future<bool> Function(String pin) onVerify;
  final Future<Duration?> Function()? lockRemaining;

  const _AppPinVerificationSheet({
    required this.title,
    required this.subtitle,
    required this.onVerify,
    this.lockRemaining,
  });

  @override
  State<_AppPinVerificationSheet> createState() =>
      _AppPinVerificationSheetState();
}

class _AppPinVerificationSheetState extends State<_AppPinVerificationSheet> {
  static const int _pinLength = 6;
  final LocalAuthentication _localAuth = LocalAuthentication();
  String _enteredPin = '';
  String? _errorText;
  bool _isChecking = false;
  bool _canUseBiometric = false;

  @override
  void initState() {
    super.initState();
    _loadBiometricAvailability();
  }

  Future<void> _loadBiometricAvailability() async {
    try {
      final biometricEnabled = await WalletStorage.isBiometricEnabled();
      if (!biometricEnabled) {
        if (mounted) {
          setState(() => _canUseBiometric = false);
        }
        return;
      }

      final canUseBiometric =
          await _localAuth.isDeviceSupported() &&
          await _localAuth.canCheckBiometrics;

      if (mounted) {
        setState(() => _canUseBiometric = canUseBiometric);
      }
    } catch (_) {
      if (mounted) {
        setState(() => _canUseBiometric = false);
      }
    }
  }

  void _handleNumberPressed(String number) {
    if (_isChecking || _enteredPin.length >= _pinLength) return;

    setState(() {
      _errorText = null;
      _enteredPin += number;
    });

    if (_enteredPin.length == _pinLength) {
      Future.delayed(const Duration(milliseconds: 160), _verifyPin);
    }
  }

  void _handleBackspace() {
    if (_isChecking || _enteredPin.isEmpty) return;

    setState(() {
      _errorText = null;
      _enteredPin = _enteredPin.substring(0, _enteredPin.length - 1);
    });
  }

  Future<void> _verifyPin() async {
    if (_isChecking) return;

    setState(() => _isChecking = true);
    final success = await widget.onVerify(_enteredPin);

    if (!mounted) return;

    if (success) {
      Navigator.pop(context, true);
      return;
    }

    final remaining = await widget.lockRemaining?.call();
    if (!mounted) return;

    setState(() {
      _isChecking = false;
      _enteredPin = '';
      _errorText = remaining == null
          ? 'Invalid PIN'
          : 'Too many attempts. Try again in ${remaining.inSeconds + 1}s';
    });
  }

  Future<void> _verifyBiometric() async {
    if (_isChecking) return;

    setState(() {
      _isChecking = true;
      _errorText = null;
    });

    try {
      final success = await _localAuth.authenticate(
        localizedReason: 'Authorize this Kanari transaction',
        biometricOnly: true,
        persistAcrossBackgrounding: true,
      );

      if (!mounted) return;

      if (success) {
        Navigator.pop(context, true);
        return;
      }

      setState(() {
        _isChecking = false;
        _errorText = 'Biometric authentication was cancelled';
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _isChecking = false;
        _errorText = 'Biometric authentication is unavailable';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return Scaffold(
      backgroundColor: Colors.transparent,
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        elevation: 0,
        leading: CloseButton(color: colorScheme.onSurface),
      ),
      body: SafeArea(
        child: Column(
          children: [
            const SizedBox(height: 16),
            Text(
              widget.title,
              style: theme.textTheme.headlineMedium?.copyWith(
                fontWeight: FontWeight.bold,
                color: colorScheme.onSurface,
              ),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 12),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 32.0),
              child: Text(
                widget.subtitle,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyLarge?.copyWith(
                  color: colorScheme.onSurfaceVariant,
                ),
              ),
            ),
            const Spacer(),
            _AppPinCirclesDisplay(
              length: _enteredPin.length,
              totalLength: _pinLength,
            ),
            AnimatedSwitcher(
              duration: const Duration(milliseconds: 160),
              child: _errorText == null
                  ? const SizedBox(height: 28)
                  : Padding(
                      padding: const EdgeInsets.only(top: 10),
                      child: Text(
                        _errorText!,
                        key: ValueKey(_errorText),
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: colorScheme.error,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
            ),
            if (_isChecking)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: SizedBox(
                  width: 22,
                  height: 22,
                  child: CircularProgressIndicator(
                    strokeWidth: 2.2,
                    color: colorScheme.tertiary,
                  ),
                ),
              )
            else
              const SizedBox(height: 30),
            const Spacer(),
            _AppCustomNumberPad(
              onNumberPressed: _handleNumberPressed,
              onBackspacePressed: _handleBackspace,
              biometricEnabled: _canUseBiometric,
              onBiometricPressed: _isChecking ? null : _verifyBiometric,
            ),
            const SizedBox(height: 16),
          ],
        ),
      ),
    );
  }
}

class _AppPinCirclesDisplay extends StatelessWidget {
  final int length;
  final int totalLength;

  const _AppPinCirclesDisplay({
    required this.length,
    required this.totalLength,
  });

  @override
  Widget build(BuildContext context) {
    final primaryColor = Theme.of(context).colorScheme.tertiary;
    final outlineColor = Theme.of(context).colorScheme.outlineVariant;

    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      mainAxisSize: MainAxisSize.min,
      children: List.generate(totalLength, (index) {
        final isFilled = index < length;
        return Flexible(
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 150),
            width: 18,
            height: 18,
            margin: const EdgeInsets.symmetric(horizontal: 6.0),
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: isFilled ? primaryColor : Colors.transparent,
              border: Border.all(
                color: isFilled ? primaryColor : outlineColor,
                width: 2,
              ),
            ),
          ),
        );
      }),
    );
  }
}

class _AppCustomNumberPad extends StatelessWidget {
  final ValueChanged<String> onNumberPressed;
  final VoidCallback onBackspacePressed;
  final bool biometricEnabled;
  final VoidCallback? onBiometricPressed;

  const _AppCustomNumberPad({
    required this.onNumberPressed,
    required this.onBackspacePressed,
    this.biometricEnabled = false,
    this.onBiometricPressed,
  });

  @override
  Widget build(BuildContext context) {
    final numbers = [
      ['1', '2', '3'],
      ['4', '5', '6'],
      ['7', '8', '9'],
    ];

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 32.0),
      child: Column(
        children: [
          for (var row in numbers) ...[
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceEvenly,
              children: row
                  .map(
                    (number) => _AppNumberButton(
                      number: number,
                      onPressed: () => onNumberPressed(number),
                    ),
                  )
                  .toList(),
            ),
            const SizedBox(height: 16),
          ],
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceEvenly,
            children: [
              biometricEnabled
                  ? SizedBox(
                      width: 64,
                      height: 64,
                      child: IconButton(
                        onPressed: onBiometricPressed,
                        icon: const Icon(Icons.fingerprint_rounded),
                        iconSize: 28,
                        color: Theme.of(context).colorScheme.onSurfaceVariant,
                        tooltip: 'Use biometric',
                      ),
                    )
                  : const SizedBox(width: 64, height: 64),
              _AppNumberButton(
                number: '0',
                onPressed: () => onNumberPressed('0'),
              ),
              SizedBox(
                width: 64,
                height: 64,
                child: IconButton(
                  onPressed: onBackspacePressed,
                  icon: const Icon(Icons.backspace_outlined),
                  iconSize: 28,
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _AppNumberButton extends StatelessWidget {
  final String number;
  final VoidCallback onPressed;

  const _AppNumberButton({required this.number, required this.onPressed});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final theme = Theme.of(context);

    return Container(
      width: 64, // â† à¸¥à¸”à¸‚à¸™à¸²à¸”à¸¥à¸‡à¸ˆà¸²à¸ 80
      height: 64,
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerHigh,
        shape: BoxShape.circle,
        border: Border.all(color: colorScheme.outlineVariant),
      ),
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          onTap: onPressed,
          customBorder: const CircleBorder(),
          child: Center(
            child: Text(
              number,
              style: theme.textTheme.titleLarge?.copyWith(
                // â† à¸¥à¸”à¸‚à¸™à¸²à¸” font à¸¥à¸‡
                fontWeight: FontWeight.w600,
                color: colorScheme.onSurface,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
