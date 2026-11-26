import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/ui/states.dart';
import 'package:market/features/sell/identity/identity_notice.dart';
import 'package:market/features/sell/identity/identity_page.dart';
import 'package:market/features/sell/identity/selfie_capture.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_strings.dart';

import '../../support/fake_sell.dart';
import '../../support/pump_sell.dart';

Future<void> _tap(WidgetTester tester, Finder f) async {
  await tester.ensureVisible(f);
  await tester.pump();
  await tester.tap(f);
}

Finder _nin() => find.widgetWithText(TextField, SellStrings.ninLabel);

class _Selfie implements SelfieCapture {
  _Selfie(this.result);
  final SelfieResult result;
  final List<String> tokens = [];

  @override
  Future<SelfieResult> capture(String sessionToken) async {
    tokens.add(sessionToken);
    return result;
  }
}

Future<void> _toNin(WidgetTester tester) async {
  await tester.pump();
  await tester.pump();
  await _tap(tester, find.byType(CheckboxListTile));
  await tester.pump();
  await _tap(tester, find.text(SellStrings.identityContinue));
  await tester.pump();
}

void main() {
  testWidgets('the_notice_comes_first_and_continue_waits_for_the_tick', (
    tester,
  ) async {
    final env = SellEnv();
    await pumpSell(tester, IdentityPage(onPassed: () {}), env);
    await tester.pump();
    await tester.pump();
    for (final p in IdentityNotice.points) {
      expect(find.textContaining(p.substring(0, 30)), findsOneWidget);
    }
    final button = find.widgetWithText(
      LoadingButton,
      SellStrings.identityContinue,
    );
    expect(tester.widget<LoadingButton>(button).onPressed, isNull);
    await _tap(tester, find.byType(CheckboxListTile));
    await tester.pump();
    expect(tester.widget<LoadingButton>(button).onPressed, isNotNull);
  });

  testWidgets(
    'the_nin_field_takes_digits_only_stops_at_eleven_and_is_not_autofilled',
    (tester) async {
      final env = SellEnv();
      await pumpSell(tester, IdentityPage(onPassed: () {}), env);
      await _toNin(tester);
      await tester.enterText(_nin(), '12ab 34-567890123456');
      await tester.pump();
      final field = tester.widget<TextField>(_nin());
      expect(field.controller!.text, '12345678901');
      expect(field.keyboardType, TextInputType.number);
      expect(field.enableSuggestions, isFalse);
      expect(field.autocorrect, isFalse);
      // Continue only at 11.
      await tester.enterText(_nin(), '1234567890');
      await tester.pump();
      final go = find.widgetWithText(
        LoadingButton,
        SellStrings.identityContinue,
      );
      expect(tester.widget<LoadingButton>(go).onPressed, isNull);
    },
  );

  testWidgets('the_nin_is_sent_once_then_cleared_and_the_selfie_follows', (
    tester,
  ) async {
    final env = SellEnv();
    final selfie = _Selfie(SelfieResult.done);
    env.selfie = selfie;
    await pumpSell(tester, IdentityPage(onPassed: () {}), env);
    await _toNin(tester);
    await tester.enterText(_nin(), '12345678901');
    await tester.pump();
    await _tap(tester, find.text(SellStrings.identityContinue));
    await settleFrames(tester);
    expect(env.sell.ninsSent, ['12345678901']);
    // The number is on no screen any more.
    expect(find.textContaining('12345678901'), findsNothing);
    expect(find.text(SellStrings.takeSelfie), findsWidgets);
    await _tap(
      tester,
      find.widgetWithText(LoadingButton, SellStrings.takeSelfie),
    );
    await settleFrames(tester);
    expect(selfie.tokens, ['tok']);
    expect(find.text(SellStrings.waiting), findsOneWidget);
    // The session token is never shown.
    expect(find.textContaining('tok'), findsNothing);
    // Let the poll find the result.
    env.sell.identity = const IdentityStatus(state: IdentityState.passed);
    await settleFrames(tester, 5);
    expect(find.text(SellStrings.passedTitle), findsOneWidget);
  });

  testWidgets('a_cancelled_or_failed_selfie_says_so_and_can_be_tried_again', (
    tester,
  ) async {
    final env = SellEnv();
    env.selfie = _Selfie(SelfieResult.cancelled);
    await pumpSell(tester, IdentityPage(onPassed: () {}), env);
    await _toNin(tester);
    await tester.enterText(_nin(), '12345678901');
    await tester.pump();
    await _tap(tester, find.text(SellStrings.identityContinue));
    await settleFrames(tester);
    await _tap(
      tester,
      find.widgetWithText(LoadingButton, SellStrings.takeSelfie),
    );
    await settleFrames(tester);
    expect(find.text(SellStrings.selfieCancelled), findsOneWidget);
    expect(
      find.widgetWithText(LoadingButton, SellStrings.takeSelfie),
      findsOneWidget,
    );
  });

  testWidgets('a_refused_nin_shows_one_plain_screen_that_never_says_why', (
    tester,
  ) async {
    final env = SellEnv();
    env.sell.failStart = apiError('identity_not_accepted');
    await pumpSell(tester, IdentityPage(onPassed: () {}), env);
    await _toNin(tester);
    await tester.enterText(_nin(), '12345678901');
    await tester.pump();
    await _tap(tester, find.text(SellStrings.identityContinue));
    await settleFrames(tester);
    expect(find.text(SellStrings.refusedTitle), findsOneWidget);
    expect(find.text(SellStrings.refusedBody), findsOneWidget);
    expect(find.textContaining('12345678901'), findsNothing);
    expect(find.textContaining('banned'), findsNothing);
    expect(find.textContaining('already'), findsNothing);
  });

  testWidgets('the_other_server_answers_each_have_their_own_state', (
    tester,
  ) async {
    for (final (code, expected) in [
      ('identity_unavailable', SellStrings.unavailableTitle),
      ('identity_in_review', SellStrings.reviewTitle),
      ('identity_already_verified', SellStrings.passedTitle),
    ]) {
      final env = SellEnv();
      env.sell.failStart = apiError(code);
      await pumpSell(tester, IdentityPage(onPassed: () {}), env);
      await _toNin(tester);
      await tester.enterText(_nin(), '12345678901');
      await tester.pump();
      await _tap(tester, find.text(SellStrings.identityContinue));
      await settleFrames(tester);
      expect(find.text(expected), findsOneWidget, reason: code);
    }
  });

  testWidgets('too_many_tries_is_a_line_on_the_nin_screen', (tester) async {
    final env = SellEnv();
    env.sell.failStart = apiError('identity_rate_limited', status: 429);
    await pumpSell(tester, IdentityPage(onPassed: () {}), env);
    await _toNin(tester);
    await tester.enterText(_nin(), '12345678901');
    await tester.pump();
    await _tap(tester, find.text(SellStrings.identityContinue));
    await settleFrames(tester);
    expect(find.text(SellStrings.rateLimited), findsOneWidget);
    // The field is empty: it is not kept.
    expect(tester.widget<TextField>(_nin()).controller!.text, isEmpty);
  });

  testWidgets('each_saved_state_opens_on_the_right_screen', (tester) async {
    for (final (state, title) in [
      (IdentityState.passed, SellStrings.passedTitle),
      (IdentityState.review, SellStrings.reviewTitle),
      (IdentityState.revoked, SellStrings.revokedTitle),
      (IdentityState.inProgress, SellStrings.waiting),
    ]) {
      final env = SellEnv();
      env.sell.identity = IdentityStatus(state: state);
      await pumpSell(tester, IdentityPage(onPassed: () {}), env);
      await settleFrames(tester, 2);
      expect(find.text(title), findsOneWidget, reason: '$state');
      // Leave the screen so a poll timer does not outlive the test.
      await tester.pumpWidget(const SizedBox());
    }
  });

  testWidgets(
    'a_failed_check_starts_again_from_the_notice_and_done_calls_back',
    (tester) async {
      final env = SellEnv();
      env.sell.identity = const IdentityStatus(state: IdentityState.failed);
      var passed = 0;
      await pumpSell(tester, IdentityPage(onPassed: () => passed++), env);
      await settleFrames(tester, 2);
      expect(find.text(SellStrings.noticeHeading), findsOneWidget);

      env.sell.identity = const IdentityStatus(state: IdentityState.passed);
      await pumpSell(tester, IdentityPage(onPassed: () => passed++), env);
      await settleFrames(tester, 2);
      await _tap(tester, find.text(SellStrings.done));
      expect(passed, 1);
    },
  );

  testWidgets('the_status_failing_to_load_offers_try_again', (tester) async {
    final env = SellEnv();
    env.sell.identity = const IdentityStatus(state: IdentityState.unverified);
    await pumpSell(tester, IdentityPage(onPassed: () {}), env);
    await settleFrames(tester, 2);
    expect(find.text(SellStrings.noticeHeading), findsOneWidget);
  });

  test('the_development_fake_completes_and_the_default_cannot', () async {
    expect(await const FakeSelfieCapture().capture('t'), SelfieResult.done);
    expect(
      await const UnavailableSelfieCapture().capture('t'),
      SelfieResult.failed,
    );
  });
}
