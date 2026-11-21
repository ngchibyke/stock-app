import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/auth/auth_repository.dart';

import '../../support/fakes.dart';

AuthRepository _repo(FakeAdapter adapter) => ApiAuthRepository(
  ApiClient(
    baseUrl: 'https://api.test',
    readToken: () async => null,
    onUnauthorized: () {},
    adapter: adapter,
  ),
);

void main() {
  test('sign_up_posts_the_exact_body_and_returns_the_challenge', () async {
    final adapter = FakeAdapter()
      ..reply(201, {
        'user_id': 'u1',
        'verification_id': 'v-123',
        'resend_after_secs': 60,
      });

    final challenge = await _repo(adapter).signUp(
      phone: '0803 123 4567',
      password: 'correct horse battery',
      displayName: 'Ada',
    );

    expect(challenge.verificationId, 'v-123');
    expect(challenge.resendAfterSecs, 60);
    final sent = adapter.requests.single;
    expect(sent.method, 'POST');
    expect(sent.path, '/auth/sign-up');
    expect(sent.body, {
      'phone': '0803 123 4567',
      'password': 'correct horse battery',
      'display_name': 'Ada',
    });
  });

  test('verify_returns_the_token', () async {
    final adapter = FakeAdapter()..reply(200, {'token': 'tok-abc'});

    final token = await _repo(adapter)
        .verifyPhone(verificationId: 'v-123', code: '123456');

    expect(token, 'tok-abc');
    final sent = adapter.requests.single;
    expect(sent.path, '/auth/verify-phone');
    expect(sent.body, {'verification_id': 'v-123', 'code': '123456'});
  });

  test('sign_in_on_an_unverified_account_throws_phone_unverified_with_the_top_level_id', () async {
    final adapter = FakeAdapter()
      ..reply(403, {
        'error': {'code': 'phone_unverified', 'message': 'Verify first.'},
        'verification_id': 'v-top',
      });

    await expectLater(
      _repo(adapter).signIn(phone: '08031234567', password: 'a-long-password'),
      throwsA(
        isA<PhoneUnverified>().having(
          (e) => e.challenge.verificationId,
          'verificationId',
          'v-top',
        ),
      ),
    );
  });

  test('sign_in_returns_the_token', () async {
    final adapter = FakeAdapter()..reply(200, {'token': 'tok-in'});
    expect(
      await _repo(adapter)
          .signIn(phone: '08031234567', password: 'a-long-password'),
      'tok-in',
    );
    expect(adapter.requests.single.path, '/auth/sign-in');
  });

  test('a_throttled_sign_up_surfaces_retry_after_secs', () async {
    final adapter = FakeAdapter()
      ..reply(429, {
        'error': {
          'code': 'otp_throttled',
          'message': 'Slow down.',
          'retry_after_secs': 42,
        },
      });

    await expectLater(
      _repo(adapter).signUp(
        phone: '08031234567',
        password: 'a-long-password',
        displayName: 'Ada',
      ),
      throwsA(
        isA<ApiError>()
            .having((e) => e.code, 'code', 'otp_throttled')
            .having((e) => e.retryAfterSecs, 'retryAfterSecs', 42),
      ),
    );
  });

  test('passwords_are_sent_exactly_as_typed', () async {
    final adapter = FakeAdapter()
      ..reply(201, {
        'user_id': 'u1',
        'verification_id': 'v',
        'resend_after_secs': 60,
      });

    await _repo(adapter).signUp(
      phone: '08031234567',
      password: '  spaced out pass  ',
      displayName: 'Ada',
    );

    expect(
      (adapter.requests.single.body! as Map)['password'],
      '  spaced out pass  ',
    );
  });

  test('password_reset_requests_use_the_reset_routes', () async {
    final adapter = FakeAdapter()
      ..reply(202, {'verification_id': 'rv', 'resend_after_secs': 60})
      ..reply(204);
    final repo = _repo(adapter);

    final challenge = await repo.requestPasswordReset('08031234567');
    await repo.confirmPasswordReset(
      verificationId: 'rv',
      code: '123456',
      newPassword: 'a-new-long-password',
    );

    expect(challenge.verificationId, 'rv');
    expect(adapter.requests[0].path, '/auth/password-reset');
    expect(adapter.requests[0].body, {'phone': '08031234567'});
    expect(adapter.requests[1].path, '/auth/password-reset/confirm');
    expect(adapter.requests[1].body, {
      'verification_id': 'rv',
      'code': '123456',
      'new_password': 'a-new-long-password',
    });
  });
}
