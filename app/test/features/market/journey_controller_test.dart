import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/market/camera/market_camera.dart';
import 'package:market/features/market/journey/journey_controller.dart';
import 'package:market/features/market/model/market.dart';

import '../../support/fake_catalog_repository.dart';

const _view = Size(400, 800);

// p1 is open at (1000, 1000); p2 is open at (1400, 1000); p3 has no store.
final _snap = MarketSnapshot(
  layout: const MarketLayout(
    version: 't',
    world: Size(2048, 2048),
    tileSize: 256,
    maxLevel: 3,
    baseImage: 'base.jpg',
    districts: [District(id: 'd', name: 'D')],
    plots: [
      Plot(id: 'p1', districtId: 'd', rect: Rect.fromLTWH(950, 960, 100, 80)),
      Plot(id: 'p2', districtId: 'd', rect: Rect.fromLTWH(1350, 960, 100, 80)),
      Plot(id: 'p3', districtId: 'd', rect: Rect.fromLTWH(950, 1460, 100, 80)),
    ],
  ),
  placements: [
    Placement(plotId: 'p1', store: fakeStore('ada'), storeType: 'boutique'),
    Placement(plotId: 'p2', store: fakeStore('musa'), storeType: 'boutique'),
  ],
);

Placement get _p1 => _snap.placementFor('p1')!;
Placement get _p2 => _snap.placementFor('p2')!;

MarketCamera cam(double zoom, Offset center) => MarketCamera(
  world: const Size(2048, 2048),
  viewport: _view,
  center: center,
  zoom: zoom,
);

void main() {
  late DateTime now;
  late JourneyController j;

  setUp(() {
    now = DateTime(2026, 11, 23, 12);
    j = JourneyController(
      now: () => now,
      flightIn: const Duration(milliseconds: 1300),
      flightOut: const Duration(milliseconds: 1100),
    );
  });

  void wait(int ms) => now = now.add(Duration(milliseconds: ms));

  void leaveTheRoom() {
    j.back();
    wait(1100);
    j.tick();
  }

  test('zoom_3_4_near_centre_commits', () {
    // 0.2 x 400 = 80 px; p1 is 40 px from the centre at this scale.
    final c = cam(
      3.4,
      const Offset(1000, 1000) + const Offset(40, 0) / (400 / 2048 * 3.4),
    );
    j.onCamera(c, _snap);
    expect(j.state, isA<FlyingIn>());
    expect((j.state as FlyingIn).target.plotId, 'p1');
  });

  test('zoom_3_4_off_centre_does_not', () {
    final c = cam(
      3.4,
      const Offset(1000, 1000) + const Offset(90, 0) / (400 / 2048 * 3.4),
    );
    j.onCamera(c, _snap);
    expect(j.state, isA<AtMarket>());
  });

  test('zoom_3_3_does_not', () {
    j.onCamera(cam(3.3, const Offset(1000, 1000)), _snap);
    expect(j.state, isA<AtMarket>());
  });

  test('a_coming_soon_plot_never_commits', () {
    j.onCamera(cam(4, const Offset(1000, 1500)), _snap);
    expect(j.state, isA<AtMarket>());
  });

  test('tap_enters_with_the_same_target_path', () {
    expect(j.requestEnter(_p2), isTrue);
    expect((j.state as FlyingIn).target.plotId, 'p2');
    expect(j.progress(), 0);
  });

  test('a_second_tap_during_flight_is_ignored', () {
    j.requestEnter(_p1);
    wait(300);
    expect(j.requestEnter(_p2), isFalse);
    expect((j.state as FlyingIn).target.plotId, 'p1');
  });

  test('camera_updates_during_flight_are_ignored', () {
    j.requestEnter(_p1);
    j.onCamera(cam(4, const Offset(1400, 1000)), _snap);
    expect((j.state as FlyingIn).target.plotId, 'p1');
  });

  test('back_during_flight_in_reverses_from_current_progress', () {
    j.requestEnter(_p1);
    wait(520); // 40% of the way in
    expect(j.back(), isTrue);
    expect(j.state, isA<FlyingOut>());
    expect(j.progress(), closeTo(0.6, 1e-9));
    wait(440); // the remaining 40% of an 1100 ms flight out
    j.tick();
    expect(j.state, isA<AtMarket>());
  });

  test('back_in_room_flies_out', () {
    j.requestEnter(_p1);
    wait(1300);
    j.tick();
    expect(j.state, isA<InRoom>());
    expect(j.back(), isTrue);
    expect((j.state as FlyingOut).from.plotId, 'p1');
    expect(j.progress(), 0);
  });

  test('back_at_the_market_does_nothing', () {
    expect(j.back(), isFalse);
  });

  test('no_reentry_for_800ms_after_leaving', () {
    j.requestEnter(_p1);
    wait(1300);
    j.tick();
    leaveTheRoom();
    j.onCamera(cam(2, const Offset(1000, 1000)), _snap); // re-arm
    wait(500);
    expect(j.requestEnter(_p1), isFalse);
    j.onCamera(cam(3.5, const Offset(1000, 1000)), _snap);
    expect(j.state, isA<AtMarket>());
    wait(300);
    expect(j.requestEnter(_p1), isTrue);
  });

  test('no_reentry_until_zoom_drops_below_3', () {
    j.requestEnter(_p1);
    wait(1300);
    j.tick();
    leaveTheRoom();
    wait(900);
    j.onCamera(cam(3.6, const Offset(1000, 1000)), _snap);
    expect(j.state, isA<AtMarket>());
    j.onCamera(cam(2.9, const Offset(1000, 1000)), _snap);
    j.onCamera(cam(3.5, const Offset(1000, 1000)), _snap);
    expect(j.state, isA<FlyingIn>());
  });

  test('tick_lands_at_exactly_the_flight_time', () {
    j.requestEnter(_p1);
    wait(1299);
    j.tick();
    expect(j.state, isA<FlyingIn>());
    expect(j.progress(), closeTo(1299 / 1300, 1e-9));
    wait(1);
    j.tick();
    expect(j.state, isA<InRoom>());
    expect(j.progress(), 1);
  });

  test('listeners_hear_each_change', () {
    var heard = 0;
    j.addListener(() => heard++);
    j.requestEnter(_p1);
    wait(1300);
    j.tick();
    j.back();
    expect(heard, 3);
  });

  test('prefetch_candidate_needs_zoom_2_and_picks_the_nearest_on_screen', () {
    expect(
      j.prefetchCandidate(cam(1.9, const Offset(1100, 1000)), _snap),
      isNull,
    );
    expect(
      j.prefetchCandidate(cam(2, const Offset(1100, 1000)), _snap)!.plotId,
      'p1',
    );
    expect(
      j.prefetchCandidate(cam(2, const Offset(1300, 1000)), _snap)!.plotId,
      'p2',
    );
    // Zoomed in on the coming-soon plot, with both stores off screen.
    expect(
      j.prefetchCandidate(cam(6, const Offset(1000, 1500)), _snap),
      isNull,
    );
  });
}
