import 'dart:ui';

import 'package:flutter/foundation.dart';

import '../camera/market_camera.dart';
import '../model/market.dart';

/// Where the buyer is on the way between the market and a store.
sealed class JourneyState {
  const JourneyState();
}

class AtMarket extends JourneyState {
  const AtMarket();
}

class FlyingIn extends JourneyState {
  const FlyingIn(this.target, this.started);

  final Placement target;
  final DateTime started;
}

class InRoom extends JourneyState {
  const InRoom(this.store);

  final Placement store;
}

class FlyingOut extends JourneyState {
  const FlyingOut(this.from, this.started);

  final Placement from;
  final DateTime started;
}

/// Zooming past this, with a store near the middle, flies the buyer in.
const commitZoom = 3.4;

/// After leaving, zoom must fall below this before zoom can commit again.
const rearmZoom = 3.0;

/// From this zoom, the store nearest the middle has its room fetched early.
const prefetchZoom = 2.0;

/// How near the middle (a share of the screen's shorter side) a store must be.
const commitRadius = 0.2;

/// No re-entry this soon after leaving, so the buyer is not pulled back in.
const cooldown = Duration(milliseconds: 800);

/// The rules of the camera journey, with time passed in so they can be tested.
/// Zooming and tapping take the same path in.
class JourneyController extends ChangeNotifier {
  JourneyController({
    required this.now,
    required this.flightIn,
    required this.flightOut,
  });

  final DateTime Function() now;
  final Duration flightIn;
  final Duration flightOut;

  JourneyState _state = const AtMarket();
  DateTime? _leftAt;
  bool _armed = true;

  JourneyState get state => _state;

  void _set(JourneyState s) {
    _state = s;
    notifyListeners();
  }

  bool get _coolingDown {
    final left = _leftAt;
    return left != null && now().difference(left) < cooldown;
  }

  /// The open store whose plot centre is on screen and nearest the middle.
  Placement? _nearest(
    MarketCamera c,
    MarketSnapshot snap, {
    required bool onScreenOnly,
  }) {
    final mid = c.viewport.center(Offset.zero);
    final view = Offset.zero & c.viewport;
    Placement? best;
    var bestDistance = double.infinity;
    for (final p in snap.placements) {
      final plot = snap.plotOf(p);
      if (plot == null) continue;
      final s = c.worldToScreen(plot.rect.center);
      if (onScreenOnly && !view.contains(s)) continue;
      final d = (s - mid).distance;
      if (d < bestDistance) {
        best = p;
        bestDistance = d;
      }
    }
    if (best == null) return null;
    return (onScreenOnly ||
            bestDistance <= commitRadius * c.viewport.shortestSide)
        ? best
        : null;
  }

  /// Called with every camera the buyer moves to while at the market.
  void onCamera(MarketCamera c, MarketSnapshot snap) {
    if (_state is! AtMarket) return;
    if (c.zoom < rearmZoom) _armed = true;
    if (!_armed || _coolingDown || c.zoom < commitZoom) return;
    final target = _nearest(c, snap, onScreenOnly: false);
    if (target != null) _set(FlyingIn(target, now()));
  }

  /// A tap on a store. False when not at the market or just after leaving.
  bool requestEnter(Placement p) {
    if (_state is! AtMarket || _coolingDown) return false;
    _set(FlyingIn(p, now()));
    return true;
  }

  /// Lands a flight whose time is up.
  void tick() {
    switch (_state) {
      case FlyingIn(:final target, :final started)
          when now().difference(started) >= flightIn:
        _set(InRoom(target));
      case FlyingOut(:final started)
          when now().difference(started) >= flightOut:
        _leftAt = now();
        _armed = false;
        _set(const AtMarket());
      default:
        break;
    }
  }

  /// The way back. A flight in turns round from where it is.
  bool back() {
    switch (_state) {
      case FlyingIn(:final target):
        final done = progress();
        final offset = flightOut * (1 - done);
        _set(FlyingOut(target, now().subtract(offset)));
        return true;
      case InRoom(:final store):
        _set(FlyingOut(store, now()));
        return true;
      default:
        return false;
    }
  }

  /// How far through the current flight, 0 to 1 (1 in a room, 0 at the market).
  double progress() => switch (_state) {
    AtMarket() => 0,
    InRoom() => 1,
    FlyingIn(:final started) => _fraction(started, flightIn),
    FlyingOut(:final started) => _fraction(started, flightOut),
  };

  double _fraction(DateTime started, Duration length) {
    if (length <= Duration.zero) return 1;
    final f = now().difference(started).inMicroseconds / length.inMicroseconds;
    return f.clamp(0.0, 1.0);
  }

  /// The store worth fetching early: from zoom 2, the open store on screen
  /// nearest the middle.
  Placement? prefetchCandidate(MarketCamera c, MarketSnapshot snap) {
    if (c.zoom < prefetchZoom) return null;
    return _nearest(c, snap, onScreenOnly: true);
  }
}
