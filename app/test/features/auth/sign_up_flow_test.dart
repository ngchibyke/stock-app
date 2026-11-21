import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart' show Override;
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/session/session_store.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/tokens.dart';
import 'package:market/core/ui/code_boxes.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/auth/countdown.dart';
import 'package:market/features/auth/sign_up_page.dart';
import 'package:market/features/auth/verify_page.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/fakes.dart';
import '../../support/pump_app.dart';

const _me = {
  'id': 'u1',
  'display_name': 'Ada',
  'phone': '+2348031234567',
  'role': 'buyer',
  'is_seller': false,
};

const _tall = Size(400, 1000);

ApiError _error(int status, String code, {String? message, int? retry}) =>
    ApiError(
      status: status,
      code: code,
      message: message ?? 'raw server message for $code',
      retryAfterSecs: retry,
    );

/// A scriptable [AuthRepository] that counts calls.
class FakeAuthRepository implements AuthRepository {
  Future<CodeChallenge> Function(String phone, String password, String name)?
  onSignUp;
  Future<String> Function(String verificationId, String code)? onVerify;
  Future<String> Function(String phone, String password)? onSignIn;

  final List<String> signUpPhones = [];
  final List<String> signUpPasswords = [];
  final List<String> signUpNames = [];
  final List<String> verifiedIds = [];
  final List<String> verifiedCodes = [];
  int signInCalls = 0;

  @override
  Future<CodeChallenge> signUp({
    required String phone,
    required String password,
    required String displayName,
  }) {
    signUpPhones.add(phone);
    signUpPasswords.add(password);
    signUpNames.add(displayName);
    return onSignUp!(phone, password, displayName);
  }

  @override
  Future<String> verifyPhone({
    required String verificationId,
    required String code,
  }) {
    verifiedIds.add(verificationId);
    verifiedCodes.add(code);
    return onVerify!(verificationId, code);
  }

  @override
  Future<String> signIn({required String phone, required String password}) {
    signInCalls++;
    return onSignIn!(phone, password);
  }

  @override
  Future<CodeChallenge> requestPasswordReset(String phone) =>
      throw UnimplementedError();

  @override
  Future<void> confirmPasswordReset({
    required String verificationId,
    required String code,
    required String newPassword,
  }) => throw UnimplementedError();
}

class _Harness {
  _Harness() {
    adapter = FakeAdapter();
    store = MemorySessionStore();
  }

  late final FakeAdapter adapter;
  late final MemorySessionStore store;
  final repo = FakeAuthRepository();

  List<Override> get overrides => [
    sessionStoreProvider.overrideWithValue(store),
    ...sessionOverrides,
    apiClientProvider.overrideWith(
      (ref) => ApiClient(
        baseUrl: 'https://api.test',
        readToken: () => ref.read(accessTokenReaderProvider)(),
        onUnauthorized: () => ref.read(unauthorizedHandlerProvider)(),
        adapter: adapter,
      ),
    ),
    authRepositoryProvider.overrideWithValue(repo),
  ];

  Future<void> pump(
    WidgetTester tester,
    Widget page, {
    double textScale = 1.0,
    Size size = _tall,
  }) => pumpApp(
    tester,
    ProviderScope(overrides: overrides, child: page),
    textScale: textScale,
    size: size,
    scaffold: false,
  );
}

Finder _field(String label) => find.widgetWithText(TextField, label);

Future<void> _tap(WidgetTester tester, Finder finder) async {
  await tester.ensureVisible(finder);
  await tester.pump();
  await tester.tap(finder);
  await tester.pump();
}

/// Lets chained futures (a fake HTTP round trip, then the session load) finish.
Future<void> _settle(WidgetTester tester) async {
  // The HTTP stack schedules timers, so time has to move too.
  for (var i = 0; i < 6; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Future<void> _fillSignUp(
  WidgetTester tester, {
  String phone = '0803 123 4567',
  String name = 'Ada',
  String password = 'a-long-password',
}) async {
  await tester.enterText(_field(Strings.phoneLabel), phone);
  await tester.enterText(_field(Strings.displayNameLabel), name);
  await tester.enterText(_field(Strings.passwordLabel), password);
}

SignUpPage _signUpPage({
  void Function(CodeChallenge, String, String)? onCodeSent,
  VoidCallback? onSignIn,
}) => SignUpPage(
  onCodeSent: onCodeSent ?? (_, _, _) {},
  onSignIn: onSignIn ?? () {},
);

VerifyPage _verifyPage({
  CodeChallenge challenge = const CodeChallenge(
    verificationId: 'v1',
    resendAfterSecs: 30,
  ),
  VoidCallback? onVerified,
  Clock? clock,
  Stream<void>? ticks,
  String password = 'a-long-password',
}) => VerifyPage(
  challenge: challenge,
  phone: '0803 123 4567',
  password: password,
  onVerified: onVerified ?? () {},
  clock: clock,
  tickSource: ticks ?? const Stream<void>.empty(),
);

TextButton _resendButton(WidgetTester tester) =>
    tester.widget<TextButton>(find.byKey(const ValueKey('resend-button')));

void main() {
  group('sign up', () {
    testWidgets('sign_up_validates_before_calling_the_server', (tester) async {
      final h = _Harness();

      // A short password (9 characters).
      await h.pump(tester, _signUpPage());
      await _fillSignUp(tester, password: '123456789');
      await _tap(tester, find.text(Strings.signUpAction));
      expect(find.text(Strings.passwordTooShort), findsOneWidget);

      // An empty name.
      await _fillSignUp(tester, name: '   ');
      await _tap(tester, find.text(Strings.signUpAction));
      expect(find.text(Strings.nameRequired), findsOneWidget);

      // A phone that does not look Nigerian.
      await _fillSignUp(tester, phone: '12345');
      await _tap(tester, find.text(Strings.signUpAction));
      expect(find.text(Strings.phoneInvalid), findsOneWidget);

      expect(h.repo.signUpPhones, isEmpty);
    });

    testWidgets('a_password_over_128_characters_is_refused_locally', (
      tester,
    ) async {
      final h = _Harness();
      await h.pump(tester, _signUpPage());
      await _fillSignUp(tester, password: 'x' * 129);
      await _tap(tester, find.text(Strings.signUpAction));
      expect(find.text(Strings.passwordTooLong), findsOneWidget);
      expect(h.repo.signUpPhones, isEmpty);
    });

    testWidgets('phone_taken_shows_the_server_message_and_a_sign_in_link', (
      tester,
    ) async {
      final h = _Harness();
      h.repo.onSignUp = (_, _, _) async => throw _error(
        409,
        'phone_taken',
        message: 'That phone number already has an account.',
      );
      var signInTaps = 0;
      await h.pump(tester, _signUpPage(onSignIn: () => signInTaps++));

      await _fillSignUp(tester);
      await _tap(tester, find.text(Strings.signUpAction));
      await tester.pump();

      expect(
        find.text('That phone number already has an account.'),
        findsOneWidget,
      );
      await _tap(tester, find.text(Strings.signInInstead));
      expect(signInTaps, 1);
    });

    testWidgets('a_good_sign_up_goes_to_the_code_page', (tester) async {
      final h = _Harness();
      h.repo.onSignUp = (_, _, _) async =>
          const CodeChallenge(verificationId: 'v9', resendAfterSecs: 45);
      CodeChallenge? gotChallenge;
      String? gotPhone;
      String? gotPassword;
      await h.pump(
        tester,
        _signUpPage(
          onCodeSent: (c, phone, password) {
            gotChallenge = c;
            gotPhone = phone;
            gotPassword = password;
          },
        ),
      );

      // Name is trimmed; the password is sent exactly as typed.
      await _fillSignUp(
        tester,
        name: '  Ada  ',
        password: '  spaced password ',
      );
      await _tap(tester, find.text(Strings.signUpAction));
      await tester.pump();

      expect(h.repo.signUpNames.single, 'Ada');
      expect(h.repo.signUpPasswords.single, '  spaced password ');
      expect(gotChallenge?.verificationId, 'v9');
      expect(gotChallenge?.resendAfterSecs, 45);
      expect(gotPhone, '0803 123 4567');
      expect(gotPassword, '  spaced password ');
    });

    testWidgets('submitting_twice_sends_one_request', (tester) async {
      final h = _Harness();
      final pending = Completer<CodeChallenge>();
      h.repo.onSignUp = (_, _, _) => pending.future;
      await h.pump(tester, _signUpPage());

      await _fillSignUp(tester);
      await _tap(tester, find.text(Strings.signUpAction));
      final button = find.byType(FilledButton);
      await tester.tap(button);
      await tester.pump();
      await tester.tap(button);
      await tester.pump();

      expect(h.repo.signUpPhones, hasLength(1));
      pending.complete(
        const CodeChallenge(verificationId: 'v', resendAfterSecs: 30),
      );
      await tester.pump();
    });

    testWidgets('a_provider_outage_503_shows_the_try_again_message', (
      tester,
    ) async {
      final h = _Harness();
      h.repo.onSignUp = (_, _, _) async => throw _error(503, 'otp_unavailable');
      await h.pump(tester, _signUpPage());

      await _fillSignUp(tester);
      await _tap(tester, find.text(Strings.signUpAction));
      await tester.pump();

      expect(find.text(Strings.otpUnavailable), findsOneWidget);
      expect(find.textContaining('raw server message'), findsNothing);
    });

    testWidgets('a_throttled_sign_up_shows_the_wait_in_plain_words', (
      tester,
    ) async {
      final h = _Harness();
      h.repo.onSignUp = (_, _, _) async =>
          throw _error(429, 'otp_throttled', retry: 90);
      await h.pump(tester, _signUpPage());

      await _fillSignUp(tester);
      await _tap(tester, find.text(Strings.signUpAction));
      await tester.pump();

      expect(find.text(Strings.throttled(90)), findsOneWidget);
      expect(Strings.throttled(90), contains('1 minute 30 seconds'));
    });

    testWidgets('offline_shows_the_offline_message_with_retry', (tester) async {
      final h = _Harness();
      var attempts = 0;
      h.repo.onSignUp = (_, _, _) async {
        attempts++;
        if (attempts == 1) throw ApiError.offline();
        return const CodeChallenge(verificationId: 'v', resendAfterSecs: 30);
      };
      var sent = 0;
      await h.pump(tester, _signUpPage(onCodeSent: (_, _, _) => sent++));

      await _fillSignUp(tester);
      await _tap(tester, find.text(Strings.signUpAction));
      await tester.pump();

      expect(find.text(Strings.offlineMessage), findsOneWidget);
      await _tap(tester, find.text(Strings.retry));
      await tester.pump();

      expect(attempts, 2);
      expect(sent, 1);
    });

    testWidgets('error_messages_use_the_error_text_colour', (tester) async {
      final h = _Harness();
      h.repo.onSignUp = (_, _, _) async => throw _error(503, 'otp_unavailable');
      await h.pump(tester, _signUpPage());
      await _fillSignUp(tester);
      await _tap(tester, find.text(Strings.signUpAction));
      await tester.pump();

      final text = tester.widget<Text>(find.text(Strings.otpUnavailable));
      final tokens = tester.element(find.text(Strings.otpUnavailable)).tokens;
      expect(text.style?.color, tokens.errorText);
    });

    testWidgets('text_at_200_percent_does_not_overflow_on_sign_up', (
      tester,
    ) async {
      final h = _Harness();
      await h.pump(
        tester,
        _signUpPage(),
        textScale: 2.0,
        size: const Size(320, 640),
      );
      expect(tester.takeException(), isNull);
    });
  });

  group('code page', () {
    testWidgets(
      'the_code_screen_uses_CodeBoxes_and_accepts_only_six_digits_and_strips_pasted_spaces',
      (tester) async {
        final h = _Harness();
        h.repo.onVerify = (_, _) => Completer<String>().future;
        await h.pump(tester, _verifyPage());

        expect(find.byType(CodeBoxes), findsOneWidget);
        await tester.enterText(find.byType(TextField), '12 34 56 78');
        await tester.pump();

        expect(h.repo.verifiedCodes.single, '123456');
        expect(find.text('7'), findsNothing);
      },
    );

    testWidgets('submitting_twice_sends_one_request', (tester) async {
      final h = _Harness();
      h.repo.onVerify = (_, _) => Completer<String>().future;
      await h.pump(tester, _verifyPage());

      // Completing the code submits it; tapping Verify twice more must not
      // send again while that request is in flight.
      await tester.enterText(find.byType(TextField), '123456');
      await tester.pump();
      final button = find.byType(FilledButton);
      await tester.tap(button);
      await tester.pump();
      await tester.tap(button);
      await tester.pump();

      expect(h.repo.verifiedCodes, hasLength(1));
    });

    testWidgets(
      'wrong_expired_and_locked_codes_each_show_their_own_plain_message_and_keep_the_field',
      (tester) async {
        final h = _Harness();
        final codes = ['otp_invalid', 'otp_expired', 'otp_locked'];
        h.repo.onVerify = (_, _) async => throw _error(422, codes.removeAt(0));
        await h.pump(tester, _verifyPage());

        await tester.enterText(find.byType(TextField), '123456');
        await tester.pump();
        expect(find.text(Strings.codeWrong), findsOneWidget);
        expect(find.text('1'), findsOneWidget);
        expect(find.textContaining('raw server message'), findsNothing);

        await tester.enterText(find.byType(TextField), '654321');
        await tester.pump();
        expect(find.text(Strings.codeWrong), findsNothing);
        await _tap(tester, find.text(Strings.verifyAction));
        await tester.pump();
        expect(find.text(Strings.codeExpired), findsOneWidget);
        expect(find.text('6'), findsOneWidget);

        await _tap(tester, find.text(Strings.verifyAction));
        await tester.pump();
        expect(find.text(Strings.codeLocked), findsOneWidget);
        expect(find.text('6'), findsOneWidget);

        expect({
          Strings.codeWrong,
          Strings.codeExpired,
          Strings.codeLocked,
        }, hasLength(3));
      },
    );

    testWidgets('verify_with_an_incomplete_code_asks_for_all_digits', (
      tester,
    ) async {
      final h = _Harness();
      h.repo.onVerify = (_, _) async => 'tok';
      await h.pump(tester, _verifyPage());

      await tester.enterText(find.byType(TextField), '123');
      await _tap(tester, find.text(Strings.verifyAction));
      await tester.pump();

      expect(find.text(Strings.codeIncomplete), findsOneWidget);
      expect(h.repo.verifiedCodes, isEmpty);
    });

    testWidgets('resend_is_disabled_until_the_deadline_and_enabled_after', (
      tester,
    ) async {
      final h = _Harness();
      var now = DateTime(2026, 1, 1, 12);
      final ticks = StreamController<void>.broadcast(sync: true);
      addTearDown(ticks.close);
      await h.pump(tester, _verifyPage(clock: () => now, ticks: ticks.stream));

      expect(find.text(Strings.resendIn(30)), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNull);

      now = now.add(const Duration(seconds: 10));
      ticks.add(null);
      await tester.pump();
      expect(find.text(Strings.resendIn(20)), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNull);

      now = now.add(const Duration(seconds: 20));
      ticks.add(null);
      await tester.pump();
      expect(find.text(Strings.resend), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNotNull);
    });

    testWidgets('the_countdown_survives_the_app_being_backgrounded', (
      tester,
    ) async {
      final h = _Harness();
      var now = DateTime(2026, 1, 1, 12);
      await h.pump(tester, _verifyPage(clock: () => now));
      expect(_resendButton(tester).onPressed, isNull);

      // The app is paused: no ticks arrive, but time passes well beyond the
      // wait. Coming back to the foreground the resend is available.
      for (final s in [
        AppLifecycleState.inactive,
        AppLifecycleState.hidden,
        AppLifecycleState.paused,
      ]) {
        tester.binding.handleAppLifecycleStateChanged(s);
      }
      now = now.add(const Duration(minutes: 5));
      for (final s in [
        AppLifecycleState.hidden,
        AppLifecycleState.inactive,
        AppLifecycleState.resumed,
      ]) {
        tester.binding.handleAppLifecycleStateChanged(s);
      }
      await tester.pump();

      expect(find.text(Strings.resend), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNotNull);
    });

    testWidgets(
      'a_resend_adopts_the_new_verification_id_and_restarts_the_wait',
      (tester) async {
        final h = _Harness();
        var now = DateTime(2026, 1, 1, 12);
        final ticks = StreamController<void>.broadcast(sync: true);
        addTearDown(ticks.close);
        h.repo.onSignIn = (_, _) async => throw const PhoneUnverified(
          CodeChallenge(verificationId: 'v2', resendAfterSecs: 60),
        );
        h.repo.onVerify = (_, _) async => throw _error(422, 'otp_invalid');
        await h.pump(
          tester,
          _verifyPage(clock: () => now, ticks: ticks.stream),
        );

        now = now.add(const Duration(seconds: 31));
        ticks.add(null);
        await tester.pump();
        await _tap(tester, find.byKey(const ValueKey('resend-button')));
        await tester.pump();

        expect(h.repo.signInCalls, 1);
        expect(find.text(Strings.resentConfirmation), findsOneWidget);
        expect(find.text(Strings.resendIn(60)), findsOneWidget);
        expect(_resendButton(tester).onPressed, isNull);

        await tester.enterText(find.byType(TextField), '123456');
        await tester.pump();
        expect(h.repo.verifiedIds.single, 'v2');
      },
    );

    testWidgets(
      'a_throttled_resend_shows_the_wait_from_retry_after_secs_and_sends_nothing',
      (tester) async {
        final h = _Harness();
        var now = DateTime(2026, 1, 1, 12);
        final ticks = StreamController<void>.broadcast(sync: true);
        addTearDown(ticks.close);
        h.repo.onSignIn = (_, _) async =>
            throw _error(429, 'otp_throttled', retry: 90);
        await h.pump(
          tester,
          _verifyPage(clock: () => now, ticks: ticks.stream),
        );

        now = now.add(const Duration(seconds: 31));
        ticks.add(null);
        await tester.pump();
        await _tap(tester, find.byKey(const ValueKey('resend-button')));
        await tester.pump();

        expect(find.text(Strings.throttled(90)), findsOneWidget);
        expect(find.text(Strings.resentConfirmation), findsNothing);
        expect(h.repo.verifiedCodes, isEmpty);
        expect(find.text(Strings.resendIn(90)), findsOneWidget);
        expect(_resendButton(tester).onPressed, isNull);
      },
    );

    testWidgets('a_provider_outage_503_shows_the_try_again_message', (
      tester,
    ) async {
      final h = _Harness();
      var now = DateTime(2026, 1, 1, 12);
      final ticks = StreamController<void>.broadcast(sync: true);
      addTearDown(ticks.close);
      h.repo.onSignIn = (_, _) async => throw _error(503, 'otp_unavailable');
      await h.pump(tester, _verifyPage(clock: () => now, ticks: ticks.stream));

      now = now.add(const Duration(seconds: 31));
      ticks.add(null);
      await tester.pump();
      await _tap(tester, find.byKey(const ValueKey('resend-button')));
      await tester.pump();

      expect(find.text(Strings.otpUnavailable), findsOneWidget);
    });

    testWidgets('a_verified_account_signs_in_and_leaves_the_flow', (
      tester,
    ) async {
      final h = _Harness();
      h.adapter.reply(200, _me);
      h.repo.onVerify = (_, _) async => 'tok-verified';
      var left = 0;
      await h.pump(tester, _verifyPage(onVerified: () => left++));

      await tester.enterText(find.byType(TextField), '123456');
      await tester.pump();
      await tester.pump();

      final container = ProviderScope.containerOf(
        tester.element(find.byType(VerifyPage)),
      );
      expect(container.read(sessionProvider).value, isA<SessionSignedIn>());
      expect(h.store.token, 'tok-verified');
      expect(left, 1);
    });

    testWidgets(
      'offline_shows_the_offline_message_with_retry_without_reusing_the_spent_code',
      (tester) async {
        final h = _Harness();
        h.adapter
          ..failWithConnectionError()
          ..reply(200, _me);
        h.repo.onVerify = (_, _) async => 'tok';
        var left = 0;
        await h.pump(tester, _verifyPage(onVerified: () => left++));

        await tester.enterText(find.byType(TextField), '123456');
        await tester.pump();
        await tester.pump();
        expect(find.text(Strings.offlineMessage), findsOneWidget);
        expect(left, 0);

        await _tap(tester, find.text(Strings.retry));
        await _settle(tester);

        // The code was spent once; the retry only repeats loading /me.
        expect(h.repo.verifiedCodes, hasLength(1));
        expect(left, 1);
      },
    );

    testWidgets('offline_on_verify_retries_the_code', (tester) async {
      final h = _Harness();
      var attempts = 0;
      h.repo.onVerify = (_, _) async {
        attempts++;
        if (attempts == 1) throw ApiError.offline();
        throw _error(422, 'otp_invalid');
      };
      await h.pump(tester, _verifyPage());

      await tester.enterText(find.byType(TextField), '123456');
      await tester.pump();
      expect(find.text(Strings.offlineMessage), findsOneWidget);

      await _tap(tester, find.text(Strings.retry));
      await tester.pump();
      expect(attempts, 2);
      expect(find.text(Strings.codeWrong), findsOneWidget);
    });

    testWidgets('text_at_200_percent_does_not_overflow_on_the_code_page', (
      tester,
    ) async {
      final h = _Harness();
      await h.pump(
        tester,
        _verifyPage(),
        textScale: 2.0,
        size: const Size(320, 640),
      );
      expect(tester.takeException(), isNull);
    });
  });
}
