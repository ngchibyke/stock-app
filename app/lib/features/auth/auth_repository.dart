import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';

/// The server's minimum gap between codes, used when a 403 `phone_unverified`
/// does not say how long to wait.
const int defaultResendAfterSecs = 60;

/// A code was sent: the id to verify it with, and how long until another may
/// be asked for.
class CodeChallenge {
  const CodeChallenge({
    required this.verificationId,
    required this.resendAfterSecs,
  });

  final String verificationId;
  final int resendAfterSecs;
}

/// Thrown by [AuthRepository.signIn] when the account exists but its phone is
/// not verified yet. The server has sent a fresh code; [challenge] carries it.
class PhoneUnverified implements Exception {
  const PhoneUnverified(this.challenge);

  final CodeChallenge challenge;

  // Never includes the verification id.
  @override
  String toString() => 'PhoneUnverified';
}

abstract class AuthRepository {
  Future<CodeChallenge> signUp({
    required String phone,
    required String password,
    required String displayName,
  });

  /// Verifies the phone code and answers the session token.
  Future<String> verifyPhone({
    required String verificationId,
    required String code,
  });

  /// Answers the token, or throws [PhoneUnverified] (403 `phone_unverified`).
  /// Other failures are an [ApiError].
  Future<String> signIn({required String phone, required String password});

  Future<CodeChallenge> requestPasswordReset(String phone);

  Future<void> confirmPasswordReset({
    required String verificationId,
    required String code,
    required String newPassword,
  });
}

/// [AuthRepository] over the server API. Passwords and codes go in the request
/// body only, exactly as given; nothing here logs or stores them.
class ApiAuthRepository implements AuthRepository {
  ApiAuthRepository(this._api);

  final ApiClient _api;

  @override
  Future<CodeChallenge> signUp({
    required String phone,
    required String password,
    required String displayName,
  }) async {
    final json = await _api.post(
      '/auth/sign-up',
      auth: false,
      body: {'phone': phone, 'password': password, 'display_name': displayName},
    );
    return _challenge(json);
  }

  @override
  Future<String> verifyPhone({
    required String verificationId,
    required String code,
  }) async {
    final json = await _api.post(
      '/auth/verify-phone',
      auth: false,
      body: {'verification_id': verificationId, 'code': code},
    );
    return _token(json);
  }

  @override
  Future<String> signIn({
    required String phone,
    required String password,
  }) async {
    try {
      final json = await _api.post(
        '/auth/sign-in',
        auth: false,
        body: {'phone': phone, 'password': password},
      );
      return _token(json);
    } on ApiError catch (e) {
      final id = e.verificationId;
      if (e.status == 403 && e.code == 'phone_unverified' && id != null) {
        throw PhoneUnverified(
          CodeChallenge(
            verificationId: id,
            resendAfterSecs: e.retryAfterSecs ?? defaultResendAfterSecs,
          ),
        );
      }
      rethrow;
    }
  }

  @override
  Future<CodeChallenge> requestPasswordReset(String phone) async {
    final json = await _api.post(
      '/auth/password-reset',
      auth: false,
      body: {'phone': phone},
    );
    return _challenge(json);
  }

  @override
  Future<void> confirmPasswordReset({
    required String verificationId,
    required String code,
    required String newPassword,
  }) async {
    await _api.post(
      '/auth/password-reset/confirm',
      auth: false,
      body: {
        'verification_id': verificationId,
        'code': code,
        'new_password': newPassword,
      },
    );
  }

  CodeChallenge _challenge(Object? json) {
    if (json is Map) {
      final id = json['verification_id'];
      final secs = json['resend_after_secs'];
      if (id is String && id.isNotEmpty) {
        return CodeChallenge(
          verificationId: id,
          resendAfterSecs: secs is num ? secs.toInt() : defaultResendAfterSecs,
        );
      }
    }
    throw _badResponse();
  }

  String _token(Object? json) {
    if (json is Map) {
      final token = json['token'];
      if (token is String && token.isNotEmpty) return token;
    }
    throw _badResponse();
  }

  ApiError _badResponse() => ApiError(
    status: 0,
    code: 'bad_response',
    message: 'The server sent an unexpected response.',
  );
}

final authRepositoryProvider = Provider<AuthRepository>(
  (ref) => ApiAuthRepository(ref.watch(apiClientProvider)),
);
