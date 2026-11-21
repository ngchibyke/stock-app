import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Where the access token lives. It is written nowhere else, never logged and
/// never put in a URL.
abstract class SessionStore {
  Future<String?> read();
  Future<void> write(String token);
  Future<void> clear();
}

const sessionTokenKey = 'session.token';

/// The token in the platform keystore (Keychain / Keystore).
class SecureSessionStore implements SessionStore {
  const SecureSessionStore([this._storage = const FlutterSecureStorage()]);

  final FlutterSecureStorage _storage;

  /// A read that throws (corrupt or unreadable keystore) answers null and
  /// clears the entry, best effort, so the user is signed out rather than stuck.
  @override
  Future<String?> read() async {
    try {
      return await _storage.read(key: sessionTokenKey);
    } catch (_) {
      try {
        await _storage.delete(key: sessionTokenKey);
      } catch (_) {
        // Best effort: the clear's own failure changes nothing for the caller.
      }
      return null;
    }
  }

  @override
  Future<void> write(String token) =>
      _storage.write(key: sessionTokenKey, value: token);

  @override
  Future<void> clear() => _storage.delete(key: sessionTokenKey);
}

final sessionStoreProvider = Provider<SessionStore>(
  (ref) => const SecureSessionStore(),
);
