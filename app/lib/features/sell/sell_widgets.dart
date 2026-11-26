import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import '../../core/ui/states.dart';
import 'sell_models.dart';
import 'sell_providers.dart';
import 'sell_strings.dart';

/// Loading, error and data for an [AsyncValue], in the app's usual states.
class AsyncBody<T> extends StatelessWidget {
  const AsyncBody({
    super.key,
    required this.value,
    required this.builder,
    required this.onRetry,
  });

  final AsyncValue<T> value;
  final Widget Function(T data) builder;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final data = value.value;
    if (data != null) return builder(data);
    if (value.hasError) {
      return ErrorState(message: sellError(value.error!), onRetry: onRetry);
    }
    return Semantics(
      label: Strings.loading,
      child: const Padding(
        padding: EdgeInsets.all(Tokens.s5),
        child: Column(
          children: [
            Skeleton(height: 72),
            SizedBox(height: Tokens.s3),
            Skeleton(height: 72),
            SizedBox(height: Tokens.s3),
            Skeleton(height: 72),
          ],
        ),
      ),
    );
  }
}

/// One row of a list: a title, an optional second line, a trailing widget and
/// press feedback.
class SellRow extends StatelessWidget {
  const SellRow({
    super.key,
    required this.title,
    this.subtitle,
    this.trailing,
    this.onTap,
  });

  final String title;
  final String? subtitle;
  final Widget? trailing;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    return Pressable(
      onTap: onTap,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: Tokens.minTap),
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Tokens.s5,
            vertical: Tokens.s3,
          ),
          child: Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(title, style: theme.textTheme.titleMedium),
                    if (subtitle != null && subtitle!.isNotEmpty) ...[
                      const SizedBox(height: Tokens.s1),
                      Text(
                        subtitle!,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: tokens.mute,
                        ),
                      ),
                    ],
                  ],
                ),
              ),
              if (trailing != null) ...[
                const SizedBox(width: Tokens.s3),
                trailing!,
              ] else if (onTap != null)
                Icon(Icons.chevron_right, color: tokens.mute),
            ],
          ),
        ),
      ),
    );
  }
}

/// A small coloured label, for a status.
class StatusPill extends StatelessWidget {
  const StatusPill({super.key, required this.label, this.good = false});

  final String label;
  final bool good;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final fg = good ? tokens.good : tokens.mute;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: fg.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(Tokens.radius),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Tokens.s3,
          vertical: Tokens.s1,
        ),
        child: Text(
          label,
          style: Theme.of(context).textTheme.labelMedium?.copyWith(color: fg),
        ),
      ),
    );
  }
}

/// An error line announced to screen readers.
class ErrorLine extends StatelessWidget {
  const ErrorLine(this.message, {super.key});

  final String message;

  @override
  Widget build(BuildContext context) => Semantics(
    liveRegion: true,
    child: Text(
      message,
      style: Theme.of(context).textTheme.bodyMedium
          ?.copyWith(color: context.tokens.errorText),
    ),
  );
}

/// Runs [action], keeps a busy flag and an error line. Pages mix it in so a
/// double tap cannot start a second call.
mixin BusyAction<T extends StatefulWidget> on State<T> {
  bool busy = false;
  String? error;

  Future<bool> run(Future<void> Function() action) async {
    if (busy) return false;
    setState(() {
      busy = true;
      error = null;
    });
    try {
      await action();
      return true;
    } on Object catch (e) {
      if (mounted) setState(() => error = sellError(e));
      return false;
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }
}

/// A page title bar for pages pushed inside the Sell area.
AppBar sellAppBar(
  BuildContext context,
  String title, {
  List<Widget>? actions,
}) => AppBar(
  title: Text(title),
  actions: actions,
  backgroundColor: context.tokens.bg,
  surfaceTintColor: const Color(0x00000000),
);

/// Opens [page] on top of the current one.
Future<T?> pushPage<T>(BuildContext context, Widget page) =>
    Navigator.of(context).push<T>(MaterialPageRoute<T>(builder: (_) => page));

/// Naira (whole or with kobo) as the person types it, to integer kobo; null when it is
/// not a valid amount.
int? nairaToKobo(String text) {
  final t = text.trim().replaceAll(',', '');
  if (t.isEmpty) return null;
  final m = RegExp(r'^(\d{1,9})(?:\.(\d{1,2}))?$').firstMatch(t);
  if (m == null) return null;
  final naira = int.parse(m.group(1)!);
  final frac = (m.group(2) ?? '').padRight(2, '0');
  return naira * 100 + (frac.isEmpty ? 0 : int.parse(frac));
}

/// The title bar of a tab of the Sell area: a way back to shopping and, when the
/// person has more than one store, the store switcher.
AppBar sellRootAppBar(BuildContext context, String title) => AppBar(
  title: Text(title),
  backgroundColor: context.tokens.bg,
  surfaceTintColor: const Color(0x00000000),
  leading: IconButton(
    tooltip: SellStrings.backToShopping,
    icon: const Icon(Icons.close),
    onPressed: () => Navigator.of(context).maybePop(),
  ),
  actions: const [StoreSwitchButton()],
);

/// Opens a sheet to choose which store the Sell area shows. Hidden with one store.
class StoreSwitchButton extends ConsumerWidget {
  const StoreSwitchButton({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final stores = ref.watch(myStoresProvider).value ?? const <SellStore>[];
    if (stores.length < 2) return const SizedBox.shrink();
    return IconButton(
      tooltip: SellStrings.switchStore,
      icon: const Icon(Icons.swap_horiz),
      onPressed: () => showModalBottomSheet<void>(
        context: context,
        showDragHandle: true,
        builder: (context) => SafeArea(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final s in stores)
                SellRow(
                  title: s.name,
                  trailing: s.id == ref.read(selectedStoreProvider)?.id
                      ? const Icon(Icons.check)
                      : null,
                  onTap: () {
                    ref.read(selectedStoreIdProvider.notifier).choose(s.id);
                    Navigator.of(context).pop();
                  },
                ),
            ],
          ),
        ),
      ),
    );
  }
}
