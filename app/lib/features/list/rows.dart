import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/format/money.dart';
import '../../core/paths.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import '../catalog/catalog_models.dart';

/// A product in a list: photo, name, store and starting price.
class ProductRow extends StatelessWidget {
  const ProductRow({super.key, required this.product, this.showStore = true});

  final ProductCard product;
  final bool showStore;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final url = product.photoUrl;
    final tile = ColoredBox(
      color: tokens.line,
      child: Icon(Icons.shopping_bag_outlined, color: tokens.mute, size: 22),
    );
    return Pressable(
      onTap: () => context.push(productPath(product.id), extra: product.store),
      child: Container(
        padding: const EdgeInsets.all(Tokens.s2),
        decoration: BoxDecoration(
          color: tokens.card,
          borderRadius: BorderRadius.circular(Tokens.radius),
          border: Border.all(color: tokens.line),
        ),
        child: Row(
          children: [
            ClipRRect(
              borderRadius: BorderRadius.circular(Tokens.s2 + 2),
              child: SizedBox.square(
                dimension: 64,
                child: url == null
                    ? tile
                    : Image.network(
                        url,
                        fit: BoxFit.cover,
                        excludeFromSemantics: true,
                        errorBuilder: (_, _, _) => tile,
                      ),
              ),
            ),
            const SizedBox(width: Tokens.s3),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    product.name,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodyLarge?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  if (showStore) ...[
                    const SizedBox(height: 2),
                    Text(
                      product.store.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: tokens.mute,
                      ),
                    ),
                  ],
                  const SizedBox(height: Tokens.s1),
                  Text(
                    Strings.fromPrice(formatNaira(product.priceFromKobo)),
                    style: theme.textTheme.bodyMedium?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A store in a list: its initial, name and rating.
class StoreRow extends StatelessWidget {
  const StoreRow({super.key, required this.store});

  final StoreSummary store;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    return Pressable(
      onTap: () => context.push(listStorePath(store.slug)),
      child: Container(
        padding: const EdgeInsets.all(Tokens.s3),
        decoration: BoxDecoration(
          color: tokens.card,
          borderRadius: BorderRadius.circular(Tokens.radius),
          border: Border.all(color: tokens.line),
        ),
        child: Row(
          children: [
            CircleAvatar(
              radius: 22,
              backgroundColor: tokens.brand.withValues(alpha: 0.1),
              child: Text(
                store.name.characters.first.toUpperCase(),
                style: TextStyle(
                  color: tokens.brand,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
            const SizedBox(width: Tokens.s3),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    store.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodyLarge?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 2),
                  StoreRating(store: store),
                ],
              ),
            ),
            Icon(Icons.chevron_right, color: tokens.mute),
          ],
        ),
      ),
    );
  }
}

/// "★ 4.2 (5)", or "No reviews yet".
class StoreRating extends StatelessWidget {
  const StoreRating({super.key, required this.store});

  final StoreSummary store;

  @override
  Widget build(BuildContext context) {
    final style = Theme.of(context).textTheme.bodySmall
        ?.copyWith(color: context.tokens.mute);
    final r = store.ratingAverage;
    if (r == null || store.ratingCount == 0) {
      return Text(Strings.noReviewsYet, style: style);
    }
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(Icons.star_rounded, size: 14, color: context.tokens.gold),
        const SizedBox(width: 2),
        Text('${Strings.rating(r)} (${store.ratingCount})', style: style),
      ],
    );
  }
}
