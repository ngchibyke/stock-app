import 'dart:math' as math;

import 'package:flutter/scheduler.dart';
import 'package:material_ui/material_ui.dart';

import '../../../core/theme/tokens.dart';
import '../camera/market_camera.dart';
import '../model/market.dart';
import '../view/market_view.dart';
import 'journey_controller.dart';

/// How close the camera gets to the shopfront before the room takes over.
const flightZoom = 8.0;

/// What one moment of the flight looks like.
class JourneyFrame {
  const JourneyFrame({
    required this.camera,
    required this.roomOpacity,
    required this.roomScale,
    required this.glow,
  });

  final MarketCamera camera;
  final double roomOpacity;
  final double roomScale;

  /// Opacity of the warm light laid over everything.
  final double glow;
}

double _lerp(double a, double b, double t) => a + (b - a) * t;

/// The flight at [p] (0 at the market, 1 in the room). Both ways use it: the
/// way out is the way in, backwards.
///
/// Eased p from 0 to 0.55: the camera drops from where it was to the shopfront.
/// 0.55 to 0.8: the room fades in, settling from 1.15 to 1 (the door).
/// The warm light rises with the room and fades out by 1.
JourneyFrame journeyFrame({
  required MarketCamera from,
  required Plot target,
  required double p,
  required bool fadeOnly,
}) {
  final t = p.clamp(0.0, 1.0);
  if (fadeOnly) {
    return JourneyFrame(camera: from, roomOpacity: t, roomScale: 1, glow: 0);
  }
  final e = Curves.easeInOutCubic.transform(t);
  final a = (e / 0.55).clamp(0.0, 1.0);
  final zoom = from.zoom * math.pow(flightZoom / from.zoom, a);
  final center = Offset.lerp(from.center, target.rect.center, a)!;
  final room = ((e - 0.55) / 0.25).clamp(0.0, 1.0);
  final glow = e <= 0.8 ? 0.35 * room : 0.35 * (1 - (e - 0.8) / 0.2);
  return JourneyFrame(
    camera: from.copyWith(zoom: zoom.toDouble(), center: center),
    roomOpacity: room,
    roomScale: _lerp(1.15, 1, room),
    glow: glow < 1e-6 ? 0 : glow.clamp(0.0, 0.35),
  );
}

/// The market and the way into its stores. Owns the market camera while the
/// buyer is outside, and draws the flight between [journey]'s states.
class JourneyView extends StatefulWidget {
  const JourneyView({
    super.key,
    required this.snapshot,
    required this.journey,
    required this.roomBuilder,
    required this.onPrefetch,
    this.fadeOnly = false,
  });

  final MarketSnapshot snapshot;
  final JourneyController journey;
  final Widget Function(Placement placement) roomBuilder;

  /// Asks for a store's room before the camera gets there.
  final ValueChanged<Placement> onPrefetch;

  /// A short crossfade instead of the flight (reduced motion, or a slow phone).
  final bool fadeOnly;

  @override
  State<JourneyView> createState() => _JourneyViewState();
}

class _JourneyViewState extends State<JourneyView>
    with SingleTickerProviderStateMixin {
  // Every frame of a flight is drawn from the journey's progress, and a
  // flight whose time is up lands.
  late final Ticker _ticker = createTicker((_) {
    widget.journey.tick();
    if (mounted) setState(() {});
  });
  MarketCamera? _camera;
  MarketCamera? _flightFrom;
  String? _lastPrefetch;

  @override
  void initState() {
    super.initState();
    widget.journey.addListener(_onJourney);
  }

  @override
  void didUpdateWidget(JourneyView old) {
    super.didUpdateWidget(old);
    if (old.journey != widget.journey) {
      old.journey.removeListener(_onJourney);
      widget.journey.addListener(_onJourney);
    }
  }

  @override
  void dispose() {
    widget.journey.removeListener(_onJourney);
    _ticker.dispose();
    super.dispose();
  }

  void _onJourney() {
    final state = widget.journey.state;
    if (state is FlyingIn && _flightFrom == null) _flightFrom = _camera;
    if (state is AtMarket) {
      if (_flightFrom != null) _camera = _flightFrom;
      _flightFrom = null;
      _lastPrefetch = null;
    }
    final flying = state is FlyingIn || state is FlyingOut;
    if (flying && !_ticker.isActive) _ticker.start();
    if (!flying && _ticker.isActive) _ticker.stop();
    setState(() {});
  }

  void _prefetch(Placement p) {
    if (_lastPrefetch == p.store.slug) return;
    _lastPrefetch = p.store.slug;
    widget.onPrefetch(p);
  }

  void _onCamera(MarketCamera c) {
    setState(() => _camera = c);
    final candidate = widget.journey.prefetchCandidate(c, widget.snapshot);
    if (candidate != null) _prefetch(candidate);
    widget.journey.onCamera(c, widget.snapshot);
  }

  void _openStore(Placement p) {
    _prefetch(p);
    widget.journey.requestEnter(p);
  }

  Placement? get _store => switch (widget.journey.state) {
    AtMarket() => null,
    FlyingIn(:final target) => target,
    InRoom(:final store) => store,
    FlyingOut(:final from) => from,
  };

  double get _inward => switch (widget.journey.state) {
    AtMarket() => 0,
    InRoom() => 1,
    FlyingIn() => widget.journey.progress(),
    FlyingOut() => 1 - widget.journey.progress(),
  };

  @override
  Widget build(BuildContext context) {
    final state = widget.journey.state;
    final store = _store;
    return PopScope(
      canPop: state is AtMarket,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) widget.journey.back();
      },
      child: LayoutBuilder(
        builder: (context, constraints) {
          final size = constraints.biggest;
          final world = widget.snapshot.layout.world;
          final camera =
              (_camera ??
                      MarketCamera(
                        world: world,
                        viewport: size,
                        center: world.center(Offset.zero),
                        zoom: 1,
                      ))
                  .copyWith(viewport: size);
          _camera = camera;
          final plot = store == null ? null : widget.snapshot.plotOf(store);
          final frame = plot == null
              ? null
              : journeyFrame(
                  from: _flightFrom ?? camera,
                  target: plot,
                  p: _inward,
                  fadeOnly: widget.fadeOnly,
                );
          return Stack(
            fit: StackFit.expand,
            children: [
              MarketView(
                snapshot: widget.snapshot,
                camera: frame?.camera ?? camera,
                interactive: state is AtMarket,
                onCamera: _onCamera,
                onOpenStore: _openStore,
                onPrefetch: _prefetch,
              ),
              if (store != null && frame != null)
                IgnorePointer(
                  ignoring: state is! InRoom,
                  child: Opacity(
                    opacity: frame.roomOpacity,
                    child: Transform.scale(
                      scale: frame.roomScale,
                      child: widget.roomBuilder(store),
                    ),
                  ),
                ),
              if (frame != null && frame.glow > 0)
                IgnorePointer(
                  child: ColoredBox(
                    color: context.tokens.gold.withValues(alpha: frame.glow),
                  ),
                ),
            ],
          );
        },
      ),
    );
  }
}
