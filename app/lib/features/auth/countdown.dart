import 'dart:async';

typedef Clock = DateTime Function();

/// A countdown built on a deadline, not on counted ticks, so it is right after
/// the app was in the background (timers do not run there). Ticks only tell
/// listeners to look again; [remaining] always comes from the clock.
class Countdown {
  Countdown(Duration total, {Clock? clock, Stream<void>? tickSource})
    : _clock = clock ?? DateTime.now,
      _tickSource =
          tickSource ?? Stream<void>.periodic(const Duration(seconds: 1)) {
    _deadline = _clock().add(total);
  }

  final Clock _clock;
  final Stream<void> _tickSource;
  late DateTime _deadline;

  /// Time left; never negative.
  Duration get remaining {
    final left = _deadline.difference(_clock());
    return left.isNegative ? Duration.zero : left;
  }

  bool get done => remaining == Duration.zero;

  /// Whole seconds left, rounded up, so "1s" shows until it is really over.
  int get remainingSecs => (remaining.inMilliseconds + 999) ~/ 1000;

  /// Sets a new deadline [total] from now.
  void restart(Duration total) {
    _deadline = _clock().add(total);
  }

  /// The remaining time on every tick of the tick source. Listening starts the
  /// source; cancelling the subscription stops it.
  Stream<Duration> get ticks => _tickSource.map((_) => remaining);
}
