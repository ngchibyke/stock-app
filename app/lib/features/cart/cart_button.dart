import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/paths.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import 'cart_repository.dart';

/// Marks where the cart button sits, so "add to cart" can fly towards it.
final cartButtonKeyProvider = Provider<GlobalKey>((ref) => GlobalKey());

/// The cart icon with its count. Opens the cart.
class CartButton extends ConsumerWidget {
  const CartButton({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final count = ref.watch(cartProvider.select((c) => c.value?.count ?? 0));
    final tokens = context.tokens;
    return Pressable(
      semanticLabel: Strings.cartLabel(count),
      radius: Tokens.minTap / 2,
      onTap: () => GoRouter.maybeOf(context)?.push(cartPath),
      child: SizedBox.square(
        key: ref.watch(cartButtonKeyProvider),
        dimension: Tokens.minTap,
        child: Stack(
          alignment: Alignment.center,
          clipBehavior: Clip.none,
          children: [
            Icon(Icons.shopping_bag_outlined, color: tokens.ink),
            if (count > 0)
              Positioned(
                top: Tokens.s1 + 2,
                right: Tokens.s1,
                child: AnimatedSwitcher(
                  duration: const Duration(milliseconds: 150),
                  transitionBuilder: (child, a) =>
                      ScaleTransition(scale: a, child: child),
                  child: Container(
                    key: const Key('cart-badge'),
                    constraints: const BoxConstraints(minWidth: 18),
                    height: 18,
                    padding: const EdgeInsets.symmetric(horizontal: Tokens.s1),
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: tokens.accent,
                      borderRadius: BorderRadius.circular(9),
                    ),
                    child: Text(
                      count > 99 ? '99+' : '$count',
                      style: const TextStyle(
                        color: Colors.white,
                        fontSize: 11,
                        fontWeight: FontWeight.w700,
                        height: 1,
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
