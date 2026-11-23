import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/market/camera/market_camera.dart';
import 'package:market/features/market/data/asset_resolver.dart';
import 'package:market/features/market/journey/journey_controller.dart';
import 'package:market/features/market/journey/journey_view.dart';
import 'package:market/features/market/model/market.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/fake_asset_resolver.dart';
import '../../support/fake_catalog_repository.dart';

const _view = Size(400, 800);
const _world = Size(2048, 2048);

final _snap = MarketSnapshot(
  layout: const MarketLayout(
    version: 't',
    world: _world,
    tileSize: 256,
    maxLevel: 3,
    baseImage: 'base.jpg',
    districts: [District(id: 'd', name: 'D')],
    plots: [
      Plot(id: 'p1', districtId: 'd', rect: Rect.fromLTWH(950, 960, 100, 80)),
    ],
  ),
  placements: [
    Placement(plotId: 'p1', store: fakeStore('ada'), storeType: 'boutique'),
  ],
);

class _Clock {
  DateTime t = DateTime(2026, 11, 23, 12);
  DateTime now() => t;
}

class _Harness {
  _Harness({int flightIn = 1300, int flightOut = 1100})
    : journey = JourneyController(
        now: _clock.now,
        flightIn: Duration(milliseconds: flightIn),
        flightOut: Duration(milliseconds: flightOut),
      );

  static final _clock = _Clock();
  final JourneyController journey;
  final prefetched = <String>[];

  Future<void> pump(WidgetTester tester, {bool reduceMotion = false}) async {
    tester.view.physicalSize = _view;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          assetResolverProvider.overrideWithValue(FakeAssetResolver()),
        ],
        child: MaterialApp(
          theme: lightTheme(),
          home: MediaQuery(
            data: MediaQueryData(size: _view, disableAnimations: reduceMotion),
            child: Scaffold(
              body: JourneyView(
                snapshot: _snap,
                journey: journey,
                fadeOnly: reduceMotion,
                onPrefetch: (p) => prefetched.add(p.store.slug),
                roomBuilder: (p) => Center(child: Text('ROOM ${p.store.slug}')),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
  }

  Future<void> advance(WidgetTester tester, int ms) async {
    for (var done = 0; done < ms; done += 50) {
      final step = (ms - done).clamp(0, 50);
      _clock.t = _clock.t.add(Duration(milliseconds: step));
      await tester.pump(Duration(milliseconds: step));
    }
  }
}

double roomOpacity(WidgetTester tester) => tester
    .widget<Opacity>(
      find
          .ancestor(of: find.text('ROOM ada'), matching: find.byType(Opacity))
          .first,
    )
    .opacity;

Offset p1OnScreen() => MarketCamera(
  world: _world,
  viewport: _view,
  center: const Offset(1024, 1024),
  zoom: 1,
).worldToScreen(const Offset(1000, 1000));

void main() {
  group('journeyFrame', () {
    final from = MarketCamera(
      world: _world,
      viewport: _view,
      center: const Offset(1024, 1024),
      zoom: 1,
    );
    final plot = _snap.layout.plots.single;

    test('at_0_the_camera_is_where_it_was_and_the_room_is_hidden', () {
      final f = journeyFrame(from: from, target: plot, p: 0, fadeOnly: false);
      expect(f.camera.zoom, 1);
      expect(f.camera.center, from.center);
      expect(f.roomOpacity, 0);
      expect(f.glow, 0);
    });

    test(
      'at_half_the_camera_is_most_of_the_way_down_and_the_room_still_hidden',
      () {
        final f = journeyFrame(
          from: from,
          target: plot,
          p: 0.5,
          fadeOnly: false,
        );
        expect(f.camera.zoom, greaterThan(5));
        expect(f.camera.zoom, lessThan(flightZoom));
        expect((f.camera.center - plot.rect.center).distance, lessThan(5));
        expect(f.roomOpacity, 0);
      },
    );

    test('at_1_the_room_is_in_full_at_rest_with_no_glow', () {
      final f = journeyFrame(from: from, target: plot, p: 1, fadeOnly: false);
      expect(f.camera.zoom, flightZoom);
      expect(f.camera.center, plot.rect.center);
      expect(f.roomOpacity, 1);
      expect(f.roomScale, 1);
      expect(f.glow, 0);
    });

    test('the_warm_light_peaks_as_the_door_opens', () {
      final f = journeyFrame(
        from: from,
        target: plot,
        p: 0.65,
        fadeOnly: false,
      );
      expect(f.glow, greaterThan(0.1));
      expect(f.roomOpacity, 1);
    });

    test('fade_only_never_moves_the_camera', () {
      final f = journeyFrame(from: from, target: plot, p: 0.5, fadeOnly: true);
      expect(f.camera.zoom, 1);
      expect(f.roomOpacity, 0.5);
      expect(f.roomScale, 1);
      expect(f.glow, 0);
    });
  });

  testWidgets('tap_flies_in_and_lands_in_the_room_after_1300ms', (
    tester,
  ) async {
    final h = _Harness();
    await h.pump(tester);
    await tester.tapAt(p1OnScreen());
    await tester.pump();
    expect(h.journey.state, isA<FlyingIn>());
    expect(h.prefetched, contains('ada'));
    await h.advance(tester, 1300);
    expect(h.journey.state, isA<InRoom>());
    expect(roomOpacity(tester), 1);
  });

  testWidgets('room_is_built_before_it_is_visible', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    await tester.tapAt(p1OnScreen());
    await tester.pump();
    expect(find.text('ROOM ada'), findsOneWidget);
    expect(roomOpacity(tester), 0);
  });

  testWidgets('system_back_mid_flight_reverses_to_the_market', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    await tester.tapAt(p1OnScreen());
    await h.advance(tester, 500);
    await tester.binding.handlePopRoute();
    await tester.pump();
    expect(h.journey.state, isA<FlyingOut>());
    await h.advance(tester, 1100);
    expect(h.journey.state, isA<AtMarket>());
    expect(find.text('ROOM ada'), findsNothing);
  });

  testWidgets('back_in_the_room_flies_out', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    await tester.tapAt(p1OnScreen());
    await h.advance(tester, 1300);
    await tester.binding.handlePopRoute();
    await h.advance(tester, 1100);
    expect(h.journey.state, isA<AtMarket>());
  });

  testWidgets('reduced_motion_is_a_250ms_crossfade_with_no_zoom', (
    tester,
  ) async {
    final h = _Harness(flightIn: 250, flightOut: 250);
    await h.pump(tester, reduceMotion: true);
    await tester.tapAt(p1OnScreen());
    await h.advance(tester, 100);
    expect(roomOpacity(tester), closeTo(0.4, 0.01));
    await h.advance(tester, 150);
    expect(h.journey.state, isA<InRoom>());
    expect(roomOpacity(tester), 1);
  });

  testWidgets('zooming_in_near_a_store_prefetches_its_room', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    // Two fingers spreading apart around the store.
    final centre = p1OnScreen();
    final a = await tester.startGesture(centre - const Offset(20, 0));
    final b = await tester.startGesture(centre + const Offset(20, 0));
    for (var i = 0; i < 6; i++) {
      await a.moveBy(const Offset(-10, 0));
      await b.moveBy(const Offset(10, 0));
      await tester.pump();
    }
    await a.up();
    await b.up();
    await tester.pump();
    expect(h.prefetched, contains('ada'));
  });
}
