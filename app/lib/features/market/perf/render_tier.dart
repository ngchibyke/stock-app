import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// How much the journey and rooms draw.
///
/// - [full]: the camera flight and the depth room with its shader.
/// - [reduced]: the flight, and a flat room that only drifts.
/// - [fade]: a short fade between the market and a flat room.
enum RenderTier { full, reduced, fade }

/// Frames in one measuring window.
const frameWindow = 30;

/// A 90th-percentile frame slower than this (about 45 fps) steps down a tier.
const frameLimit = Duration(milliseconds: 22);

/// Watches frame times during a flight or a room drag and steps down one tier
/// when the phone cannot keep up. It only ever steps down.
class FrameBudget {
  FrameBudget({required RenderTier start, required this.onChange})
    : _tier = start;

  final void Function(RenderTier) onChange;
  RenderTier _tier;
  final List<Duration> _window = [];

  RenderTier get tier => _tier;

  /// One frame's build plus raster time.
  void add(Duration frameTotal) {
    if (_tier == RenderTier.fade) return;
    _window.add(frameTotal);
    if (_window.length < frameWindow) return;
    final sorted = [..._window]..sort();
    _window.clear();
    final p90 = sorted[(sorted.length * 0.9).ceil() - 1];
    if (p90 > frameLimit) {
      _tier = RenderTier.values[_tier.index + 1];
      onChange(_tier);
    }
  }
}

/// The tier to draw with: reduced motion never gets the moving room.
RenderTier effectiveTier(RenderTier stored, {required bool reduceMotion}) {
  if (!reduceMotion || stored.index >= RenderTier.reduced.index) return stored;
  return RenderTier.reduced;
}

const _key = 'render_tier.v1';

/// The phone's tier, kept between launches. Nothing personal is stored.
class RenderTierNotifier extends Notifier<RenderTier> {
  late final Future<void> ready = _load();

  @override
  RenderTier build() {
    ready;
    return RenderTier.full;
  }

  Future<void> _load() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final stored = prefs.getString(_key);
      for (final t in RenderTier.values) {
        if (t.name == stored && t.index > state.index) state = t;
      }
    } on Object {
      // Without storage the phone starts at full and measures again.
    }
  }

  /// Steps down to [tier]; a tier above the current one is ignored.
  Future<void> stepDownTo(RenderTier tier) async {
    if (tier.index <= state.index) return;
    state = tier;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString(_key, tier.name);
    } on Object {
      // The tier still holds for this launch.
    }
  }
}

final renderTierProvider = NotifierProvider<RenderTierNotifier, RenderTier>(
  RenderTierNotifier.new,
);
