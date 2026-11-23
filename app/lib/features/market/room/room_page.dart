import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../../core/api/api_error.dart';
import '../../../core/strings.dart';
import '../../../core/theme/tokens.dart';
import '../../../core/ui/pressable.dart';
import '../../../core/ui/states.dart';
import '../../cart/cart_button.dart';
import '../../catalog/catalog_models.dart';
import '../data/asset_resolver.dart';
import '../data/market_source.dart';
import '../model/market.dart';
import '../model/room.dart';
import '../perf/render_tier.dart';
import 'depth_room.dart';
import 'shelf_card.dart';

/// Inside a store: its room, with its products on the shelves.
///
/// The room's picture shows as soon as its type is known (from the bundle);
/// the shelves fill when the store's products arrive.
class RoomPage extends ConsumerWidget {
  const RoomPage({
    super.key,
    required this.placement,
    required this.onBack,
    required this.onOpenProduct,
    required this.onSeeAll,
  });

  final Placement placement;
  final VoidCallback onBack;
  final void Function(String productId) onOpenProduct;
  final VoidCallback onSeeAll;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final slug = placement.store.slug;
    final room = ref.watch(roomProvider(slug));
    final type =
        room.value?.type ??
        ref.watch(storeTypeProvider(placement.storeType)).value;
    final assets = ref.watch(assetResolverProvider);
    final tier = effectiveTier(
      ref.watch(renderTierProvider),
      reduceMotion: MediaQuery.disableAnimationsOf(context),
    );
    final tokens = context.tokens;

    Widget? panel;
    // Read even while a retry runs, so a failure never looks like loading.
    final error = room.hasValue ? null : room.error;
    if (error != null) {
      panel = error is ApiError && error.code == 'store_not_found'
          ? _Notice(text: Strings.storeClosed, onBack: onBack)
          : ErrorState(
              message: Strings.roomLoadFailed,
              onRetry: () => ref.invalidate(roomProvider(slug)),
            );
    } else if (room case AsyncData(:final value) when value.placed.isEmpty) {
      panel = _Notice(text: Strings.emptyShelves, onBack: onBack);
    }

    return Scaffold(
      backgroundColor: tokens.bg,
      body: Stack(
        fit: StackFit.expand,
        children: [
          if (type != null)
            DepthRoom(
              room: assets.room(type),
              depth: assets.depth(type),
              tier: tier,
              overlay: (look, picture) => _Shelves(
                type: type,
                room: room.value,
                loading: !room.hasValue && room.error == null,
                picture: picture,
                look: look,
                onOpenProduct: onOpenProduct,
              ),
            ),
          if (panel != null)
            Center(
              child: Padding(
                padding: const EdgeInsets.all(Tokens.s5),
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: tokens.card,
                    borderRadius: BorderRadius.circular(Tokens.radius),
                  ),
                  child: Padding(
                    padding: const EdgeInsets.all(Tokens.s4),
                    child: panel,
                  ),
                ),
              ),
            ),
          SafeArea(
            child: Padding(
              padding: const EdgeInsets.all(Tokens.s3),
              child: Column(
                children: [
                  _TopBar(
                    store: room.value?.store ?? placement.store,
                    onBack: onBack,
                  ),
                  const Spacer(),
                  if (room.value?.hasMore ?? false)
                    FilledButton.tonal(
                      onPressed: onSeeAll,
                      style: FilledButton.styleFrom(
                        minimumSize: const Size(120, Tokens.minTap),
                      ),
                      child: const Text(Strings.seeAll),
                    ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// The product cards (or their skeletons) laid on the room's slots.
class _Shelves extends StatelessWidget {
  const _Shelves({
    required this.type,
    required this.room,
    required this.loading,
    required this.picture,
    required this.look,
    required this.onOpenProduct,
  });

  final StoreType type;
  final RoomView? room;
  final bool loading;

  /// Where the room picture is drawn on screen.
  final Rect picture;
  final Offset look;
  final void Function(String productId) onOpenProduct;

  Rect _onScreen(ShelfSlot s) => Rect.fromLTWH(
    picture.left + s.rect.left * picture.width,
    picture.top + s.rect.top * picture.height,
    s.rect.width * picture.width,
    s.rect.height * picture.height,
  );

  @override
  Widget build(BuildContext context) {
    final children = <Widget>[];
    if (loading) {
      for (final s in type.slots) {
        final r = _onScreen(s);
        children.add(
          Positioned.fromRect(
            rect: r,
            child: const Skeleton(key: Key('shelf-skeleton')),
          ),
        );
      }
    } else if (room case final room?) {
      for (final p in room.placed) {
        final slot = type.slot(p.slotId);
        if (slot == null) continue;
        final r = _onScreen(slot);
        // At least 48 dp, centred on the slot.
        final hit = Rect.fromCenter(
          center: r.center,
          width: r.width < Tokens.minTap ? Tokens.minTap : r.width,
          height: r.height < Tokens.minTap ? Tokens.minTap : r.height,
        );
        children.add(
          Positioned.fromRect(
            rect: hit,
            child: ShelfCard(
              product: p.product,
              slot: slot,
              type: type,
              size: r.size,
              look: look,
              onTap: () => onOpenProduct(p.product.id),
            ),
          ),
        );
      }
    }
    return Stack(clipBehavior: Clip.none, children: children);
  }
}

class _TopBar extends StatelessWidget {
  const _TopBar({required this.store, required this.onBack});

  final StoreSummary store;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final rating = store.ratingAverage;
    final count = store.ratingCount;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tokens.card.withValues(alpha: 0.94),
        borderRadius: BorderRadius.circular(Tokens.minTap / 2 + Tokens.s1),
      ),
      child: Padding(
        padding: const EdgeInsets.all(Tokens.s1),
        child: Row(
          children: [
            Pressable(
              semanticLabel: Strings.backToMarket,
              radius: Tokens.minTap / 2,
              onTap: onBack,
              child: SizedBox.square(
                dimension: Tokens.minTap,
                child: Icon(Icons.arrow_back, color: tokens.ink),
              ),
            ),
            const SizedBox(width: Tokens.s1),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    store.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.titleSmall?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  Row(
                    children: [
                      if (rating != null && count > 0) ...[
                        Icon(Icons.star_rounded, size: 14, color: tokens.gold),
                        const SizedBox(width: 2),
                        Text(
                          '${Strings.rating(rating)} ($count)',
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
              ),
            ),
            const CartButton(),
          ],
        ),
      ),
    );
  }
}

/// A short sentence with the way back, for a closed or empty store.
class _Notice extends StatelessWidget {
  const _Notice({required this.text, required this.onBack});

  final String text;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          text,
          textAlign: TextAlign.center,
          style: Theme.of(context).textTheme.titleMedium,
        ),
        const SizedBox(height: Tokens.s4),
        FilledButton(
          onPressed: onBack,
          style: FilledButton.styleFrom(
            minimumSize: const Size(160, Tokens.minTap),
          ),
          child: const Text(Strings.backToMarket),
        ),
      ],
    );
  }
}
