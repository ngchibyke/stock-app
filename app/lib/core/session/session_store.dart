import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';

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
  const SecureSessionStore([this._storage = defaultStorage]);

  /// On iOS the item stays on this device: it is not synced and not carried
  /// to another phone by a backup, so a token never signs the next owner of a
  /// device into someone else's account.
  static const defaultStorage = FlutterSecureStorage(
    iOptions: IOSOptions(
      accessibility: KeychainAccessibility.first_unlock_this_device,
    ),
  );

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

/// Set in the app's preferences once a start has run on this install.
const installedFlagKey = 'app.installed';

/// The iOS keychain outlives an uninstall but the app's preferences do not.
/// On the first start of an install, clear any token an earlier install left
/// behind, so a reinstalled app (or a phone passed on) starts signed out.
/// A failed clear is tried again on the next start and never stops the app.
Future<void> clearSessionLeftByEarlierInstall(
  SharedPreferences prefs,
  SessionStore store,
) async {
  if (prefs.getBool(installedFlagKey) ?? false) return;
  try {
    await store.clear();
    await prefs.setBool(installedFlagKey, true);
  } catch (_) {
    // Best effort; the next start tries again.
  }
}

final sessionStoreProvider = Provider<SessionStore>(
  (ref) => const SecureSessionStore(),
);
