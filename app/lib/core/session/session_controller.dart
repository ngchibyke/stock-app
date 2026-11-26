import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/api_error.dart';
import '../api/providers.dart';
import 'me.dart';
import 'session_store.dart';

sealed class SessionState {
  const SessionState();
}

class SessionSignedOut extends SessionState {
  const SessionSignedOut();
}

class SessionSignedIn extends SessionState {
  const SessionSignedIn(this.me);
  final Me me;
}

/// Signed in, but the person asked to delete the account and the cooling-off is running.
/// The server answers every route except the request itself with 403 `deletion_pending`,
/// so the app shows one screen with the date and a way to keep the account.
class SessionDeleting extends SessionState {
  const SessionDeleting(this.dueAt);
  final DateTime? dueAt;
}

/// Holds who is signed in. While it restores the stored session it is
/// loading. At start a 401 on `/me` means signed out (token cleared); any
/// other failure (offline, timeout, 5xx) is surfaced as an error and the token
/// is KEPT, because being offline is not being signed out. The UI retries with
/// `ref.invalidate(sessionProvider)`.
class SessionController extends AsyncNotifier<SessionState> {
  SessionStore get _store => ref.read(sessionStoreProvider);

  // True while build() runs, so a 401 on its own /me call is handled by
  // build() rather than by handleUnauthorized() writing state mid-build.
  bool _restoring = false;

  @override
  Future<SessionState> build() async {
    _restoring = true;
    try {
      final token = await _store.read();
      if (token == null || token.isEmpty) return const SessionSignedOut();
      try {
        return await _load();
      } on ApiError catch (e) {
        if (e.status == 401) {
          await _store.clear();
          return const SessionSignedOut();
        }
        rethrow;
      }
    } finally {
      _restoring = false;
    }
  }

  /// Who is signed in, or the waiting-for-deletion state when the server says so.
  Future<SessionState> _load() async {
    try {
      return SessionSignedIn(await _loadMe());
    } on ApiError catch (e) {
      if (e.status == 403 && e.code == 'deletion_pending') {
        return SessionDeleting(await _deletionDate());
      }
      rethrow;
    }
  }

  Future<DateTime?> _deletionDate() async {
    try {
      final json = await ref.read(apiClientProvider).get('/me/deletion');
      final due = json is Map ? json['due_at'] : null;
      return due is String ? DateTime.tryParse(due) : null;
    } on ApiError {
      return null;
    }
  }

  /// The person asked to delete the account (or the app learned it is waiting).
  void markDeleting(DateTime? dueAt) {
    state = AsyncData(SessionDeleting(dueAt));
  }

  /// The person kept the account: load who they are again.
  Future<void> keepAccount() async {
    state = AsyncData(await _load());
  }

  Future<Me> _loadMe() async {
    final json = await ref.read(apiClientProvider).get('/me');
    if (json is! Map<String, dynamic>) {
      throw ApiError(
        status: 0,
        code: 'bad_response',
        message: 'The server sent an unexpected response.',
      );
    }
    return Me.fromJson(json);
  }

  /// Stores [token] and loads `/me`. If that fails the token is cleared and
  /// the error rethrown.
  Future<void> signInWithToken(String token) async {
    await _store.write(token);
    try {
      state = AsyncData(await _load());
    } catch (_) {
      await _store.clear();
      state = const AsyncData(SessionSignedOut());
      rethrow;
    }
  }

  /// Loads `/me` again (for example after becoming a seller). A failure keeps what
  /// is held; a 401 signs out.
  Future<void> refreshMe() async {
    if (state.value is! SessionSignedIn) return;
    try {
      state = AsyncData(SessionSignedIn(await _loadMe()));
    } on ApiError catch (e) {
      if (e.status == 401) handleUnauthorized();
    }
  }

  /// Best-effort server sign-out, then clear locally whatever the result.
  Future<void> signOut() async {
    try {
      await ref.read(apiClientProvider).post('/auth/sign-out');
    } catch (_) {
      // The local session ends regardless.
    }
    await _store.clear();
    state = const AsyncData(SessionSignedOut());
  }

  /// The server rejected our token: clear and sign out, without calling it.
  void handleUnauthorized() {
    if (_restoring) return;
    state = const AsyncData(SessionSignedOut());
    unawaited(_store.clear().catchError((Object _) {}));
  }
}

final sessionProvider = AsyncNotifierProvider<SessionController, SessionState>(
  SessionController.new,
  // No automatic retry: a failed restore is surfaced at once so the UI can
  // offer its own retry (ref.invalidate), instead of loading in the background.
  retry: (retryCount, error) => null,
);

/// Wires the API client's seams to the session. Add to the root ProviderScope.
final sessionOverrides = [
  accessTokenReaderProvider.overrideWith(
    (ref) =>
        () => ref.read(sessionStoreProvider).read(),
  ),
  unauthorizedHandlerProvider.overrideWith(
    (ref) =>
        () => ref.read(sessionProvider.notifier).handleUnauthorized(),
  ),
];
