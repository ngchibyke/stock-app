import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/catalog/catalog_models.dart';
import 'package:market/features/market/camera/market_camera.dart';
import 'package:market/features/market/camera/tiles.dart';
import 'package:market/features/market/data/asset_resolver.dart';
import 'package:market/features/market/model/market.dart';
import 'package:market/features/market/view/market_view.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/fake_asset_resolver.dart';
import '../../support/fake_catalog_repository.dart';

const _view = Size(400, 800);

MarketSnapshot snapshot({
  List<Placement>? placements,
  String name = 'Store ada',
}) {
  const layout = MarketLayout(
    version: 't',
    world: Size(2048, 2048),
    tileSize: 256,
    maxLevel: 3,
    baseImage: 'base.jpg',
    districts: [
      District(id: 'fashion', name: 'Fashion'),
      District(id: 'food', name: 'Food and groceries'),
    ],
    plots: [
      Plot(
        id: 'p1',
        districtId: 'fashion',
        rect: Rect.fromLTWH(100, 100, 100, 80),
      ),
      Plot(
        id: 'p2',
        districtId: 'food',
        rect: Rect.fromLTWH(1800, 1800, 100, 80),
      ),
    ],
  );
  return MarketSnapshot(
    layout: layout,
    placements:
        placements ??
        [
          Placement(
            plotId: 'p1',
            storeType: 'boutique',
            store: StoreSummary(
              id: 'id-ada',
              slug: 'ada',
              name: name,
              ratingAverage: 4.2,
              ratingCount: 5,
            ),
          ),
        ],
  );
}

MarketCamera camera({
  double zoom = 1,
  Offset center = const Offset(1024, 1024),
}) => MarketCamera(
  world: const Size(2048, 2048),
  viewport: _view,
  center: center,
  zoom: zoom,
);

class _Harness {
  final resolver = FakeAssetResolver();
  final opened = <Placement>[];
  final prefetched = <Placement>[];
  MarketCamera? last;

  Future<void> pump(
    WidgetTester tester, {
    MarketSnapshot? snap,
    MarketCamera? cam,
    double textScale = 1,
  }) async {
    tester.view.physicalSize = _view;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [assetResolverProvider.overrideWithValue(resolver)],
        child: MaterialApp(
          theme: lightTheme(),
          home: MediaQuery(
            data: MediaQueryData(
              size: _view,
              textScaler: TextScaler.linear(textScale),
              disableAnimations: true,
            ),
            child: Scaffold(
              body: MarketView(
                snapshot: snap ?? snapshot(),
                camera: cam ?? camera(),
                onCamera: (c) => last = c,
                onOpenStore: opened.add,
                onPrefetch: prefetched.add,
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
  }
}

// p1's centre (150, 140) on screen at zoom 1, centre (1024, 1024).
Offset p1Screen(MarketCamera c) => c.worldToScreen(const Offset(150, 140));

void main() {
  testWidgets('draws_the_base_picture_before_any_tile_arrives', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    final images = tester.widgetList<Image>(find.byType(Image));
    expect(images.any((i) => identical(i.image, h.resolver.baseImage)), isTrue);
  });

  testWidgets('requests_only_visible_tiles_at_the_chosen_level', (
    tester,
  ) async {
    final h = _Harness();
    final cam = camera(zoom: 4);
    await h.pump(tester, cam: cam);
    final level = levelFor(cam, maxLevel: 3, dpr: 1);
    expect(
      h.resolver.tiles,
      visibleTiles(cam, level: level, maxLevel: 3).toSet(),
    );
  });

  testWidgets('open_and_coming_soon_markers_are_labelled', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    expect(
      find.bySemanticsLabel('Store ada, 4.2 stars, 5 reviews, open'),
      findsOneWidget,
    );
    expect(
      find.bySemanticsLabel('Coming soon, Food and groceries'),
      findsOneWidget,
    );
  });

  testWidgets('a_store_with_no_reviews_says_so', (tester) async {
    final h = _Harness();
    final snap = snapshot(
      placements: [
        Placement(
          plotId: 'p1',
          storeType: 'boutique',
          store: fakeStore('new', rating: null, count: 0),
        ),
      ],
    );
    await h.pump(tester, snap: snap);
    expect(
      find.bySemanticsLabel('Store new, no reviews yet, open'),
      findsOneWidget,
    );
  });

  testWidgets('names_appear_only_from_zoom_1_6', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    expect(find.text('Store ada'), findsNothing);
    await h.pump(
      tester,
      cam: camera(zoom: 1.7, center: const Offset(400, 400)),
    );
    expect(find.text('Store ada'), findsOneWidget);
  });

  testWidgets('marker_hit_area_is_at_least_48dp_at_zoom_1', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    final c = camera();
    await tester.tapAt(p1Screen(c) + const Offset(22, 22));
    expect(h.opened.single.plotId, 'p1');
    await tester.tapAt(p1Screen(c) + const Offset(60, 0));
    expect(h.opened, hasLength(1));
  });

  testWidgets('tap_opens_and_press_prefetches', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    final g = await tester.startGesture(p1Screen(camera()));
    await tester.pump(const Duration(milliseconds: 150));
    expect(h.prefetched.single.plotId, 'p1');
    expect(h.opened, isEmpty);
    await g.up();
    await tester.pump();
    expect(h.opened.single.plotId, 'p1');
  });

  testWidgets('tapping_a_coming_soon_plot_opens_nothing', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    await tester.tapAt(camera().worldToScreen(const Offset(1850, 1840)));
    expect(h.opened, isEmpty);
  });

  testWidgets('no_stores_shows_every_plot_as_coming_soon', (tester) async {
    final h = _Harness();
    await h.pump(tester, snap: snapshot(placements: const []));
    expect(find.bySemanticsLabel(RegExp(r'^Coming soon, ')), findsNWidgets(2));
    expect(find.bySemanticsLabel(RegExp(r', open$')), findsNothing);
  });

  testWidgets('a_120_char_name_is_ellipsised_and_fully_in_the_label', (
    tester,
  ) async {
    final h = _Harness();
    final long = 'A' * 120;
    await h.pump(
      tester,
      snap: snapshot(name: long),
      cam: camera(zoom: 1.7, center: const Offset(400, 400)),
    );
    final text = tester.widget<Text>(find.text(long));
    expect(text.overflow, TextOverflow.ellipsis);
    expect(text.maxLines, 1);
    expect(
      find.bySemanticsLabel('$long, 4.2 stars, 5 reviews, open'),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('markers_off_screen_are_not_in_semantics', (tester) async {
    final h = _Harness();
    await h.pump(tester, cam: camera(zoom: 4, center: const Offset(300, 300)));
    expect(find.bySemanticsLabel(RegExp('^Store ada')), findsOneWidget);
    expect(
      find.bySemanticsLabel('Coming soon, Food and groceries'),
      findsNothing,
    );
  });

  testWidgets('a_pinch_reports_a_new_camera', (tester) async {
    final h = _Harness();
    await h.pump(tester, cam: camera(zoom: 2));
    await tester.dragFrom(const Offset(200, 600), const Offset(-40, 0));
    expect(h.last, isNotNull);
    expect(h.last!.center.dx, greaterThan(1024));
  });

  testWidgets('text_at_200_percent_does_not_overflow', (tester) async {
    final h = _Harness();
    await h.pump(
      tester,
      cam: camera(zoom: 1.7, center: const Offset(400, 400)),
      textScale: 2,
    );
    expect(tester.takeException(), isNull);
  });
}
