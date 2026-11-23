import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../cart/cart_button.dart';
import '../catalog/catalog_models.dart';
import '../catalog/catalog_repository.dart';
import 'paged_list.dart';
import 'rows.dart';

final storeDetailProvider = FutureProvider.autoDispose
    .family<StoreDetail, String>(
      (ref, slug) => ref.watch(catalogRepositoryProvider).store(slug),
    );

/// A store as a plain page: who they are and what they sell. No room.
class StorePage extends ConsumerWidget {
  const StorePage({super.key, required this.slug});

  final String slug;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final detail = ref.watch(storeDetailProvider(slug));
    final store = detail.value;
    final error = store == null ? detail.error : null;
    final catalog = ref.watch(catalogRepositoryProvider);
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(
        title: Text(store?.summary.name ?? ''),
        actions: const [
          CartButton(),
          SizedBox(width: Tokens.s2),
        ],
      ),
      body: switch ((store, error)) {
        (final s?, _) => PagedList<ProductCard>(
          key: ValueKey(('store', slug)),
          load: (cursor) => catalog.products(store: slug, cursor: cursor),
          itemBuilder: (context, p) => ProductRow(product: p, showStore: false),
          empty: const EmptyState(title: Strings.emptyShelves),
          header: Padding(
            padding: const EdgeInsets.only(bottom: Tokens.s4),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                StoreRating(store: s.summary),
                if (s.description.isNotEmpty) ...[
                  const SizedBox(height: Tokens.s3),
                  Text(s.description, style: theme.textTheme.bodyLarge),
                ],
              ],
            ),
          ),
        ),
        (null, final ApiError e) when e.status == 404 => const EmptyState(
          title: Strings.storeClosed,
        ),
        (null, final _?) => ErrorState(
          message: Strings.storeLoadFailed,
          onRetry: () => ref.invalidate(storeDetailProvider(slug)),
        ),
        _ => ListView(
          padding: const EdgeInsets.all(Tokens.s4),
          children: const [
            Skeleton(height: 20, width: 120),
            SizedBox(height: Tokens.s3),
            Skeleton(height: 48),
            SizedBox(height: Tokens.s5),
            Skeleton(height: 64),
            SizedBox(height: Tokens.s3),
            Skeleton(height: 64),
          ],
        ),
      },
    );
  }
}
