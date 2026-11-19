import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/providers.dart';
import 'auth_models.dart';
import 'auth_repository.dart';

enum AuthStatus { starting, signedOut, signedIn }

class AuthState {
  const AuthState(this.status, [this.user]);
  final AuthStatus status;
  final Me? user;
}

class AuthController extends Notifier<AuthState> {
  AuthRepository get _repo => ref.read(authRepositoryProvider);

  @override
  AuthState build() => const AuthState(AuthStatus.starting);

  /// Checks a stored session once at launch.
  Future<void> start() async {
    try {
      final me = await _repo.restore();
      state = me == null
          ? const AuthState(AuthStatus.signedOut)
          : AuthState(AuthStatus.signedIn, me);
    } catch (_) {
      // Offline at launch: let them in to sign in again rather than hang on the splash.
      state = const AuthState(AuthStatus.signedOut);
    }
  }

  /// Called by the API client when the server says the session is gone.
  void sessionEnded() => state = const AuthState(AuthStatus.signedOut);

  Future<PendingVerification> signUp({
    required String phone,
    required String password,
    required String displayName,
  }) async {
    final id = await _repo.signUp(
      phone: phone,
      password: password,
      displayName: displayName,
    );
    return PendingVerification(
      verificationId: id,
      phone: phone,
      password: password,
    );
  }

  Future<void> verify(PendingVerification pending, String code) async {
    final me = await _repo.verify(
      verificationId: pending.verificationId,
      code: code,
    );
    state = AuthState(AuthStatus.signedIn, me);
  }

  /// Asks for a fresh code. Returns the pending verification with its new id.
  Future<PendingVerification> resend(PendingVerification pending) async {
    try {
      final me = await _repo.signIn(
        phone: pending.phone,
        password: pending.password,
      );
      // Already verified (the code was used elsewhere): they are simply in.
      state = AuthState(AuthStatus.signedIn, me);
      return pending;
    } on PhoneNotVerified catch (e) {
      return pending.withId(e.verificationId);
    }
  }

  /// Returns null when signed in, or the pending verification when a code is still needed.
  Future<PendingVerification?> signIn({
    required String phone,
    required String password,
  }) async {
    try {
      final me = await _repo.signIn(phone: phone, password: password);
      state = AuthState(AuthStatus.signedIn, me);
      return null;
    } on PhoneNotVerified catch (e) {
      return PendingVerification(
        verificationId: e.verificationId,
        phone: phone,
        password: password,
      );
    }
  }

  Future<void> signOut() async {
    await _repo.signOut();
    state = const AuthState(AuthStatus.signedOut);
  }
}

final authControllerProvider = NotifierProvider<AuthController, AuthState>(
  AuthController.new,
);
