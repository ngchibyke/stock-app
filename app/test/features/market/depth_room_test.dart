import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/market/perf/render_tier.dart';
import 'package:market/features/market/room/depth_room.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/fake_asset_resolver.dart';

const _view = Size(400, 800);

class _Harness {
  int loads = 0;
  Offset last = Offset.zero;

  Future<void> pump(
    WidgetTester tester, {
    RenderTier tier = RenderTier.full,
    bool reduceMotion = false,
    bool shaderFails = false,
  }) async {
    tester.view.physicalSize = _view;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      MaterialApp(
        theme: lightTheme(),
        home: MediaQuery(
          data: MediaQueryData(size: _view, disableAnimations: reduceMotion),
          child: DepthRoom(
            room: MemoryImage(onePixelPng),
            depth: MemoryImage(onePixelPng),
            tier: tier,
            loadShader: () async {
              loads++;
              if (shaderFails) throw Exception('no shader');
              return ui.FragmentProgram.fromAsset(
                'shaders/depth_parallax.frag',
              );
            },
            overlay: (look, imageRect) {
              last = look;
              return const SizedBox.expand();
            },
          ),
        ),
      ),
    );
    await tester.pump();
  }
}

void main() {
  test('the_room_picture_covers_the_screen_with_a_margin', () {
    // A 9:16 picture on a 400 x 800 screen: height decides, then 5% more.
    final r = roomImageRect(const Size(400, 800), const Size(720, 1280));
    expect(r.height, closeTo(800 * 1.05, 1e-9));
    expect(r.width, closeTo(800 * 1.05 * 720 / 1280, 1e-9));
    expect(r.center, const Offset(200, 400));
  });

  testWidgets('drag_moves_look_and_releases_back_to_centre', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    final g = await tester.startGesture(const Offset(300, 400));
    await g.moveBy(const Offset(-20, 0));
    await g.moveBy(const Offset(-80, 0));
    await tester.pump();
    // 100 px of a 400 px screen is half of the range.
    expect(h.last.dx, closeTo(-0.5, 0.06));
    await g.up();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 450));
    expect(h.last.dx, closeTo(0, 1e-6));
  });

  testWidgets('look_is_clamped_to_minus_1_1', (tester) async {
    final h = _Harness();
    await h.pump(tester);
    final g = await tester.startGesture(const Offset(390, 400));
    for (var i = 0; i < 10; i++) {
      await g.moveBy(const Offset(-80, 0));
    }
    await tester.pump();
    expect(h.last.dx, -1);
    await g.up();
  });

  testWidgets('reduced_motion_keeps_look_at_zero', (tester) async {
    final h = _Harness();
    await h.pump(tester, reduceMotion: true);
    final g = await tester.startGesture(const Offset(300, 400));
    await g.moveBy(const Offset(-100, 0));
    await tester.pump();
    expect(h.last, Offset.zero);
    await g.up();
  });

  testWidgets('shader_load_failure_falls_back_to_flat', (tester) async {
    final h = _Harness();
    await h.pump(tester, shaderFails: true);
    await tester.pump();
    expect(h.loads, 1);
    expect(find.byKey(const Key('room-flat')), findsOneWidget);
    expect(find.byKey(const Key('room-shader')), findsNothing);
  });

  testWidgets('reduced_tier_uses_flat_and_never_loads_the_shader', (
    tester,
  ) async {
    final h = _Harness();
    await h.pump(tester, tier: RenderTier.reduced);
    expect(h.loads, 0);
    expect(find.byKey(const Key('room-flat')), findsOneWidget);
  });

  testWidgets('the_flat_room_drifts_one_percent_of_the_width', (tester) async {
    final h = _Harness();
    await h.pump(tester, tier: RenderTier.reduced);
    final g = await tester.startGesture(const Offset(300, 400));
    await g.moveBy(const Offset(-20, 0));
    await g.moveBy(const Offset(-180, 0));
    await tester.pump();
    final shift = tester.widget<Transform>(
      find
          .ancestor(
            of: find.byKey(const Key('room-flat')),
            matching: find.byType(Transform),
          )
          .first,
    );
    expect(shift.transform.getTranslation().x, closeTo(-1 * 0.01 * 400, 0.5));
    await g.up();
  });
}
