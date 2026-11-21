import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/session/session_controller.dart';
import 'auth_repository.dart';

/// What the reset page needs: the challenge and the phone as typed (for a
/// resend). It lives in memory only and is never put in a route path, query
/// string or `extra`. It never holds a password.
@immutable
class PendingReset {
  const PendingReset({required this.challenge, required this.phone});

  final CodeChallenge challenge;
  final String phone;

  // Never includes the id or the phone.
  @override
  String toString() => 'PendingReset';
}

/// A short-lived hand-off from the forgot page to the reset page. The phone is
/// personal data, so it is dropped on success, when the reset page goes away,
/// on sign-in or sign-out and when the app's scope goes away.
class PendingResetNotifier extends Notifier<PendingReset?> {
  PendingReset? _current;

  @override
  PendingReset? build() {
    ref.listen(sessionProvider, (previous, next) {
      final wasIn = previous?.value is SessionSignedIn;
      final isIn = next.value is SessionSignedIn;
      if (isIn || wasIn) clear();
    });
    ref.onDispose(() => _current = null);
    return null;
  }

  @visibleForTesting
  PendingReset? get debugHeld => _current;

  void set(PendingReset next) {
    _current = next;
    state = next;
  }

  void clear() {
    _current = null;
    if (state != null) state = null;
  }

  /// Clears only if [held] is still what is held, so a page going away can
  /// never wipe a newer hand-off.
  void clearIfCurrent(PendingReset held) {
    if (identical(_current, held)) clear();
  }
}

final pendingResetProvider =
    NotifierProvider<PendingResetNotifier, PendingReset?>(
      PendingResetNotifier.new,
    );

/// Whether the sign-in page should say the password was just changed. Not
/// personal data: a plain flag, set on a good reset and consumed once.
class PasswordChangedNotice extends Notifier<bool> {
  @override
  bool build() => false;

  void show() => state = true;

  /// Reads the flag and turns it off.
  bool consume() {
    final was = state;
    if (was) state = false;
    return was;
  }
}

final passwordChangedNoticeProvider =
    NotifierProvider<PasswordChangedNotice, bool>(PasswordChangedNotice.new);
