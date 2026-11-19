import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/session.dart';
import 'auth_models.dart';

abstract class AuthRepository {
  /// The signed-in person, or null if there is no good session.
  Future<Me?> restore();

  /// Creates the account; returns the verification id for the code just sent.
  Future<String> signUp({
    required String phone,
    required String password,
    required String displayName,
  });

  /// Verifies the code, stores the session, returns the person.
  Future<Me> verify({required String verificationId, required String code});

  /// Signs in. Throws [PhoneNotVerified] when the account still needs its code.
  Future<Me> signIn({required String phone, required String password});

  Future<void> signOut();
}

/// Sign-in answered 403 `phone_unverified`: a fresh code was sent.
class PhoneNotVerified implements Exception {
  const PhoneNotVerified(this.verificationId);
  final String verificationId;
}

class ApiAuthRepository implements AuthRepository {
  ApiAuthRepository(this._api, this._session);
  final ApiClient _api;
  final SessionStore _session;

  @override
  Future<Me?> restore() async {
    if (await _session.read() == null) return null;
    try {
      return Me.fromJson(await _api.get('/me') as Map<String, dynamic>);
    } on ApiError catch (e) {
      // A network failure keeps the session for next time; anything else means it is no good.
      if (e.isNetwork) {
        rethrow;
      }
      await _session.clear();
      return null;
    }
  }

  @override
  Future<String> signUp({
    required String phone,
    required String password,
    required String displayName,
  }) async {
    final res = await _api.post(
      '/auth/sign-up',
      auth: false,
      body: {'phone': phone, 'password': password, 'display_name': displayName},
    );
    return (res as Map<String, dynamic>)['verification_id'] as String;
  }

  @override
  Future<Me> verify({
    required String verificationId,
    required String code,
  }) async {
    final res = await _api.post(
      '/auth/verify-phone',
      auth: false,
      body: {'verification_id': verificationId, 'code': code},
    );
    await _session.write((res as Map<String, dynamic>)['token'] as String);
    return _me();
  }

  @override
  Future<Me> signIn({required String phone, required String password}) async {
    try {
      final res = await _api.post(
        '/auth/sign-in',
        auth: false,
        body: {'phone': phone, 'password': password},
      );
      await _session.write((res as Map<String, dynamic>)['token'] as String);
    } on ApiError catch (e) {
      final id = e.extra['verification_id'];
      if (e.code == 'phone_unverified' && id is String) {
        throw PhoneNotVerified(id);
      }
      rethrow;
    }
    return _me();
  }

  Future<Me> _me() async =>
      Me.fromJson(await _api.get('/me') as Map<String, dynamic>);

  @override
  Future<void> signOut() async {
    try {
      await _api.post('/auth/sign-out');
    } on ApiError {
      // The session may already be gone; either way we are out.
    }
    await _session.clear();
  }
}
