import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/config/app_gate.dart';
import 'package:market/core/session/me.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/strings.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/auth/pending_credential.dart';
import 'package:market/features/auth/pending_verification.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';

import 'support/app_harness.dart';
import 'support/fakes.dart';

const _out = AsyncData<SessionState>(SessionSignedOut());
final _in = AsyncData<SessionState>(
  SessionSignedIn(Me.fromJson(Map<String, dynamic>.from(testMe))),
);

String? _go(
  String location, {
  AsyncValue<SessionState> session = _out,
  Gate gate = Gate.open,
}) => redirectFor(session: session, gate: gate, location: location);

Finder _field(String label) => find.widgetWithText(TextField, label);

String _uri(WidgetTester tester, AppHarness h) => h
    .container(tester)
    .read(routerProvider)
    .routerDelegate
    .currentConfiguration
    .last
    .matchedLocation;

void main() {
  group('redirectFor', () {
    test('loading_session_stays_on_a_splash', () {
      const loading = AsyncLoading<SessionState>();
      expect(_go('/home', session: loading), '/splash');
      expect(_go('/sign-in', session: loading), '/splash');
      expect(_go('/splash', session: loading), isNull);
      // A failed restore also shows the splash page (which has the retry).
      final failed = AsyncError<SessionState>(
        ApiError.offline(),
        StackTrace.empty,
      );
      expect(_go('/home', session: failed), '/splash');
      expect(_go('/splash', session: failed), isNull);
    });

    test('signed_out_is_sent_to_sign_in_from_any_protected_page', () {
      for (final path in [
        '/home',
        '/account',
        '/addresses',
        '/addresses/new',
      ]) {
        expect(_go(path), '/sign-in', reason: path);
      }
      expect(_go('/splash'), '/sign-in');
      expect(_go('/sign-in'), isNull);
    });

    test('signed_out_can_open_sign_up_forgot_and_reset', () {
      for (final path in ['/sign-up', '/forgot', '/reset', '/verify']) {
        expect(_go(path), isNull, reason: path);
      }
    });

    test('signed_in_is_sent_home_from_the_auth_pages', () {
      for (final path in [
        '/sign-in',
        '/sign-up',
        '/verify',
        '/forgot',
        '/reset',
        '/splash',
      ]) {
        expect(_go(path, session: _in), '/home', reason: path);
      }
      expect(_go('/home', session: _in), isNull);
      expect(_go('/account', session: _in), isNull);
      expect(_go('/addresses/new', session: _in), isNull);
    });

    test('a_gate_of_update_required_or_maintenance_overrides_every_route', () {
      for (final gate in [Gate.updateRequired, Gate.maintenance]) {
        for (final session in [_out, _in, const AsyncLoading<SessionState>()]) {
          for (final path in ['/home', '/sign-in', '/verify', '/splash']) {
            expect(
              _go(path, session: session, gate: gate),
              '/blocked',
              reason: '$gate $path',
            );
          }
          expect(_go('/blocked', session: session, gate: gate), isNull);
        }
      }
      // An open gate lets the person out of the notice.
      expect(_go('/blocked'), '/sign-in');
      expect(_go('/blocked', session: _in), '/home');
    });

    test('an_unknown_location_goes_home_or_to_sign_in', () {
      expect(_go('/nowhere'), '/sign-in');
      expect(_go('/nowhere', session: _in), '/home');
    });

    test('a_query_string_does_not_change_which_page_it_is', () {
      expect(_go('/sign-up?x=1'), isNull);
      expect(_go('/home?tab=1', session: _in), isNull);
    });
  });

  group('router wiring', () {
    testWidgets('signed_out_start_lands_on_sign_in', (tester) async {
      final h = AppHarness();
      await h.pump(tester);

      expect(find.text(Strings.signInTitle), findsOneWidget);
      expect(_uri(tester, h), '/sign-in');
    });

    testWidgets('a_stored_session_lands_on_home', (tester) async {
      final h = AppHarness(store: GatedSessionStore('tok'));
      h.adapter.reply(200, testMe);
      await h.pump(tester);
      (h.store as GatedSessionStore).open();
      await settle(tester);

      expect(find.text(Strings.home), findsOneWidget);
      expect(_uri(tester, h), '/home');
    });

    testWidgets('the_splash_is_labelled_while_the_session_restores', (
      tester,
    ) async {
      final semantics = tester.ensureSemantics();
      final store = GatedSessionStore();
      final h = AppHarness(store: store);
      await h.pump(tester);

      expect(find.bySemanticsLabel(Strings.loading), findsOneWidget);
      expect(find.text(Strings.signInTitle), findsNothing);

      store.open();
      await settle(tester);
      expect(find.bySemanticsLabel(Strings.loading), findsNothing);
      expect(find.text(Strings.signInTitle), findsOneWidget);
      semantics.dispose();
    });

    testWidgets('sign_in_links_to_sign_up_and_back', (tester) async {
      final h = AppHarness();
      await h.pump(tester);

      await tester.tap(find.text(Strings.createAccountInstead));
      await settle(tester);
      expect(find.text(Strings.signUpTitle), findsOneWidget);
      expect(_uri(tester, h), '/sign-up');

      await tester.ensureVisible(find.text(Strings.signInInstead));
      await tester.tap(find.text(Strings.signInInstead));
      await settle(tester);
      expect(find.text(Strings.signInTitle), findsOneWidget);
    });

    testWidgets('verify_without_a_pending_code_goes_to_sign_in', (
      tester,
    ) async {
      final h = AppHarness();
      await h.pump(tester);

      h.container(tester).read(routerProvider).go('/verify');
      await settle(tester);

      expect(find.text(Strings.signInTitle), findsOneWidget);
      expect(find.text(Strings.verifyTitle), findsNothing);
    });

    testWidgets(
      'an_unverified_sign_in_opens_the_code_page_without_the_password_in_the_route',
      (tester) async {
        final h = AppHarness();
        h.repo.onSignIn = (_, _) async => throw const PhoneUnverified(
          CodeChallenge(verificationId: 'v-abc', resendAfterSecs: 30),
        );
        h.repo.onVerify = (_, _) async => 'tok-v';
        h.adapter.reply(200, testMe);
        await h.pump(tester);

        await tester.enterText(_field(Strings.phoneLabel), '0803 123 4567');
        await tester.enterText(_field(Strings.passwordLabel), 'my-secret-pass');
        await tester.tap(find.text(Strings.signInAction));
        await settle(tester);

        expect(find.text(Strings.verifyTitle), findsOneWidget);
        final location = _uri(tester, h);
        expect(location, '/verify');
        expect(location, isNot(contains('my-secret-pass')));
        expect(location, isNot(contains('v-abc')));
        final container = h.container(tester);
        final pending = container.read(pendingVerificationProvider);
        expect(pending?.challenge.verificationId, 'v-abc');
        expect(pending?.credential.password, 'my-secret-pass');

        // A correct code signs in, goes home and drops the held password.
        await tester.enterText(find.byType(TextField), '123456');
        await settle(tester);
        expect(h.repo.verifiedIds.single, 'v-abc');
        expect(find.text(Strings.home), findsOneWidget);
        expect(container.read(pendingVerificationProvider), isNull);
      },
    );

    testWidgets('sign_up_opens_the_code_page_with_its_challenge', (
      tester,
    ) async {
      final h = AppHarness();
      h.repo.onSignUp = (_, _, _) async =>
          const CodeChallenge(verificationId: 'v-up', resendAfterSecs: 30);
      await h.pump(tester);

      await tester.tap(find.text(Strings.createAccountInstead));
      await settle(tester);
      await tester.enterText(_field(Strings.phoneLabel), '0803 123 4567');
      await tester.enterText(_field(Strings.displayNameLabel), 'Ada');
      await tester.enterText(_field(Strings.passwordLabel), 'a-long-password');
      await tester.ensureVisible(find.text(Strings.signUpAction));
      await tester.tap(find.text(Strings.signUpAction));
      await settle(tester);

      expect(find.text(Strings.verifyTitle), findsOneWidget);
      expect(_uri(tester, h), '/verify');
      expect(
        h.container(tester).read(pendingVerificationProvider)?.phone,
        '0803 123 4567',
      );
    });

    testWidgets('signing_out_returns_to_sign_in', (tester) async {
      final h = AppHarness(store: MemorySessionStore('tok'));
      h.adapter
        ..reply(200, testMe)
        ..reply(200, <String, dynamic>{});
      await h.pump(tester);
      expect(find.text(Strings.home), findsOneWidget);

      unawaited(h.container(tester).read(sessionProvider.notifier).signOut());
      await settle(tester);

      expect(find.text(Strings.signInTitle), findsOneWidget);
    });

    // Signs in as an unverified account so the code page is open with a live
    // credential.
    Future<void> openVerify(WidgetTester tester, AppHarness h) async {
      h.repo.onSignIn = (_, _) async => throw const PhoneUnverified(
        CodeChallenge(verificationId: 'v-abc', resendAfterSecs: 30),
      );
      await h.pump(tester);
      await tester.enterText(_field(Strings.phoneLabel), '0803 123 4567');
      await tester.enterText(_field(Strings.passwordLabel), 'my-secret-pass');
      await tester.tap(find.text(Strings.signInAction));
      await settle(tester);
      expect(find.text(Strings.verifyTitle), findsOneWidget);
    }

    testWidgets('verify_with_a_wiped_credential_redirects_to_sign_in', (
      tester,
    ) async {
      final h = AppHarness();
      await h.pump(tester);
      final container = h.container(tester);
      final credential = PendingCredential('secret-pass')..clear();
      container
          .read(pendingVerificationProvider.notifier)
          .set(
            PendingVerification(
              challenge: const CodeChallenge(
                verificationId: 'v',
                resendAfterSecs: 30,
              ),
              phone: '0803 123 4567',
              credential: credential,
            ),
          );

      container.read(routerProvider).go('/verify');
      await settle(tester);

      expect(find.text(Strings.signInTitle), findsOneWidget);
      expect(find.text(Strings.verifyTitle), findsNothing);
      expect(_uri(tester, h), '/sign-in');
    });

    testWidgets(
      'a_gate_flash_on_verify_ends_on_sign_in_not_on_a_broken_code_page',
      (tester) async {
        final h = AppHarness();
        await openVerify(tester, h);

        h.config.set(const AppConfig(maintenanceActive: true));
        await settle(tester);
        expect(find.text(Strings.maintenanceTitle), findsOneWidget);

        h.config.set(AppConfig.empty);
        await settle(tester);
        expect(find.text(Strings.signInTitle), findsOneWidget);
        expect(find.text(Strings.verifyTitle), findsNothing);
        expect(h.container(tester).read(pendingVerificationProvider), isNull);
      },
    );

    testWidgets('back_from_verify_then_verify_again_is_refused', (
      tester,
    ) async {
      final h = AppHarness();
      await openVerify(tester, h);
      final container = h.container(tester);

      await tester.tap(find.byTooltip(Strings.back));
      await settle(tester);
      expect(find.text(Strings.signInTitle), findsOneWidget);
      expect(container.read(pendingVerificationProvider), isNull);

      container.read(routerProvider).push('/verify');
      await settle(tester);

      expect(find.text(Strings.verifyTitle), findsNothing);
      expect(find.text(Strings.signInTitle), findsOneWidget);
    });
  });

  group('pending verification holder', () {
    test('it_is_cleared_when_the_person_signs_out', () async {
      final h = AppHarness(store: MemorySessionStore('tok'));
      h.adapter.reply(200, testMe);
      final container = ProviderContainer(overrides: h.pageOverrides);
      addTearDown(container.dispose);
      await container.read(sessionProvider.future);
      expect(container.read(sessionProvider).value, isA<SessionSignedIn>());

      container
          .read(pendingVerificationProvider.notifier)
          .set(
            PendingVerification(
              challenge: const CodeChallenge(
                verificationId: 'v',
                resendAfterSecs: 30,
              ),
              phone: '0803 123 4567',
              credential: PendingCredential('secret-pass'),
            ),
          );
      expect(container.read(pendingVerificationProvider), isNotNull);

      h.adapter.reply(200, <String, dynamic>{});
      await container.read(sessionProvider.notifier).signOut();

      expect(container.read(pendingVerificationProvider), isNull);
    });

    test('it_wipes_the_password_when_it_is_cleared_or_replaced', () {
      final h = AppHarness();
      final container = ProviderContainer(overrides: h.pageOverrides);
      addTearDown(container.dispose);
      final notifier = container.read(pendingVerificationProvider.notifier);
      final first = PendingCredential('one');
      final second = PendingCredential('two');
      PendingVerification make(PendingCredential c) => PendingVerification(
        challenge: const CodeChallenge(verificationId: 'v', resendAfterSecs: 1),
        phone: '0803 123 4567',
        credential: c,
      );

      notifier.set(make(first));
      notifier.set(make(second));
      expect(first.password, isNull);
      expect(second.password, 'two');

      notifier.clear();
      expect(second.password, isNull);
      expect(container.read(pendingVerificationProvider), isNull);
    });

    test('it_wipes_the_password_when_the_app_scope_goes_away', () {
      final h = AppHarness();
      final container = ProviderContainer(overrides: h.pageOverrides);
      final credential = PendingCredential('three');
      container
          .read(pendingVerificationProvider.notifier)
          .set(
            PendingVerification(
              challenge: const CodeChallenge(
                verificationId: 'v',
                resendAfterSecs: 1,
              ),
              phone: '0803 123 4567',
              credential: credential,
            ),
          );

      container.dispose();

      expect(credential.password, isNull);
    });
  });
}
