import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/tokens.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/auth/sign_in_page.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/app_harness.dart';
import '../../support/pump_app.dart';

ApiError _error(int status, String code, {int? retry}) => ApiError(
  status: status,
  code: code,
  message: 'raw server message for $code',
  retryAfterSecs: retry,
);

class _Calls {
  final List<(CodeChallenge, String, String?)> unverified = [];
  int signUp = 0;
}

Future<_Calls> _pump(
  WidgetTester tester,
  AppHarness h, {
  double textScale = 1.0,
  Size size = const Size(400, 1000),
}) async {
  final calls = _Calls();
  await pumpApp(
    tester,
    ProviderScope(
      overrides: h.pageOverrides,
      child: SignInPage(
        onUnverified: (challenge, phone, credential) =>
            calls.unverified.add((challenge, phone, credential.password)),
        onSignUp: () => calls.signUp++,
      ),
    ),
    textScale: textScale,
    size: size,
    scaffold: false,
  );
  return calls;
}

Finder _field(String label) => find.widgetWithText(TextField, label);

Future<void> _fill(
  WidgetTester tester, {
  String phone = '0803 123 4567',
  String password = 'a-long-password',
}) async {
  await tester.enterText(_field(Strings.phoneLabel), phone);
  await tester.enterText(_field(Strings.passwordLabel), password);
}

Future<void> _tap(WidgetTester tester, Finder finder) async {
  await tester.ensureVisible(finder);
  await tester.pump();
  await tester.tap(finder);
  await tester.pump();
}

Future<void> _signIn(WidgetTester tester) async {
  await _tap(tester, find.text(Strings.signInAction));
  await tester.pump();
}

void main() {
  testWidgets(
    'wrong_password_shows_one_message_that_does_not_say_whether_the_phone_exists',
    (tester) async {
      final h = AppHarness();
      h.repo.onSignIn = (_, _) async =>
          throw _error(401, 'invalid_credentials');
      await _pump(tester, h);

      await _fill(tester, phone: '0803 123 4567');
      await _signIn(tester);
      expect(find.text(Strings.invalidCredentials), findsOneWidget);
      expect(find.textContaining('raw server message'), findsNothing);

      // Any other phone (known or not) gets exactly the same text.
      await _fill(tester, phone: '0701 987 6543');
      await _signIn(tester);
      expect(find.text(Strings.invalidCredentials), findsOneWidget);
      expect(h.repo.signInPhones, ['0803 123 4567', '0701 987 6543']);

      // The words do not claim the phone is (un)known.
      final words = Strings.invalidCredentials.toLowerCase();
      expect(words, isNot(contains('no account')));
      expect(words, isNot(contains('not registered')));
      expect(words, isNot(contains('not found')));
      expect(words, isNot(contains('exist')));
    },
  );

  testWidgets('too_many_attempts_shows_the_wait_message', (tester) async {
    final h = AppHarness();
    h.repo.onSignIn = (_, _) async =>
        throw _error(429, 'too_many_attempts', retry: 90);
    await _pump(tester, h);

    await _fill(tester);
    await _signIn(tester);

    expect(find.text(Strings.throttled(90)), findsOneWidget);
    expect(Strings.throttled(90), contains('1 minute 30 seconds'));
  });

  testWidgets(
    'an_unverified_account_goes_to_the_code_page_with_the_returned_id',
    (tester) async {
      final h = AppHarness();
      h.repo.onSignIn = (_, _) async => throw const PhoneUnverified(
        CodeChallenge(verificationId: 'v7', resendAfterSecs: 45),
      );
      final calls = await _pump(tester, h);

      await _fill(tester, password: '  spaced password ');
      await _signIn(tester);

      expect(calls.unverified, hasLength(1));
      final (challenge, phone, password) = calls.unverified.single;
      expect(challenge.verificationId, 'v7');
      expect(challenge.resendAfterSecs, 45);
      expect(phone, '0803 123 4567');
      expect(password, '  spaced password ');
      // The person is not signed in.
      final container = ProviderScope.containerOf(
        tester.element(find.byType(SignInPage)),
      );
      expect(container.read(sessionProvider).value, isA<SessionSignedOut>());
    },
  );

  testWidgets(
    'a_throttled_resend_on_an_unverified_account_shows_the_wait_and_does_not_navigate',
    (tester) async {
      final h = AppHarness();
      h.repo.onSignIn = (_, _) async =>
          throw _error(429, 'otp_throttled', retry: 45);
      final calls = await _pump(tester, h);

      await _fill(tester);
      await _signIn(tester);

      expect(find.text(Strings.throttled(45)), findsOneWidget);
      expect(calls.unverified, isEmpty);
    },
  );

  testWidgets('a_provider_outage_503_shows_the_try_again_message', (
    tester,
  ) async {
    final h = AppHarness();
    h.repo.onSignIn = (_, _) async => throw _error(503, 'otp_unavailable');
    final calls = await _pump(tester, h);

    await _fill(tester);
    await _signIn(tester);

    expect(find.text(Strings.otpUnavailable), findsOneWidget);
    expect(calls.unverified, isEmpty);
  });

  testWidgets('signing_in_twice_quickly_sends_one_request', (tester) async {
    final h = AppHarness();
    final pending = Completer<String>();
    h.repo.onSignIn = (_, _) => pending.future;
    await _pump(tester, h);

    await _fill(tester);
    await _signIn(tester);
    final button = find.byType(FilledButton);
    await tester.tap(button);
    await tester.pump();
    await tester.tap(button);
    await tester.pump();

    expect(h.repo.signInPhones, hasLength(1));
    pending.completeError(_error(401, 'invalid_credentials'));
    await tester.pump();
  });

  testWidgets('leading_and_trailing_spaces_in_the_password_are_kept', (
    tester,
  ) async {
    final h = AppHarness();
    h.repo.onSignIn = (_, _) async => throw _error(401, 'invalid_credentials');
    await _pump(tester, h);

    await _fill(tester, phone: '  0803 123 4567  ', password: '  pass word  ');
    await _signIn(tester);

    expect(h.repo.signInPasswords.single, '  pass word  ');
    // The phone is trimmed: it is not a secret and spaces mean nothing.
    expect(h.repo.signInPhones.single, '0803 123 4567');
  });

  testWidgets('offline_shows_the_offline_message_with_retry', (tester) async {
    final h = AppHarness();
    var attempts = 0;
    h.repo.onSignIn = (_, _) async {
      attempts++;
      if (attempts == 1) throw ApiError.offline();
      return 'tok';
    };
    h.adapter.reply(200, testMe);
    await _pump(tester, h);

    await _fill(tester);
    await _signIn(tester);
    expect(find.text(Strings.offlineMessage), findsOneWidget);

    await _tap(tester, find.text(Strings.retry));
    await settle(tester);

    expect(attempts, 2);
    final container = ProviderScope.containerOf(
      tester.element(find.byType(SignInPage)),
    );
    expect(container.read(sessionProvider).value, isA<SessionSignedIn>());
  });

  testWidgets('a_good_sign_in_stores_the_token_and_loads_the_session', (
    tester,
  ) async {
    final h = AppHarness();
    h.repo.onSignIn = (_, _) async => 'tok-good';
    h.adapter.reply(200, testMe);
    await _pump(tester, h);

    await _fill(tester);
    await _signIn(tester);
    await settle(tester);

    final container = ProviderScope.containerOf(
      tester.element(find.byType(SignInPage)),
    );
    expect(container.read(sessionProvider).value, isA<SessionSignedIn>());
    expect(await h.store.read(), 'tok-good');
  });

  testWidgets('invalid_input_is_refused_before_calling_the_server', (
    tester,
  ) async {
    final h = AppHarness();
    await _pump(tester, h);

    await _fill(tester, phone: '12345', password: '');
    await _signIn(tester);

    expect(find.text(Strings.phoneInvalid), findsOneWidget);
    expect(find.text(Strings.passwordRequired), findsOneWidget);
    expect(h.repo.signInPhones, isEmpty);
  });

  testWidgets('the_create_account_link_calls_back', (tester) async {
    final h = AppHarness();
    final calls = await _pump(tester, h);

    await _tap(tester, find.text(Strings.createAccountInstead));

    expect(calls.signUp, 1);
  });

  testWidgets('error_messages_use_the_error_text_colour', (tester) async {
    final h = AppHarness();
    h.repo.onSignIn = (_, _) async => throw _error(401, 'invalid_credentials');
    await _pump(tester, h);

    await _fill(tester);
    await _signIn(tester);

    final text = tester.widget<Text>(find.text(Strings.invalidCredentials));
    final tokens = tester.element(find.text(Strings.invalidCredentials)).tokens;
    expect(text.style?.color, tokens.errorText);
  });

  testWidgets('text_at_200_percent_does_not_overflow_on_sign_in', (
    tester,
  ) async {
    final h = AppHarness();
    h.repo.onSignIn = (_, _) async => throw _error(401, 'invalid_credentials');
    await _pump(tester, h, textScale: 2.0, size: const Size(320, 640));
    await _fill(tester);
    await _signIn(tester);
    expect(tester.takeException(), isNull);
  });

  testWidgets('the_sign_in_button_and_link_are_at_least_48dp_tall', (
    tester,
  ) async {
    final h = AppHarness();
    await _pump(tester, h);

    expect(
      tester.getSize(find.byType(FilledButton)).height,
      greaterThanOrEqualTo(48),
    );
    expect(
      tester
          .getSize(
            find.widgetWithText(TextButton, Strings.createAccountInstead),
          )
          .height,
      greaterThanOrEqualTo(48),
    );
  });
}
