import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/paths.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import '../../core/ui/states.dart';
import '../cart/cart_button.dart';
import '../catalog/catalog_models.dart';
import 'data/asset_resolver.dart';
import 'data/market_source.dart';
import 'journey/journey_controller.dart';
import 'journey/journey_view.dart';
import 'model/market.dart';
import 'perf/render_tier.dart';
import 'room/room_page.dart';

/// The Market tab: the market itself, walked into store by store.
class MarketPage extends ConsumerStatefulWidget {
  const MarketPage({super.key});

  @override
  ConsumerState<MarketPage> createState() => _MarketPageState();
}

class _MarketPageState extends ConsumerState<MarketPage> {
  JourneyController? _journey;
  bool _fadeOnly = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final fadeOnly =
        effectiveTier(
          ref.read(renderTierProvider),
          reduceMotion: MediaQuery.disableAnimationsOf(context),
        ) ==
        RenderTier.fade;
    if (_journey == null || fadeOnly != _fadeOnly) {
      final old = _journey;
      _fadeOnly = fadeOnly;
      _journey = JourneyController(
        now: DateTime.now,
        flightIn: Duration(milliseconds: fadeOnly ? 250 : 1300),
        flightOut: Duration(milliseconds: fadeOnly ? 250 : 1100),
      );
      // A controller swapped mid-flight is dropped after this frame.
      if (old != null) {
        WidgetsBinding.instance.addPostFrameCallback((_) => old.dispose());
      }
    }
  }

  @override
  void dispose() {
    _journey?.dispose();
    super.dispose();
  }

  void _prefetch(Placement p) {
    // Started early so the shelves are full when the camera lands.
    ref.read(roomProvider(p.store.slug).future).ignore();
  }

  @override
  Widget build(BuildContext context) {
    final journey = _journey!;
    final market = ref.watch(marketProvider);
    final snapshot = market.value;
    final error = snapshot == null ? market.error : null;
    final tokens = context.tokens;

    return ColoredBox(
      color: tokens.bg,
      child: Stack(
        fit: StackFit.expand,
        children: [
          if (snapshot != null)
            JourneyView(
              snapshot: snapshot,
              journey: journey,
              fadeOnly: _fadeOnly,
              onPrefetch: _prefetch,
              roomBuilder: (p) => RoomPage(
                placement: p,
                onBack: journey.back,
                onOpenProduct: (id) =>
                    context.push(productPath(id), extra: p.store),
                onSeeAll: () => context.push(listStorePath(p.store.slug)),
              ),
            )
          else if (error != null)
            SafeArea(
              child: ErrorState(
                message: Strings.marketLoadFailed,
                onRetry: () => ref.invalidate(marketProvider),
              ),
            )
          else
            const _Loading(),
          ListenableBuilder(
            listenable: journey,
            builder: (context, _) => journey.state is AtMarket
                ? _Overlay(offline: snapshot?.fromCache ?? false)
                : const SizedBox.shrink(),
          ),
        ],
      ),
    );
  }
}

/// The market's picture with a strip of skeletons while the stores load.
class _Loading extends ConsumerWidget {
  const _Loading();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Stack(
      fit: StackFit.expand,
      children: [
        Image(
          image: ref.watch(assetResolverProvider).base(),
          fit: BoxFit.cover,
          excludeFromSemantics: true,
          errorBuilder: (_, _, _) => const SizedBox.shrink(),
        ),
        Align(
          alignment: Alignment.bottomCenter,
          child: SafeArea(
            child: Padding(
              padding: const EdgeInsets.all(Tokens.s4),
              child: Row(
                children: [
                  for (var i = 0; i < 3; i++) ...[
                    if (i > 0) const SizedBox(width: Tokens.s3),
                    const Expanded(child: Skeleton(height: 64)),
                  ],
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// Always on screen at the market: search, the list view and the cart.
class _Overlay extends StatelessWidget {
  const _Overlay({required this.offline});

  final bool offline;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final theme = Theme.of(context);
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(Tokens.s3, Tokens.s3, Tokens.s3, 0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            DecoratedBox(
              decoration: BoxDecoration(
                color: tokens.card,
                borderRadius: BorderRadius.circular(
                  Tokens.minTap / 2 + Tokens.s1,
                ),
                boxShadow: [
                  BoxShadow(
                    color: Colors.black.withValues(alpha: 0.08),
                    blurRadius: 12,
                    offset: const Offset(0, 4),
                  ),
                ],
              ),
              child: Padding(
                padding: const EdgeInsets.all(Tokens.s1),
                child: Row(
                  children: [
                    Expanded(
                      child: Pressable(
                        radius: Tokens.minTap / 2,
                        onTap: () => context.push('$listPath?focus=1'),
                        child: SizedBox(
                          height: Tokens.minTap,
                          child: Row(
                            children: [
                              const SizedBox(width: Tokens.s3),
                              Icon(Icons.search, color: tokens.mute),
                              const SizedBox(width: Tokens.s2),
                              Expanded(
                                child: Text(
                                  Strings.searchMarket,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: theme.textTheme.bodyLarge?.copyWith(
                                    color: tokens.mute,
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ),
                    Pressable(
                      semanticLabel: Strings.listView,
                      radius: Tokens.minTap / 2,
                      onTap: () => context.push(listPath),
                      child: SizedBox.square(
                        dimension: Tokens.minTap,
                        child: Icon(
                          Icons.view_list_outlined,
                          color: tokens.ink,
                        ),
                      ),
                    ),
                    const CartButton(),
                  ],
                ),
              ),
            ),
            if (offline) ...[
              const SizedBox(height: Tokens.s2),
              Center(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: tokens.ink.withValues(alpha: 0.85),
                    borderRadius: BorderRadius.circular(Tokens.s3),
                  ),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Tokens.s3,
                      vertical: Tokens.s1,
                    ),
                    child: Text(
                      Strings.offlineMarket,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: Colors.white,
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// A deep link to a store: its room straight away, with no flight.
class StoreRoomRoute extends ConsumerWidget {
  const StoreRoomRoute({super.key, required this.slug});

  final String slug;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final snapshot = ref.watch(marketProvider).value;
    Placement? placed;
    for (final p in snapshot?.placements ?? const <Placement>[]) {
      if (p.store.slug == slug) placed = p;
    }
    // A store not on a plot yet still opens, in the default room.
    final placement =
        placed ??
        Placement(
          plotId: '',
          store: StoreSummary(id: '', slug: slug, name: ''),
          storeType: 'boutique',
        );
    void back() => context.canPop() ? context.pop() : context.go(homePath);
    return RoomPage(
      placement: placement,
      onBack: back,
      onOpenProduct: (id) =>
          context.push(productPath(id), extra: placement.store),
      onSeeAll: () => context.push(listStorePath(slug)),
    );
  }
}
