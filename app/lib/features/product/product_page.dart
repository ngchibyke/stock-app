import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/money.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../cart/cart_button.dart';
import '../cart/cart_repository.dart';
import '../catalog/catalog_models.dart';
import '../catalog/catalog_repository.dart';

final productProvider = FutureProvider.autoDispose
    .family<ProductDetail, String>(
      (ref, id) => ref.watch(catalogRepositoryProvider).product(id),
    );

/// One product: its photos, its variants to choose from, and "Add to cart".
class ProductPage extends ConsumerStatefulWidget {
  const ProductPage({super.key, required this.productId, this.store});

  final String productId;

  /// The store it is sold by, when the caller knows it (the product itself
  /// only carries the store's id).
  final StoreSummary? store;

  @override
  ConsumerState<ProductPage> createState() => _ProductPageState();
}

class _ProductPageState extends ConsumerState<ProductPage> {
  /// This page's own cart button, where an added item flies to.
  final GlobalKey _cartAnchor = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final product = ref.watch(productProvider(widget.productId));
    final error = product.hasValue ? null : product.error;
    return Scaffold(
      appBar: AppBar(
        actions: [
          CartButton(anchor: _cartAnchor),
          const SizedBox(width: Tokens.s2),
        ],
      ),
      body: switch (product.value) {
        final p? => _Loaded(
          product: p,
          store: widget.store,
          cartAnchor: _cartAnchor,
        ),
        null when error != null => ErrorState(
          message: Strings.productLoadFailed,
          onRetry: () => ref.invalidate(productProvider(widget.productId)),
        ),
        null => const _Skeleton(),
      },
    );
  }
}

class _Skeleton extends StatelessWidget {
  const _Skeleton();

  @override
  Widget build(BuildContext context) => ListView(
    padding: const EdgeInsets.all(Tokens.s4),
    children: const [
      AspectRatio(aspectRatio: 1, child: Skeleton()),
      SizedBox(height: Tokens.s4),
      Skeleton(height: 24, width: 220),
      SizedBox(height: Tokens.s2),
      Skeleton(height: 20, width: 120),
      SizedBox(height: Tokens.s5),
      Skeleton(height: 40),
      SizedBox(height: Tokens.s3),
      Skeleton(height: 40),
    ],
  );
}

class _Loaded extends ConsumerStatefulWidget {
  const _Loaded({
    required this.product,
    required this.store,
    required this.cartAnchor,
  });

  final ProductDetail product;
  final StoreSummary? store;
  final GlobalKey cartAnchor;

  @override
  ConsumerState<_Loaded> createState() => _LoadedState();
}

class _LoadedState extends ConsumerState<_Loaded> {
  final Map<String, String> _chosen = {};
  final GlobalKey _addKey = GlobalKey();
  bool _adding = false;

  ProductDetail get _p => widget.product;

  /// Every attribute name the variants use, sorted.
  List<String> get _keys =>
      ({for (final v in _p.variants) ...v.attributes.keys}.toList()..sort());

  List<String> _values(String key) =>
      ({for (final v in _p.variants) ?v.attributes[key]}.toList()..sort());

  /// The variant the choices point to, once every attribute is chosen.
  Variant? get _variant {
    if (_p.variants.length == 1 && _p.variants.single.attributes.isEmpty) {
      return _p.variants.single;
    }
    final keys = _keys;
    if (keys.isEmpty || keys.any((k) => !_chosen.containsKey(k))) return null;
    for (final v in _p.variants) {
      if (keys.every((k) => v.attributes[k] == _chosen[k])) return v;
    }
    return null;
  }

  Future<void> _add(Variant v) async {
    if (_adding) return;
    setState(() => _adding = true);
    HapticFeedback.lightImpact();
    final messenger = ScaffoldMessenger.of(context);
    try {
      await ref.read(cartProvider.notifier).add(v.id);
      if (!mounted) return;
      _flyToCart();
      messenger
        ..hideCurrentSnackBar()
        ..showSnackBar(const SnackBar(content: Text(Strings.addedToCart)));
    } on ApiError catch (e) {
      if (!mounted) return;
      final text = e.status == 422
          ? (e.code == 'insufficient_stock'
                ? e.message
                : Strings.noLongerBuyable)
          : Strings.cartUpdateFailed;
      messenger
        ..hideCurrentSnackBar()
        ..showSnackBar(SnackBar(content: Text(text)));
    } finally {
      if (mounted) setState(() => _adding = false);
    }
  }

  /// A small bag flies from "Add to cart" into the cart button.
  void _flyToCart() {
    if (MediaQuery.disableAnimationsOf(context)) return;
    final overlay = Overlay.maybeOf(context);
    final from = _rectOf(_addKey);
    final to = _rectOf(widget.cartAnchor);
    if (overlay == null || from == null || to == null) return;
    final tokens = context.tokens;
    late final OverlayEntry entry;
    entry = OverlayEntry(
      builder: (_) => IgnorePointer(
        child: TweenAnimationBuilder<double>(
          tween: Tween(begin: 0, end: 1),
          duration: const Duration(milliseconds: 450),
          curve: Curves.easeInCubic,
          onEnd: entry.remove,
          builder: (context, t, _) {
            final c = Offset.lerp(from.center, to.center, t)!;
            final size = 28 * (1 - 0.5 * t);
            return Stack(
              children: [
                Positioned(
                  left: c.dx - size / 2,
                  top: c.dy - size / 2,
                  child: Container(
                    width: size,
                    height: size,
                    decoration: BoxDecoration(
                      color: tokens.accent,
                      shape: BoxShape.circle,
                    ),
                    child: Icon(
                      Icons.shopping_bag,
                      size: size * 0.55,
                      color: Colors.white,
                    ),
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
    overlay.insert(entry);
  }

  Rect? _rectOf(GlobalKey key) {
    final box = key.currentContext?.findRenderObject();
    if (box is! RenderBox || !box.hasSize) return null;
    return box.localToGlobal(Offset.zero) & box.size;
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final variant = _variant;
    final allSoldOut = _p.variants.every((v) => v.availableStock <= 0);
    final soldOut = variant != null ? variant.availableStock <= 0 : allSoldOut;
    final cheapest = _p.variants.isEmpty
        ? null
        : _p.variants.map((v) => v.priceKobo).reduce((a, b) => a < b ? a : b);
    final priceText = variant != null
        ? formatNaira(variant.priceKobo)
        : cheapest == null
        ? null
        : Strings.fromPrice(formatNaira(cheapest));
    final stock = variant?.availableStock;
    final stockText = soldOut
        ? Strings.soldOut
        : stock != null && stock <= 5
        ? Strings.onlyLeft(stock)
        : null;
    final store = widget.store;

    return Column(
      children: [
        Expanded(
          child: ListView(
            padding: const EdgeInsets.only(bottom: Tokens.s5),
            children: [
              _Photos(urls: _p.photoUrls),
              Padding(
                padding: const EdgeInsets.fromLTRB(
                  Tokens.s4,
                  Tokens.s4,
                  Tokens.s4,
                  0,
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      _p.name,
                      style: theme.textTheme.headlineSmall?.copyWith(
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    const SizedBox(height: Tokens.s1),
                    Wrap(
                      spacing: Tokens.s2,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      children: [
                        if (_p.brand case final brand?)
                          Text(
                            brand,
                            style: theme.textTheme.bodyMedium?.copyWith(
                              color: tokens.mute,
                            ),
                          ),
                        Text(
                          Strings.conditionLabel(_p.condition),
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: tokens.mute,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: Tokens.s3),
                    if (priceText != null)
                      Text(
                        priceText,
                        style: theme.textTheme.titleLarge?.copyWith(
                          fontWeight: FontWeight.w800,
                        ),
                      ),
                    if (stockText != null) ...[
                      const SizedBox(height: Tokens.s1),
                      Text(
                        stockText,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: soldOut ? tokens.errorText : tokens.accent,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ],
                    for (final key in _keys) ...[
                      const SizedBox(height: Tokens.s4),
                      Text(key, style: theme.textTheme.titleSmall),
                      const SizedBox(height: Tokens.s2),
                      Wrap(
                        spacing: Tokens.s2,
                        runSpacing: Tokens.s2,
                        children: [
                          for (final value in _values(key))
                            ChoiceChip(
                              label: Text(value),
                              selected: _chosen[key] == value,
                              materialTapTargetSize:
                                  MaterialTapTargetSize.padded,
                              onSelected: (_) {
                                HapticFeedback.selectionClick();
                                setState(() => _chosen[key] = value);
                              },
                            ),
                        ],
                      ),
                    ],
                    if (_p.description.isNotEmpty) ...[
                      const SizedBox(height: Tokens.s5),
                      Text(_p.description, style: theme.textTheme.bodyLarge),
                    ],
                    if (store != null) ...[
                      const SizedBox(height: Tokens.s5),
                      Divider(color: tokens.line, height: 1),
                      const SizedBox(height: Tokens.s3),
                      Row(
                        children: [
                          Icon(
                            Icons.storefront_outlined,
                            color: tokens.mute,
                            size: 20,
                          ),
                          const SizedBox(width: Tokens.s2),
                          Expanded(
                            child: Text(
                              store.name,
                              style: theme.textTheme.titleSmall,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                          if (store.ratingAverage case final r?
                              when store.ratingCount > 0) ...[
                            Icon(
                              Icons.star_rounded,
                              size: 16,
                              color: tokens.gold,
                            ),
                            const SizedBox(width: 2),
                            Text(
                              '${Strings.rating(r)} (${store.ratingCount})',
                              style: theme.textTheme.bodySmall?.copyWith(
                                color: tokens.mute,
                              ),
                            ),
                          ] else
                            Text(
                              Strings.noReviewsYet,
                              style: theme.textTheme.bodySmall?.copyWith(
                                color: tokens.mute,
                              ),
                            ),
                        ],
                      ),
                    ],
                  ],
                ),
              ),
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
              padding: const EdgeInsets.all(Tokens.s4),
              child: FilledButton(
                key: _addKey,
                onPressed: variant == null || soldOut || _adding
                    ? null
                    : () => _add(variant),
                style: FilledButton.styleFrom(
                  minimumSize: const Size.fromHeight(Tokens.minTap + 4),
                ),
                child: const Text(Strings.addToCart),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _Photos extends StatefulWidget {
  const _Photos({required this.urls});

  final List<String> urls;

  @override
  State<_Photos> createState() => _PhotosState();
}

class _PhotosState extends State<_Photos> {
  int _at = 0;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final tile = ColoredBox(
      color: tokens.line,
      child: Center(
        child: Icon(Icons.shopping_bag_outlined, color: tokens.mute, size: 48),
      ),
    );
    final urls = widget.urls;
    return AspectRatio(
      aspectRatio: 1,
      child: urls.isEmpty
          ? tile
          : Stack(
              fit: StackFit.expand,
              children: [
                PageView.builder(
                  itemCount: urls.length,
                  onPageChanged: (i) => setState(() => _at = i),
                  itemBuilder: (context, i) => Image.network(
                    urls[i],
                    key: Key('product-photo-$i'),
                    fit: BoxFit.cover,
                    excludeFromSemantics: true,
                    errorBuilder: (_, _, _) => tile,
                  ),
                ),
                if (urls.length > 1)
                  Positioned(
                    right: Tokens.s3,
                    bottom: Tokens.s3,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        color: Colors.black.withValues(alpha: 0.55),
                        borderRadius: BorderRadius.circular(Tokens.s3),
                      ),
                      child: Padding(
                        padding: const EdgeInsets.symmetric(
                          horizontal: Tokens.s2,
                          vertical: 2,
                        ),
                        child: Text(
                          Strings.photoCount(_at + 1, urls.length),
                          style: const TextStyle(
                            color: Colors.white,
                            fontSize: 12,
                          ),
                        ),
                      ),
                    ),
                  ),
              ],
            ),
    );
  }
}
