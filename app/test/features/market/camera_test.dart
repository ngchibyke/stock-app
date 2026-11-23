import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/market/camera/market_camera.dart';
import 'package:market/features/market/camera/tiles.dart';

const _world = Size(2048, 2048);
const _view = Size(400, 800);

MarketCamera cam({
  double zoom = 1,
  Offset center = const Offset(1024, 1024),
  Size world = _world,
}) => MarketCamera(world: world, viewport: _view, center: center, zoom: zoom);

void main() {
  test('zoom_1_fits_the_world', () {
    final c = cam();
    expect(c.scale, closeTo(400 / 2048, 1e-9));
    expect(c.visibleWorld.width, closeTo(2048, 1e-6));
    expect(c.worldToScreen(const Offset(1024, 1024)), const Offset(200, 400));
  });

  test('world_to_screen_round_trips', () {
    final c = cam(zoom: 3.7, center: const Offset(700, 1300));
    const p = Offset(812.5, 1222.25);
    final back = c.screenToWorld(c.worldToScreen(p));
    expect(back.dx, closeTo(p.dx, 1e-9));
    expect(back.dy, closeTo(p.dy, 1e-9));
  });

  test('zoom_around_keeps_the_focal_point_still', () {
    final c = cam(zoom: 2, center: const Offset(900, 900));
    const focal = Offset(120, 610);
    final before = c.screenToWorld(focal);
    final z = c.zoomAround(focal, 1.8);
    expect(z.zoom, closeTo(3.6, 1e-9));
    final after = z.screenToWorld(focal);
    expect(after.dx, closeTo(before.dx, 1e-9));
    expect(after.dy, closeTo(before.dy, 1e-9));
  });

  test('pan_moves_the_world_with_the_finger', () {
    final c = cam(zoom: 4, center: const Offset(1000, 1000));
    final p = c.panBy(const Offset(30, -10));
    final moved = p.worldToScreen(c.screenToWorld(const Offset(200, 400)));
    expect(moved.dx, closeTo(230, 1e-9));
    expect(moved.dy, closeTo(390, 1e-9));
  });

  test('clamped_never_shows_outside_the_world_when_zoomed_in', () {
    final c = cam(
      zoom: 4,
      center: const Offset(-500, 3000),
    ).clamped(maxZoom: 8);
    final v = c.visibleWorld;
    expect(v.left, greaterThanOrEqualTo(-1e-9));
    expect(v.bottom, lessThanOrEqualTo(2048 + 1e-9));
    expect(cam(zoom: 0.3).clamped(maxZoom: 8).zoom, 1);
    expect(cam(zoom: 30).clamped(maxZoom: 8).zoom, 8);
  });

  test('clamped_centres_an_axis_the_view_is_wider_than', () {
    // At zoom 1 the world fits the width; the view is taller than the world.
    final c = cam(center: const Offset(300, 200)).clamped(maxZoom: 8);
    expect(c.center.dy, 1024);
    expect(c.center.dx, 1024);
  });

  test('level_for_picks_low_levels_when_whole_and_max_when_deep', () {
    expect(levelFor(cam(), maxLevel: 3, dpr: 1), 1);
    expect(levelFor(cam(), maxLevel: 3, dpr: 2.75), 3);
    expect(levelFor(cam(zoom: 8), maxLevel: 3, dpr: 1), 3);
    expect(levelFor(cam(zoom: 0.1), maxLevel: 3, dpr: 0.1), 0);
  });

  test('visible_tiles_cover_the_view_with_a_margin_and_stay_in_grid', () {
    final c = cam(zoom: 4, center: const Offset(1024, 1024));
    // Level 3: 8 x 8 tiles of 256 world units. The view is 512 x 1024 units,
    // from (768, 512) to (1280, 1536).
    final tiles = visibleTiles(c, level: 3, maxLevel: 3);
    final xs = tiles.map((t) => t.x).toSet();
    final ys = tiles.map((t) => t.y).toSet();
    expect(xs, {2, 3, 4, 5});
    expect(ys, {1, 2, 3, 4, 5, 6});
    expect(tiles.every((t) => t.level == 3), isTrue);

    final corner = cam(zoom: 4, center: Offset.zero).clamped(maxZoom: 8);
    for (final t in visibleTiles(corner, level: 3, maxLevel: 3)) {
      expect(t.x >= 0 && t.x < 8 && t.y >= 0 && t.y < 8, isTrue);
    }
    expect(
      visibleTiles(cam(), level: 0, maxLevel: 3).single,
      const TileKey(0, 0, 0),
    );
  });

  test('max_zoom_matches_the_density_rule', () {
    // A world of 8192 units on a 400-wide view at 2.75 device px per point:
    // 1.5 device px per world unit at the deepest level.
    final big = MarketCamera.maxZoomFor(const Size(8192, 8192), _view, 2.75);
    expect(big, closeTo(1.5 / ((400 / 8192) * 2.75), 1e-9));
    // A small world never caps below 4.5, so a store can always be entered.
    expect(MarketCamera.maxZoomFor(_world, _view, 2.75), 4.5);
  });
}
