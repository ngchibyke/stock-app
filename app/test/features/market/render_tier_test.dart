import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/market/perf/render_tier.dart';
import 'package:shared_preferences/shared_preferences.dart';

const _fast = Duration(milliseconds: 16);
const _slow = Duration(milliseconds: 30);

void main() {
  group('FrameBudget', () {
    late List<RenderTier> changes;
    late FrameBudget budget;

    setUp(() {
      changes = [];
      budget = FrameBudget(start: RenderTier.full, onChange: changes.add);
    });

    void feed(int count, Duration frame) {
      for (var i = 0; i < count; i++) {
        budget.add(frame);
      }
    }

    test('steady_16ms_frames_stay_full', () {
      feed(300, _fast);
      expect(changes, isEmpty);
    });

    test('p90_over_22ms_steps_down_one_tier', () {
      // 4 slow frames in 30 put the 90th percentile on a slow frame.
      feed(26, _fast);
      feed(4, _slow);
      expect(changes, [RenderTier.reduced]);
    });

    test('a_few_slow_frames_are_forgiven', () {
      feed(27, _fast);
      feed(3, _slow);
      expect(changes, isEmpty);
    });

    test('a_window_needs_30_frames', () {
      feed(29, _slow);
      expect(changes, isEmpty);
      feed(1, _slow);
      expect(changes, [RenderTier.reduced]);
    });

    test('never_steps_below_fade', () {
      feed(300, _slow);
      expect(changes, [RenderTier.reduced, RenderTier.fade]);
    });
  });

  group('renderTierProvider', () {
    test('never_steps_up', () async {
      SharedPreferences.setMockInitialValues({});
      final c = ProviderContainer();
      addTearDown(c.dispose);
      final n = c.read(renderTierProvider.notifier);
      await n.ready;
      await n.stepDownTo(RenderTier.fade);
      await n.stepDownTo(RenderTier.full);
      expect(c.read(renderTierProvider), RenderTier.fade);
    });

    test('tier_survives_a_restart', () async {
      SharedPreferences.setMockInitialValues({});
      final first = ProviderContainer();
      await first.read(renderTierProvider.notifier).ready;
      await first
          .read(renderTierProvider.notifier)
          .stepDownTo(RenderTier.reduced);
      first.dispose();

      final second = ProviderContainer();
      addTearDown(second.dispose);
      await second.read(renderTierProvider.notifier).ready;
      expect(second.read(renderTierProvider), RenderTier.reduced);
    });

    test('a_corrupt_stored_tier_is_full', () async {
      SharedPreferences.setMockInitialValues({'render_tier.v1': 'shiny'});
      final c = ProviderContainer();
      addTearDown(c.dispose);
      await c.read(renderTierProvider.notifier).ready;
      expect(c.read(renderTierProvider), RenderTier.full);
    });
  });

  test('reduced_motion_caps_the_tier_at_reduced', () {
    expect(
      effectiveTier(RenderTier.full, reduceMotion: true),
      RenderTier.reduced,
    );
    expect(effectiveTier(RenderTier.fade, reduceMotion: true), RenderTier.fade);
    expect(
      effectiveTier(RenderTier.full, reduceMotion: false),
      RenderTier.full,
    );
  });
}
