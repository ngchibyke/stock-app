import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/money.dart';
import '../../core/paths.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import '../../core/ui/states.dart';
import 'cart.dart';
import 'cart_repository.dart';

/// The cart, grouped by store. Every amount on it is the server's.
class CartPage extends ConsumerStatefulWidget {
  const CartPage({super.key});

  @override
  ConsumerState<CartPage> createState() => _CartPageState();
}

class _CartPageState extends ConsumerState<CartPage> {
  // Lines waiting on the server, so one cannot be changed twice at once.
  final Set<String> _busy = {};

  Future<void> _set(CartLine line, int quantity) async {
    if (_busy.contains(line.variantId)) return;
    if (quantity == 0 && line.available && !await _confirmRemove(line)) return;
    if (!mounted) return;
    setState(() => _busy.add(line.variantId));
    final messenger = ScaffoldMessenger.of(context);
    try {
      await ref
          .read(cartProvider.notifier)
          .setQuantity(line.variantId, quantity);
    } catch (e) {
      if (!mounted) return;
      // 404: the product or its store went away since the cart loaded.
      final gone = e is ApiError && e.status == 404;
      messenger
        ..hideCurrentSnackBar()
        ..showSnackBar(
          SnackBar(
            content: Text(
              gone ? Strings.noLongerBuyable : Strings.cartUpdateFailed,
            ),
          ),
        );
    } finally {
      if (mounted) setState(() => _busy.remove(line.variantId));
    }
  }

  Future<bool> _confirmRemove(CartLine line) async {
    final yes = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(Strings.removeItemQuestion(line.productName)),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text(Strings.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text(Strings.remove),
          ),
        ],
      ),
    );
    return yes == true && mounted;
  }

  @override
  Widget build(BuildContext context) {
    final cart = ref.watch(cartProvider);
    return Scaffold(
      appBar: AppBar(title: const Text(Strings.cart)),
      body: switch (cart) {
        AsyncData(:final value) when value.items.isEmpty => EmptyState(
          title: Strings.cartEmptyTitle,
          body: Strings.cartEmptyBody,
          actionLabel: Strings.goToMarket,
          onAction: () => context.go(homePath),
        ),
        AsyncData(:final value) => _Filled(
          cart: value,
          busy: _busy,
          onSet: _set,
        ),
        AsyncError() => ErrorState(
          message: Strings.cartLoadFailed,
          onRetry: () => ref.read(cartProvider.notifier).refresh(),
        ),
        _ => ListView(
          padding: const EdgeInsets.all(Tokens.s4),
          children: [
            for (var i = 0; i < 3; i++)
              const Padding(
                padding: EdgeInsets.only(bottom: Tokens.s3),
                child: Skeleton(height: 88),
              ),
          ],
        ),
      },
    );
  }
}

class _Filled extends StatelessWidget {
  const _Filled({required this.cart, required this.busy, required this.onSet});

  final Cart cart;
  final Set<String> busy;
  final void Function(CartLine line, int quantity) onSet;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    return Column(
      children: [
        Expanded(
          child: ListView(
            padding: const EdgeInsets.fromLTRB(
              Tokens.s4,
              Tokens.s3,
              Tokens.s4,
              Tokens.s5,
            ),
            children: [
              for (final MapEntry(key: store, value: lines)
                  in cart.byStore.entries) ...[
                Padding(
                  padding: const EdgeInsets.fromLTRB(
                    Tokens.s1,
                    Tokens.s3,
                    0,
                    Tokens.s2,
                  ),
                  child: Text(
                    store,
                    style: theme.textTheme.titleSmall?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),
                DecoratedBox(
                  decoration: BoxDecoration(
                    color: tokens.card,
                    borderRadius: BorderRadius.circular(Tokens.radius),
                    border: Border.all(color: tokens.line),
                  ),
                  child: Column(
                    children: [
                      for (final (i, line) in lines.indexed) ...[
                        if (i > 0) Divider(height: 1, color: tokens.line),
                        _Line(
                          key: ValueKey('cart-line-${line.variantId}'),
                          line: line,
                          busy: busy.contains(line.variantId),
                          onSet: (q) => onSet(line, q),
                        ),
                      ],
                    ],
                  ),
                ),
              ],
            ],
          ),
        ),
        DecoratedBox(
          decoration: BoxDecoration(
            color: tokens.card,
            border: Border(top: BorderSide(color: tokens.line)),
          ),
          child: SafeArea(
            top: false,
            child: Padding(
              padding: const EdgeInsets.fromLTRB(
                Tokens.s4,
                Tokens.s4,
                Tokens.s4,
                Tokens.s4,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      Text(Strings.total, style: theme.textTheme.titleMedium),
                      const Spacer(),
                      Text(
                        formatNaira(cart.totalKobo),
                        style: theme.textTheme.titleLarge?.copyWith(
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: Tokens.s2),
                  Text(
                    Strings.checkoutSoon,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: tokens.mute,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _Line extends StatelessWidget {
  const _Line({
    super.key,
    required this.line,
    required this.busy,
    required this.onSet,
  });

  final CartLine line;
  final bool busy;
  final ValueChanged<int> onSet;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final attrs = line.attributes.entries
        .map((e) => '${e.key}: ${e.value}')
        .join(' · ');
    final details = Opacity(
      opacity: line.available ? 1 : 0.5,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            line.productName,
            style: theme.textTheme.bodyLarge?.copyWith(
              fontWeight: FontWeight.w600,
            ),
          ),
          if (attrs.isNotEmpty) ...[
            const SizedBox(height: 2),
            Text(
              attrs,
              style: theme.textTheme.bodySmall?.copyWith(color: tokens.mute),
            ),
          ],
          const SizedBox(height: 2),
          Text(
            Strings.eachPrice(formatNaira(line.unitPriceKobo)),
            style: theme.textTheme.bodySmall?.copyWith(color: tokens.mute),
          ),
        ],
      ),
    );
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Tokens.s4,
        Tokens.s3,
        Tokens.s2,
        Tokens.s3,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(child: details),
          const SizedBox(width: Tokens.s2),
          if (line.available)
            Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Text(
                  formatNaira(line.lineTotalKobo),
                  style: theme.textTheme.bodyLarge?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: Tokens.s1),
                _Stepper(
                  quantity: line.quantity,
                  busy: busy,
                  onMinus: () => onSet(line.quantity - 1),
                  onPlus: () => onSet(line.quantity + 1),
                ),
              ],
            )
          else
            Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Text(
                  Strings.noLongerAvailable,
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: tokens.errorText,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                TextButton(
                  onPressed: busy ? null : () => onSet(0),
                  style: TextButton.styleFrom(
                    minimumSize: const Size(Tokens.minTap, Tokens.minTap),
                  ),
                  child: const Text(Strings.remove),
                ),
              ],
            ),
        ],
      ),
    );
  }
}

/// Minus, the count, plus. The count only changes when the server answers.
class _Stepper extends StatelessWidget {
  const _Stepper({
    required this.quantity,
    required this.busy,
    required this.onMinus,
    required this.onPlus,
  });

  final int quantity;
  final bool busy;
  final VoidCallback onMinus;
  final VoidCallback onPlus;

  /// The server's per-line limit.
  static const max = 100;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    Widget button(IconData icon, String label, VoidCallback? onTap) =>
        Pressable(
          semanticLabel: label,
          radius: Tokens.minTap / 2,
          onTap: onTap == null
              ? null
              : () {
                  HapticFeedback.selectionClick();
                  onTap();
                },
          child: SizedBox.square(
            dimension: Tokens.minTap,
            child: Icon(
              icon,
              size: 20,
              color: onTap == null ? tokens.line : tokens.ink,
            ),
          ),
        );
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: tokens.line),
        borderRadius: BorderRadius.circular(Tokens.minTap / 2),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          button(Icons.remove, Strings.decreaseQuantity, busy ? null : onMinus),
          SizedBox(
            width: 28,
            child: busy
                ? const Center(
                    child: SizedBox.square(
                      dimension: 14,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  )
                : Text(
                    '$quantity',
                    textAlign: TextAlign.center,
                    style: const TextStyle(fontWeight: FontWeight.w700),
                  ),
          ),
          button(
            Icons.add,
            Strings.increaseQuantity,
            busy || quantity >= max ? null : onPlus,
          ),
        ],
      ),
    );
  }
}
