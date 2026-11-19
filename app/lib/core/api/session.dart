import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Where the session token lives. Only ever the token: never a name, a phone
/// number or a password.
abstract class SessionStore {
  Future<String?> read();
  Future<void> write(String token);
  Future<void> clear();
}

class SecureSessionStore implements SessionStore {
  SecureSessionStore([FlutterSecureStorage? storage])
    : _storage = storage ?? const FlutterSecureStorage();

  static const _key = 'market.session';
  final FlutterSecureStorage _storage;

  @override
  Future<String?> read() => _storage.read(key: _key);

  @override
  Future<void> write(String token) => _storage.write(key: _key, value: token);

  @override
  Future<void> clear() => _storage.delete(key: _key);
}

/// For tests and previews.
class MemorySessionStore implements SessionStore {
  MemorySessionStore([this._token]);
  String? _token;

  @override
  Future<String?> read() async => _token;

  @override
  Future<void> write(String token) async => _token = token;

  @override
  Future<void> clear() async => _token = null;
}
