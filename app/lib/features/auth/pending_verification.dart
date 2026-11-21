import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/session/session_controller.dart';
import 'auth_repository.dart';
import 'pending_credential.dart';

/// What the code page needs: the challenge, the phone as typed and the
/// password just entered (for a resend). It lives in memory only and is never
/// put in a route path, query string or `extra`.
class PendingVerification {
  const PendingVerification({
    required this.challenge,
    required this.phone,
    required this.credential,
  });

  final CodeChallenge challenge;
  final String phone;
  final PendingCredential credential;

  // Never includes the id, the phone or the password.
  @override
  String toString() => 'PendingVerification';
}

/// A short-lived hand-off from the sign-up or sign-in page to the code page.
///
/// It is set when a page hands over, read by the code route, and cleared on a
/// good verification (the person is then signed in), on sign-out and when the
/// app's scope goes away. Whenever it is cleared or replaced the password it
/// held is wiped. If the app is killed on the code page nothing is kept: the
/// person signs in again and gets a fresh code.
class PendingVerificationNotifier extends Notifier<PendingVerification?> {
  PendingVerification? _current;

  @override
  PendingVerification? build() {
    ref.listen(sessionProvider, (previous, next) {
      final wasIn = previous?.value is SessionSignedIn;
      final isIn = next.value is SessionSignedIn;
      // Signing in (the code was good) or signing out ends the hand-off.
      if (isIn || wasIn) clear();
    });
    ref.onDispose(() => _current?.credential.clear());
    return null;
  }

  void set(PendingVerification next) {
    _current?.credential.clear();
    _current = next;
    state = next;
  }

  void clear() {
    _current?.credential.clear();
    _current = null;
    if (state != null) state = null;
  }
}

final pendingVerificationProvider =
    NotifierProvider<PendingVerificationNotifier, PendingVerification?>(
      PendingVerificationNotifier.new,
    );
