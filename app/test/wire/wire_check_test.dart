// Wire check: drives the app's real network code against a REAL running
// server, because the unit tests use a fake HTTP adapter and cannot prove the
// wire contract.
//
// Skipped unless WIRE_BASE_URL and WIRE_OTP_LOG are set, so the normal
// `flutter test` run stays offline and green. See "Wire check" in README.md.
//
// The server must run with OTP_DEV_ECHO=1 on a scratch database, with its
// stdout redirected to the file named by WIRE_OTP_LOG. The one-time codes it
// prints are throwaway test values.
import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/config/app_gate.dart';
import 'package:market/core/session/me.dart';
import 'package:market/features/addresses/address.dart';
import 'package:market/features/addresses/address_repository.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:shared_preferences/shared_preferences.dart';

final _baseUrl = Platform.environment['WIRE_BASE_URL'] ?? '';
final _otpLog = Platform.environment['WIRE_OTP_LOG'] ?? '';
final String? _skip = (_baseUrl.isEmpty || _otpLog.isEmpty)
    ? 'set WIRE_BASE_URL and WIRE_OTP_LOG to run the wire check'
    : null;

final _codeLine = RegExp(r'OTP code:\s*(\d{6})');

List<String> _codeLines() {
  final file = File(_otpLog);
  if (!file.existsSync()) return const [];
  return [
    for (final line in file.readAsLinesSync())
      if (_codeLine.hasMatch(line)) line,
  ];
}

/// Runs [trigger] (which makes the server send a code) and answers the code
/// from the NEW line that appeared in the server log afterwards.
Future<String> _codeAfter(Future<void> Function() trigger) async {
  final before = _codeLines().length;
  await trigger();
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    final lines = _codeLines();
    if (lines.length > before) {
      return _codeLine.firstMatch(lines.last)!.group(1)!;
    }
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  fail('no new "OTP code" line in the server log within 10 seconds');
}

final _random = Random.secure();
// Distinct per run, so the check can be re-run against the same scratch
// database. Valid Nigerian mobile numbers: 0803 + 7 digits.
final int _phoneBase =
    (DateTime.now().millisecondsSinceEpoch ~/ 1000 % 1000000) * 10;
int _phoneCount = 0;
String _nextPhone() =>
    '0803${(_phoneBase + _phoneCount++).toString().padLeft(7, '0')}';

String _password() =>
    'Wc-${List.generate(12, (_) => _random.nextInt(36).toRadixString(36)).join()}';

/// One app-side stack against the real server: a real [ApiClient] whose token
/// comes from a variable, and the real repositories.
class _Stack {
  _Stack() {
    api = ApiClient(
      baseUrl: _baseUrl,
      readToken: () async => token,
      onUnauthorized: () => unauthorizedCount++,
    );
    auth = ApiAuthRepository(api);
    addresses = ApiAddressRepository(api);
  }

  String? token;
  int unauthorizedCount = 0;
  late final ApiClient api;
  late final ApiAuthRepository auth;
  late final ApiAddressRepository addresses;

  Future<Me> me() async {
    final json = await api.get('/me');
    return Me.fromJson((json! as Map).cast<String, dynamic>());
  }

  /// Signs up and verifies a fresh phone; leaves [token] set.
  Future<({String phone, String password})> verifiedUser(String name) async {
    final phone = _nextPhone();
    final password = _password();
    late CodeChallenge challenge;
    final code = await _codeAfter(() async {
      challenge = await auth.signUp(
        phone: phone,
        password: password,
        displayName: name,
      );
    });
    token = await auth.verifyPhone(
      verificationId: challenge.verificationId,
      code: code,
    );
    return (phone: phone, password: password);
  }
}

/// A check that, on any difference between what the app expects and what the
/// server does, prints a `MISMATCH:` line and fails with a clear message.
void _wire(String name, Future<void> Function() body) {
  test(name, () async {
    try {
      await body();
    } catch (e) {
      // ignore: avoid_print
      print('MISMATCH: $name: $e');
      fail('MISMATCH in $name: $e');
    }
  }, skip: _skip);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() {
    // flutter test blocks real network access by default.
    HttpOverrides.global = null;
  });

  _wire('app_config_parses_the_real_answer', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final s = _Stack();
    final repo = AppConfigRepository(s.api, prefs);

    // The raw body first, so a transport problem is not hidden by the
    // repository's fail-open fallback.
    final raw = await s.api.get('/app/config', auth: false);
    expect(raw, isA<Map>(), reason: 'GET /app/config must answer an object');

    final config = await repo.load();
    expect(config.maintenanceActive, isFalse);
    expect(
      repo.lastGood,
      isNotNull,
      reason: 'a good answer must have been stored',
    );
    for (final platform in ['android', 'ios']) {
      expect(
        evaluateGate(config, platform: platform, version: '1.0.0'),
        Gate.open,
      );
    }
  });

  _wire('sign_up_then_verify_then_me', () async {
    final s = _Stack();
    final phone = _nextPhone();
    final password = _password();
    late CodeChallenge challenge;
    final code = await _codeAfter(() async {
      challenge = await s.auth.signUp(
        phone: phone,
        password: password,
        displayName: 'Wire Check',
      );
    });
    expect(challenge.verificationId, isNotEmpty);
    expect(challenge.resendAfterSecs, greaterThan(0));

    final token = await s.auth.verifyPhone(
      verificationId: challenge.verificationId,
      code: code,
    );
    expect(token, isNotEmpty);
    s.token = token;

    final me = await s.me();
    expect(me.displayName, 'Wire Check');
    expect(me.phone, isNotEmpty);
    expect(me.id, isNotEmpty);
    expect(me.role, isNotEmpty);
    expect(s.unauthorizedCount, 0);
  });

  _wire(
    'an_unverified_sign_in_is_403_with_the_top_level_verification_id',
    () async {
      final s = _Stack();
      final phone = _nextPhone();
      final password = _password();
      await s.auth.signUp(
        phone: phone,
        password: password,
        displayName: 'Wire',
      );

      // The raw answer, to pin the exact shape the app depends on.
      try {
        await s.api.post(
          '/auth/sign-in',
          auth: false,
          body: {'phone': phone, 'password': password},
        );
        fail('an unverified sign-in must not succeed');
      } on ApiError catch (e) {
        expect(e.status, 403);
        expect(e.code, 'phone_unverified');
        expect(
          e.verificationId,
          isNotNull,
          reason: 'verification_id must be at the TOP LEVEL of the 403 body',
        );
      }

      // And through the repository.
      try {
        await s.auth.signIn(phone: phone, password: password);
        fail('an unverified sign-in must not succeed');
      } on PhoneUnverified catch (e) {
        expect(e.challenge.verificationId, isNotEmpty);
        expect(e.challenge.resendAfterSecs, greaterThan(0));
      }
      expect(s.unauthorizedCount, 0);
    },
  );

  _wire('a_throttled_sign_up_reclaim_is_429_with_retry_after_secs', () async {
    final s = _Stack();
    final phone = _nextPhone();
    await s.auth.signUp(
      phone: phone,
      password: _password(),
      displayName: 'Wire',
    );
    try {
      await s.auth.signUp(
        phone: phone,
        password: _password(),
        displayName: 'Wire Two',
      );
      fail('a second sign-up within 60 seconds must be throttled');
    } on ApiError catch (e) {
      expect(e.status, 429);
      expect(e.code, 'otp_throttled');
      expect(e.retryAfterSecs, isNotNull);
      expect(e.retryAfterSecs, greaterThan(0));
    }
  });

  _wire(
    'a_wrong_password_is_401_and_does_not_fire_the_unauthorized_handler',
    () async {
      final s = _Stack();
      final user = await s.verifiedUser('Wire');
      s.token = null;
      try {
        await s.auth.signIn(phone: user.phone, password: '${user.password}-x');
        fail('a wrong password must not sign in');
      } on ApiError catch (e) {
        expect(e.status, 401);
        expect(e.code, 'invalid_credentials');
      }
      expect(s.unauthorizedCount, 0);
    },
  );

  _wire('addresses_create_list_delete', () async {
    final s = _Stack();
    await s.verifiedUser('Wire');

    final created = await s.addresses.create(
      const NewAddress(
        recipientName: 'Ada Wire',
        phone: '08031234567',
        addressLine: '12 Test Street, Wuse',
        landmark: 'Near the test market',
        areaCouncil: AreaCouncil.amac,
      ),
    );
    expect(created.id, isNotEmpty);
    expect(created.areaCouncil, AreaCouncil.amac);
    expect(created.recipientName, 'Ada Wire');
    expect(created.addressLine, '12 Test Street, Wuse');
    expect(created.landmark, 'Near the test market');
    expect(created.phone, isNotEmpty);

    var list = await s.addresses.list();
    expect(list, isNotEmpty);
    expect(list.first.id, created.id, reason: 'newest first');
    expect(list.first.areaCouncil, AreaCouncil.amac);
    expect(list.first.recipientName, 'Ada Wire');
    expect(list.first.addressLine, '12 Test Street, Wuse');
    expect(list.first.landmark, 'Near the test market');
    expect(list.first.phone, isNotEmpty);

    await s.addresses.remove(created.id);
    list = await s.addresses.list();
    expect(list.any((a) => a.id == created.id), isFalse);

    // A council the server refuses, through the raw client.
    try {
      await s.api.post(
        '/me/addresses',
        body: {
          'recipient_name': 'Ada Wire',
          'phone': '08031234567',
          'address_line': '1 Test Road',
          'area_council': 'ikeja',
        },
      );
      fail('a council outside the service area must be refused');
    } on ApiError catch (e) {
      expect(e.status, 422);
      expect(e.code, 'outside_service_area');
    }

    // A bad phone.
    try {
      await s.api.post(
        '/me/addresses',
        body: {
          'recipient_name': 'Ada Wire',
          'phone': '12345',
          'address_line': '1 Test Road',
          'area_council': 'amac',
        },
      );
      fail('a bad phone must be refused');
    } on ApiError catch (e) {
      expect(e.status, 422);
      expect(e.code, 'invalid_phone');
    }
    expect(s.unauthorizedCount, 0);
  });

  _wire('password_reset_changes_the_password_and_kills_old_sessions', () async {
    final s = _Stack();
    final user = await s.verifiedUser('Wire');
    final oldToken = s.token!;
    expect((await s.me()).phone, isNotEmpty);

    // A reset for a real account.
    late CodeChallenge challenge;
    final code = await _codeAfter(() async {
      challenge = await s.auth.requestPasswordReset(user.phone);
    });
    expect(challenge.verificationId, isNotEmpty);
    expect(challenge.resendAfterSecs, greaterThan(0));

    // A reset for a phone with no account must look the same.
    final ghost = await s.api.post(
      '/auth/password-reset',
      auth: false,
      body: {'phone': _nextPhone()},
    );
    expect(ghost, isA<Map>());
    final ghostMap = (ghost! as Map).cast<String, Object?>();
    expect(ghostMap['verification_id'], isA<String>());
    expect(
      ghostMap['resend_after_secs'],
      challenge.resendAfterSecs,
      reason: 'the unknown-phone answer must carry the same resend_after_secs',
    );
    final real = await s.api.post(
      '/auth/password-reset',
      auth: false,
      // Throttled for the real phone now; compare shapes via a second ghost.
      body: {'phone': _nextPhone()},
    );
    expect(
      (real! as Map).keys.toSet(),
      ghostMap.keys.toSet(),
      reason: 'the answer for two unknown phones must have the same keys',
    );

    // A wrong code is refused.
    final wrong = code == '000000' ? '111111' : '000000';
    final newPassword = _password();
    try {
      await s.auth.confirmPasswordReset(
        verificationId: challenge.verificationId,
        code: wrong,
        newPassword: newPassword,
      );
      fail('a wrong code must be refused');
    } on ApiError catch (e) {
      expect(e.code, 'code_invalid');
    }

    // The right code changes the password.
    await s.auth.confirmPasswordReset(
      verificationId: challenge.verificationId,
      code: code,
      newPassword: newPassword,
    );

    // The old session is dead, and the handler fired once.
    expect(s.unauthorizedCount, 0);
    s.token = oldToken;
    try {
      await s.api.get('/me');
      fail('the old token must be revoked');
    } on ApiError catch (e) {
      expect(e.status, 401);
    }
    expect(s.unauthorizedCount, 1);

    // The new password signs in; the old one does not.
    s.token = null;
    final fresh = await s.auth.signIn(phone: user.phone, password: newPassword);
    expect(fresh, isNotEmpty);
    try {
      await s.auth.signIn(phone: user.phone, password: user.password);
      fail('the old password must not sign in');
    } on ApiError catch (e) {
      expect(e.code, 'invalid_credentials');
    }
    expect(s.unauthorizedCount, 1);
  });

  _wire('sign_out_revokes_the_token', () async {
    final s = _Stack();
    await s.verifiedUser('Wire');
    expect((await s.me()).id, isNotEmpty);

    await s.api.post('/auth/sign-out');
    expect(s.unauthorizedCount, 0);
    try {
      await s.api.get('/me');
      fail('the token must be revoked after sign-out');
    } on ApiError catch (e) {
      expect(e.status, 401);
    }
  });
}
