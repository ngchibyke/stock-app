import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/tokens.dart';
import 'package:market/core/ui/code_boxes.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/auth/countdown.dart';
import 'package:market/features/auth/forgot_page.dart';
import 'package:market/features/auth/pending_reset.dart';
import 'package:market/features/auth/reset_page.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/app_harness.dart';
import '../../support/pump_app.dart';

ApiError _error(int status, String code, {int? retry}) => ApiError(
  status: status,
  code: code,
  message: 'raw server message for $code',
  retryAfterSecs: retry,
);

const _challenge = CodeChallenge(verificationId: 'v1', resendAfterSecs: 30);
const _phone = '0803 123 4567';
const _goodPassword = 'a-long-password';

Finder _field(String label) => find.widgetWithText(TextField, label);

Finder get _codeField => find.descendant(
  of: find.byType(CodeBoxes),
  matching: find.byType(TextField),
);

Future<void> _tap(WidgetTester tester, Finder finder) async {
  await tester.ensureVisible(finder);
  await tester.pump();
  await tester.tap(finder);
  await tester.pump();
}

Future<void> _pumpPage(
  WidgetTester tester,
  AppHarness h,
  Widget page, {
  double textScale = 1.0,
  Size size = const Size(400, 1000),
  bool disableAnimations = false,
}) => pumpApp(
  tester,
  ProviderScope(overrides: h.pageOverrides, child: page),
  textScale: textScale,
  size: size,
  scaffold: false,
  disableAnimations: disableAnimations,
);

class _Forgot {
  final List<(CodeChallenge, String)> sent = [];
  int back = 0;
  late final ForgotPage page = ForgotPage(
    onCodeSent: (challenge, phone) => sent.add((challenge, phone)),
    onBack: () => back++,
  );
}

ResetPage _resetPage({
  CodeChallenge challenge = _challenge,
  VoidCallback? onReset,
  Clock? clock,
  Stream<void>? ticks,
}) => ResetPage(
  challenge: challenge,
  phone: _phone,
  onReset: onReset ?? () {},
  clock: clock,
  tickSource: ticks ?? const Stream<void>.empty(),
);

Future<void> _fillPasswords(
  WidgetTester tester, {
  String password = _goodPassword,
  String? confirm,
}) async {
  await tester.enterText(_field(Strings.newPasswordLabel), password);
  await tester.enterText(
    _field(Strings.confirmPasswordLabel),
    confirm ?? password,
  );
}

Future<void> _settleFor(WidgetTester tester) async {
  for (var i = 0; i < 6; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

TextButton _resendButton(WidgetTester tester) =>
    tester.widget<TextButton>(find.byKey(const ValueKey('resend-button')));

String _location(WidgetTester tester, AppHarness h) => h
    .container(tester)
    .read(routerProvider)
    .routerDelegate
    .currentConfiguration
    .last
    .matchedLocation;

List<String> _allText(WidgetTester tester) => tester
    .widgetList<Text>(find.byType(Text))
    .map((t) => t.data ?? t.textSpan?.toPlainText() ?? '')
    .toList();

/// From the sign-in page through the forgot page to the reset page.
Future<void> _openReset(
  WidgetTester tester,
  AppHarness h, {
  String phone = _phone,
}) async {
  await h.pump(tester);
  await _tap(tester, find.text(Strings.forgotLink));
  await _settleFor(tester);
  await tester.enterText(_field(Strings.phoneLabel), phone);
  await _tap(tester, find.text(Strings.forgotAction));
  await _settleFor(tester);
}

void main() {
  group('forgot page', () {
    testWidgets('the_forgot_page_gives_the_same_message_for_any_phone', (
      tester,
    ) async {
      final texts = <List<String>>[];
      final places = <String>[];
      // One phone with an account, one without: the answer is the same by
      // design, and so is everything the person sees.
      for (final (phone, id) in [
        ('0803 123 4567', 'real-id'),
        ('0809 999 9999', 'decoy-id'),
      ]) {
        final h = AppHarness();
        h.repo.onRequestReset = (_) async =>
            CodeChallenge(verificationId: id, resendAfterSecs: 30);
        await _openReset(tester, h, phone: phone);

        expect(find.text(Strings.resetSentNotice), findsOneWidget);
        places.add(_location(tester, h));
        texts.add(_allText(tester));
        expect(h.container(tester).read(pendingResetProvider)?.phone, phone);
        await tester.pumpWidget(const SizedBox());
      }
      expect(places, ['/reset', '/reset']);
      expect(texts[0], texts[1]);
      expect(
        Strings.resetSentNotice,
        'If that number has an account, we have sent a code.',
      );
    });

    testWidgets('a_badly_formatted_phone_is_caught_before_the_call', (
      tester,
    ) async {
      final h = AppHarness();
      final forgot = _Forgot();
      await _pumpPage(tester, h, forgot.page);

      await tester.enterText(_field(Strings.phoneLabel), '12345');
      await _tap(tester, find.text(Strings.forgotAction));

      expect(find.text(Strings.phoneInvalid), findsOneWidget);
      expect(h.repo.resetPhones, isEmpty);
      expect(forgot.sent, isEmpty);
    });

    testWidgets('too_many_requests_shows_the_wait_message', (tester) async {
      final h = AppHarness();
      h.repo.onRequestReset = (_) async =>
          throw _error(429, 'too_many_attempts', retry: 90);
      final forgot = _Forgot();
      await _pumpPage(tester, h, forgot.page);

      await tester.enterText(_field(Strings.phoneLabel), _phone);
      await _tap(tester, find.text(Strings.forgotAction));
      await tester.pump();

      expect(find.text(Strings.throttled(90)), findsOneWidget);
      expect(find.textContaining('raw server message'), findsNothing);
      expect(forgot.sent, isEmpty);
    });

    testWidgets('a_second_tap_while_the_request_is_in_flight_sends_one', (
      tester,
    ) async {
      final h = AppHarness();
      h.repo.onRequestReset = (_) => Completer<CodeChallenge>().future;
      final forgot = _Forgot();
      await _pumpPage(tester, h, forgot.page);

      await tester.enterText(_field(Strings.phoneLabel), _phone);
      await _tap(tester, find.text(Strings.forgotAction));
      await tester.tap(find.byType(FilledButton));
      await tester.pump();

      expect(h.repo.resetPhones, hasLength(1));
    });

    testWidgets('offline_shows_plain_copy_with_a_retry', (tester) async {
      final h = AppHarness();
      var attempts = 0;
      h.repo.onRequestReset = (_) async {
        attempts++;
        if (attempts == 1) throw ApiError.offline();
        return _challenge;
      };
      final forgot = _Forgot();
      await _pumpPage(tester, h, forgot.page);

      await tester.enterText(_field(Strings.phoneLabel), _phone);
      await _tap(tester, find.text(Strings.forgotAction));
      await tester.pump();
      expect(find.text(Strings.offlineMessage), findsOneWidget);

      await _tap(tester, find.text(Strings.retry));
      await tester.pump();
      expect(attempts, 2);
      expect(forgot.sent.single.$2, _phone);
    });

    testWidgets('back_to_sign_in_is_a_labelled_target_of_at_least_48dp', (
      tester,
    ) async {
      final h = AppHarness();
      final forgot = _Forgot();
      await _pumpPage(tester, h, forgot.page);

      final back = find.widgetWithText(TextButton, Strings.backToSignIn);
      expect(tester.getSize(back).height, greaterThanOrEqualTo(48));
      await _tap(tester, back);
      expect(forgot.back, 1);
    });

    testWidgets('text_at_200_percent_does_not_overflow_on_the_forgot_page', (
      tester,
    ) async {
      final h = AppHarness();
      await _pumpPage(
        tester,
        h,
        _Forgot().page,
        textScale: 2.0,
        size: const Size(320, 640),
      );
      expect(tester.takeException(), isNull);
    });
  });

  group('reset page', () {
    testWidgets(
      'the_reset_page_validates_the_new_password_like_sign_up_and_the_confirmation_must_match',
      (tester) async {
        final h = AppHarness();
        h.repo.onConfirmReset = (_, _, _) async {};
        await _pumpPage(tester, h, _resetPage());
        await tester.enterText(_codeField, '123456');
        await tester.pump();

        // Nine characters: too short. Nothing is sent, and typing the code
        // first did not shout at an empty password.
        await _fillPasswords(tester, password: '123456789');
        await _tap(tester, find.text(Strings.resetAction));
        expect(find.text(Strings.passwordTooShort), findsOneWidget);

        // 129 characters: too long.
        await _fillPasswords(tester, password: 'x' * 129);
        await _tap(tester, find.text(Strings.resetAction));
        expect(find.text(Strings.passwordTooLong), findsOneWidget);

        // A good password that the confirmation does not repeat.
        await _fillPasswords(tester, confirm: 'another-password');
        await _tap(tester, find.text(Strings.resetAction));
        expect(find.text(Strings.passwordsDoNotMatch), findsOneWidget);

        expect(h.repo.resetCodes, isEmpty);

        // An incomplete code.
        await tester.pumpWidget(const SizedBox());
        await _pumpPage(tester, h, _resetPage());
        await tester.enterText(_codeField, '123');
        await _fillPasswords(tester);
        await _tap(tester, find.text(Strings.resetAction));
        expect(find.text(Strings.codeIncomplete), findsOneWidget);
        expect(h.repo.resetCodes, isEmpty);
      },
    );

    testWidgets(
      'a_wrong_or_expired_code_shows_one_message_and_keeps_the_fields',
      (tester) async {
        final h = AppHarness();
        h.repo.onConfirmReset = (_, _, _) async =>
            throw _error(422, 'code_invalid');
        await _pumpPage(tester, h, _resetPage());

        await _fillPasswords(tester);
        await tester.enterText(_codeField, '123456');
        await _settleFor(tester);

        expect(find.text(Strings.resetCodeInvalid), findsOneWidget);
        expect(find.textContaining('raw server message'), findsNothing);
        // The fields stay as they were.
        expect(find.text('1'), findsOneWidget);
        expect(find.text('6'), findsOneWidget);
        expect(
          tester
              .widget<TextField>(_field(Strings.newPasswordLabel))
              .controller!
              .text,
          _goodPassword,
        );
        expect(
          tester
              .widget<TextField>(_field(Strings.confirmPasswordLabel))
              .controller!
              .text,
          _goodPassword,
        );
        // Typing again clears the message.
        await tester.enterText(_codeField, '12345');
        await tester.pump();
        expect(find.text(Strings.resetCodeInvalid), findsNothing);
      },
    );

    testWidgets(
      'a_weak_password_rejected_by_the_server_keeps_the_code_field_so_it_can_be_retried',
      (tester) async {
        final h = AppHarness();
        var calls = 0;
        h.repo.onConfirmReset = (_, _, _) async {
          calls++;
          if (calls == 1) throw _error(422, 'validation_failed');
        };
        var done = 0;
        await _pumpPage(tester, h, _resetPage(onReset: () => done++));

        await _fillPasswords(tester, password: 'password12');
        await tester.enterText(_codeField, '123456');
        await _settleFor(tester);

        expect(find.text(Strings.resetPasswordRejected), findsOneWidget);
        expect(find.text(Strings.resetCodeInvalid), findsNothing);
        expect(find.text('1'), findsOneWidget);
        expect(done, 0);

        // The same code goes again with a better password, without retyping.
        await _fillPasswords(tester, password: 'a-much-better-pass');
        await _tap(tester, find.text(Strings.resetAction));
        await _settleFor(tester);

        expect(h.repo.resetCodes, ['123456', '123456']);
        expect(h.repo.resetIds, ['v1', 'v1']);
        expect(h.repo.resetPasswords.last, 'a-much-better-pass');
        expect(done, 1);
      },
    );

    testWidgets('resend_waits_for_the_deadline', (tester) async {
      final h = AppHarness();
      var now = DateTime(2026, 1, 1, 12);
      final ticks = StreamController<void>.broadcast(sync: true);
      addTearDown(ticks.close);
      h.repo.onRequestReset = (_) async =>
          const CodeChallenge(verificationId: 'v2', resendAfterSecs: 60);
      h.repo.onConfirmReset = (_, _, _) async {};
      await _pumpPage(
        tester,
        h,
        _resetPage(clock: () => now, ticks: ticks.stream),
      );

      expect(find.text(Strings.resendIn(30)), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNull);

      now = now.add(const Duration(seconds: 10));
      ticks.add(null);
      await tester.pump();
      expect(find.text(Strings.resendIn(20)), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNull);
      expect(h.repo.resetPhones, isEmpty);

      now = now.add(const Duration(seconds: 21));
      ticks.add(null);
      await tester.pump();
      expect(find.text(Strings.resend), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNotNull);

      await _tap(tester, find.byKey(const ValueKey('resend-button')));
      await tester.pump();

      // Asked again for the same phone, the new challenge is adopted and the
      // wait starts over.
      expect(h.repo.resetPhones, [_phone]);
      expect(find.text(Strings.resentConfirmation), findsOneWidget);
      expect(find.text(Strings.resendIn(60)), findsOneWidget);
      expect(_resendButton(tester).onPressed, isNull);

      await _fillPasswords(tester);
      await tester.enterText(_codeField, '123456');
      await tester.pump();
      expect(h.repo.resetIds.single, 'v2');
    });

    testWidgets('a_code_typed_during_a_resend_is_submitted_when_it_ends', (
      tester,
    ) async {
      final h = AppHarness();
      var now = DateTime(2026, 1, 1, 12);
      final ticks = StreamController<void>.broadcast(sync: true);
      addTearDown(ticks.close);
      final pending = Completer<CodeChallenge>();
      h.repo.onRequestReset = (_) => pending.future;
      h.repo.onConfirmReset = (_, _, _) async =>
          throw _error(422, 'code_invalid');
      await _pumpPage(
        tester,
        h,
        _resetPage(clock: () => now, ticks: ticks.stream),
      );
      await _fillPasswords(tester);

      now = now.add(const Duration(seconds: 31));
      ticks.add(null);
      await tester.pump();
      await _tap(tester, find.byKey(const ValueKey('resend-button')));

      await tester.enterText(_codeField, '123456');
      await tester.pump();
      expect(h.repo.resetCodes, isEmpty);

      pending.complete(
        const CodeChallenge(verificationId: 'v2', resendAfterSecs: 60),
      );
      await _settleFor(tester);

      expect(h.repo.resetIds.single, 'v2');
      expect(h.repo.resetCodes.single, '123456');
      expect(find.text(Strings.resetCodeInvalid), findsOneWidget);
    });

    testWidgets(
      'a_throttled_resend_shows_the_wait_and_restarts_the_countdown',
      (tester) async {
        final h = AppHarness();
        var now = DateTime(2026, 1, 1, 12);
        final ticks = StreamController<void>.broadcast(sync: true);
        addTearDown(ticks.close);
        h.repo.onRequestReset = (_) async =>
            throw _error(429, 'too_many_attempts', retry: 90);
        await _pumpPage(
          tester,
          h,
          _resetPage(clock: () => now, ticks: ticks.stream),
        );

        now = now.add(const Duration(seconds: 31));
        ticks.add(null);
        await tester.pump();
        await _tap(tester, find.byKey(const ValueKey('resend-button')));
        await tester.pump();

        expect(find.text(Strings.throttled(90)), findsOneWidget);
        expect(find.text(Strings.resendIn(90)), findsOneWidget);
      },
    );

    testWidgets('double_submit_sends_one_request', (tester) async {
      final h = AppHarness();
      h.repo.onConfirmReset = (_, _, _) => Completer<void>().future;
      await _pumpPage(tester, h, _resetPage());

      await _fillPasswords(tester);
      // Completing the code submits it; two more taps must not send again.
      await tester.enterText(_codeField, '123456');
      await tester.pump();
      final button = find.byType(FilledButton);
      await tester.tap(button);
      await tester.pump();
      await tester.tap(button);
      await tester.pump();

      expect(h.repo.resetCodes, hasLength(1));
    });

    testWidgets('offline_shows_plain_copy_and_a_retry_that_sends_again', (
      tester,
    ) async {
      final h = AppHarness();
      var attempts = 0;
      h.repo.onConfirmReset = (_, _, _) async {
        attempts++;
        if (attempts == 1) throw ApiError.offline();
      };
      var done = 0;
      await _pumpPage(tester, h, _resetPage(onReset: () => done++));
      await _fillPasswords(tester);
      await tester.enterText(_codeField, '123456');
      await _settleFor(tester);

      expect(find.text(Strings.offlineMessage), findsOneWidget);
      expect(done, 0);

      await _tap(tester, find.text(Strings.retry));
      await _settleFor(tester);
      expect(attempts, 2);
      expect(done, 1);
    });

    testWidgets('a_server_error_5xx_shows_plain_copy', (tester) async {
      final h = AppHarness();
      h.repo.onConfirmReset = (_, _, _) async =>
          throw _error(503, 'internal_error');
      await _pumpPage(tester, h, _resetPage());
      await _fillPasswords(tester);
      await tester.enterText(_codeField, '123456');
      await _settleFor(tester);

      expect(find.text(Strings.serverDownMessage), findsOneWidget);
      expect(find.textContaining('raw server message'), findsNothing);
    });

    testWidgets('errors_are_text_in_the_error_text_colour', (tester) async {
      final h = AppHarness();
      h.repo.onConfirmReset = (_, _, _) async =>
          throw _error(503, 'internal_error');
      await _pumpPage(tester, h, _resetPage());
      await _fillPasswords(tester);
      await tester.enterText(_codeField, '123456');
      await _settleFor(tester);

      final text = tester.widget<Text>(find.text(Strings.serverDownMessage));
      final tokens = tester
          .element(find.text(Strings.serverDownMessage))
          .tokens;
      expect(text.style?.color, tokens.errorText);
    });

    testWidgets('the_new_password_is_never_trimmed_or_logged', (tester) async {
      const secret = '  spaced new pass  ';
      final logs = <String>[];
      final original = debugPrint;
      debugPrint = (String? message, {int? wrapWidth}) {
        if (message != null) logs.add(message);
      };
      addTearDown(() => debugPrint = original);

      final h = AppHarness();
      h.repo.onConfirmReset = (_, _, _) async {};
      await _pumpPage(tester, h, _resetPage());
      await _fillPasswords(tester, password: secret);
      await tester.enterText(_codeField, '123456');
      await _settleFor(tester);

      // Sent exactly as typed, spaces and all.
      expect(h.repo.resetPasswords.single, secret);
      expect(logs.where((l) => l.contains('spaced new pass')), isEmpty);

      // The hand-off never prints the phone or the id either.
      final held = PendingReset(challenge: _challenge, phone: _phone);
      expect('$held', isNot(contains('0803')));
      expect('$held', isNot(contains('v1')));
    });

    testWidgets('the_password_fields_are_wiped_when_the_page_goes_away', (
      tester,
    ) async {
      final h = AppHarness();
      await _pumpPage(tester, h, _resetPage());
      await _fillPasswords(tester);
      final controller = tester
          .widget<TextField>(_field(Strings.newPasswordLabel))
          .controller!;
      expect(controller.text, _goodPassword);

      await tester.pumpWidget(const SizedBox());
      expect(controller.text, isEmpty);
    });

    testWidgets('text_at_200_percent_does_not_overflow_on_the_reset_page', (
      tester,
    ) async {
      final h = AppHarness();
      await _pumpPage(
        tester,
        h,
        _resetPage(),
        textScale: 2.0,
        size: const Size(320, 640),
        disableAnimations: true,
      );
      expect(tester.takeException(), isNull);
      expect(find.byType(CodeBoxes), findsOneWidget);
    });
  });

  group('whole flow', () {
    testWidgets(
      'a_good_reset_returns_to_sign_in_with_a_confirmation_message_and_no_session',
      (tester) async {
        final h = AppHarness();
        h.repo.onRequestReset = (_) async => _challenge;
        h.repo.onConfirmReset = (_, _, _) async {};
        await _openReset(tester, h);
        expect(_location(tester, h), '/reset');

        await _fillPasswords(tester);
        await tester.enterText(_codeField, '123456');
        await _settleFor(tester);

        expect(find.text(Strings.signInTitle), findsOneWidget);
        expect(_location(tester, h), '/sign-in');
        expect(find.text(Strings.passwordChanged), findsOneWidget);
        final container = h.container(tester);
        expect(container.read(sessionProvider).value, isA<SessionSignedOut>());
        expect(await h.store.read(), isNull);
        expect(h.repo.signInPhones, isEmpty);
        // The hand-off with the phone is gone.
        expect(container.read(pendingResetProvider), isNull);

        // The message is shown once: leaving and coming back shows none.
        await _tap(tester, find.text(Strings.createAccountInstead));
        await _settleFor(tester);
        await tester.ensureVisible(find.text(Strings.signInInstead));
        await tester.tap(find.text(Strings.signInInstead));
        await _settleFor(tester);
        expect(find.text(Strings.signInTitle), findsOneWidget);
        expect(find.text(Strings.passwordChanged), findsNothing);
      },
    );

    testWidgets(
      'back_from_the_reset_page_returns_to_forgot_and_drops_the_hand_off',
      (tester) async {
        final h = AppHarness();
        h.repo.onRequestReset = (_) async => _challenge;
        await _openReset(tester, h);
        final container = h.container(tester);
        expect(container.read(pendingResetProvider), isNotNull);

        await tester.tap(find.byTooltip(Strings.back));
        await _settleFor(tester);

        expect(find.text(Strings.forgotTitle), findsOneWidget);
        expect(container.read(pendingResetProvider), isNull);
      },
    );
  });
}
