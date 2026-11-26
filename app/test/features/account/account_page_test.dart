import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/config/app_gate.dart';
import 'package:market/core/links/external_link.dart';
import 'package:market/core/session/me.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/core/ui/states.dart';
import 'package:market/features/account/account_page.dart';
import 'package:market/features/sell/sell_repository.dart';

import '../../support/fake_sell.dart';

import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/auth/pending_credential.dart';
import 'package:market/features/auth/pending_reset.dart';
import 'package:market/features/auth/pending_verification.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/app_harness.dart';
import '../../support/fakes.dart';

const _fullConfig = AppConfig(
  termsUrl: 'https://example.com/terms',
  privacyUrl: 'https://example.com/privacy',
  supportEmail: 'help@example.com',
);

/// Records every link it is asked to open and answers [result].
class _Opener {
  _Opener({this.result = true, this.throws = false});

  final bool result;
  final bool throws;
  final List<Uri> opened = [];

  Future<bool> call(Uri uri) async {
    opened.add(uri);
    if (throws) throw StateError('no handler');
    return result;
  }
}

/// A session controller whose restore is scripted by the test.
class _ScriptedSession extends SessionController {
  _ScriptedSession(this._script);

  final Future<SessionState> Function() _script;

  @override
  Future<SessionState> build() => _script();
}

SessionState _signedIn() =>
    SessionSignedIn(Me.fromJson(Map<String, dynamic>.from(testMe)));

/// Opens the real app, signed in, on the Account tab.
Future<AppHarness> _openAccount(
  WidgetTester tester, {
  AppConfig config = AppConfig.empty,
  _Opener? opener,
}) async {
  final h = AppHarness(store: MemorySessionStore('tok'), config: config);
  h.adapter
    ..reply(200, testMe)
    ..reply(200, <String, dynamic>{});
  await h.pump(
    tester,
    extra: [
      if (opener != null)
        externalLinkOpenerProvider.overrideWithValue(opener.call),
    ],
  );
  await tester.tap(find.text(Strings.tabAccount));
  await settle(tester);
  return h;
}

/// The page on its own, with a scripted session.
Future<void> _pumpPage(
  WidgetTester tester,
  Future<SessionState> Function() script,
) async {
  await tester.pumpWidget(
    ProviderScope(
      retry: (_, _) => null,
      overrides: [
        sessionProvider.overrideWith(() => _ScriptedSession(script)),
        appConfigProvider.overrideWith(() => FakeConfigNotifier()),
        packageVersionProvider.overrideWith((ref) async => '1.0.0'),
        sellRepositoryProvider.overrideWithValue(FakeSellRepository()),
      ],
      child: MaterialApp(
        theme: lightTheme(),
        home: const Scaffold(body: AccountPage()),
      ),
    ),
  );
}

void main() {
  testWidgets('the_account_page_shows_name_phone_role_and_version', (
    tester,
  ) async {
    await _openAccount(tester);

    expect(find.text('Ada'), findsOneWidget);
    expect(find.text('0803 123 4567'), findsOneWidget);
    expect(find.text('+2348031234567'), findsNothing);
    expect(find.text(Strings.roleName('buyer')), findsOneWidget);
    expect(find.text(Strings.appVersion('1.0.0')), findsOneWidget);
    expect(find.text(Strings.addresses), findsOneWidget);
    expect(find.text(Strings.signOut), findsOneWidget);
  });

  group('links', () {
    testWidgets('terms_privacy_and_support_rows_appear_only_when_configured', (
      tester,
    ) async {
      await _openAccount(tester);
      expect(find.text(Strings.terms), findsNothing);
      expect(find.text(Strings.privacy), findsNothing);
      expect(find.text(Strings.contactSupport), findsNothing);
    });

    testWidgets('each_configured_row_appears_on_its_own', (tester) async {
      await _openAccount(
        tester,
        config: const AppConfig(privacyUrl: 'https://example.com/privacy'),
      );
      expect(find.text(Strings.terms), findsNothing);
      expect(find.text(Strings.privacy), findsOneWidget);
      expect(find.text(Strings.contactSupport), findsNothing);
    });

    testWidgets('configured_rows_open_outside_the_app_with_safe_uris', (
      tester,
    ) async {
      final opener = _Opener();
      await _openAccount(tester, config: _fullConfig, opener: opener);

      await tester.tap(find.text(Strings.terms));
      await tester.pump();
      await tester.tap(find.text(Strings.privacy));
      await tester.pump();
      await tester.tap(find.text(Strings.contactSupport));
      await tester.pump();

      expect(opener.opened, [
        Uri.parse('https://example.com/terms'),
        Uri.parse('https://example.com/privacy'),
        Uri.parse('mailto:help@example.com'),
      ]);
    });

    testWidgets('the_support_address_cannot_add_mail_headers', (tester) async {
      final opener = _Opener();
      await _openAccount(
        tester,
        config: const AppConfig(
          supportEmail: 'help@example.com?bcc=evil@example.com&body=x y',
        ),
        opener: opener,
      );

      await tester.tap(find.text(Strings.contactSupport));
      await tester.pump();

      final uri = opener.opened.single;
      expect(uri.scheme, 'mailto');
      expect(uri.hasQuery, isFalse);
      expect(uri.toString(), isNot(contains('?')));
      expect(uri.toString(), isNot(contains(' ')));
    });

    testWidgets('a_link_that_will_not_open_shows_a_plain_message', (
      tester,
    ) async {
      await _openAccount(
        tester,
        config: _fullConfig,
        opener: _Opener(result: false),
      );

      await tester.tap(find.text(Strings.terms));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));

      expect(find.text(Strings.linkOpenFailed), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('a_launcher_that_throws_never_crashes_the_page', (
      tester,
    ) async {
      await _openAccount(
        tester,
        config: _fullConfig,
        opener: _Opener(throws: true),
      );

      await tester.tap(find.text(Strings.contactSupport));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));

      expect(find.text(Strings.linkOpenFailed), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    for (final bad in [
      'http://example.com/terms',
      'javascript:alert(1)',
      'https:///x',
    ]) {
      for (final isTerms in [true, false]) {
        testWidgets('a_non_https_link_is_refused_$bad${isTerms ? '' : '_p'}', (
          tester,
        ) async {
          final opener = _Opener();
          await _openAccount(
            tester,
            config: isTerms
                ? AppConfig(termsUrl: bad)
                : AppConfig(privacyUrl: bad),
            opener: opener,
          );

          await tester.tap(
            find.text(isTerms ? Strings.terms : Strings.privacy),
          );
          await tester.pump();
          await tester.pump(const Duration(milliseconds: 300));

          expect(opener.opened, isEmpty);
          expect(find.text(Strings.linkOpenFailed), findsOneWidget);
        });
      }
    }

    testWidgets('an_uppercase_https_scheme_is_still_https_and_is_opened', (
      tester,
    ) async {
      // Uri.parse lowercases the scheme, so a legitimate HTTPS:// link is
      // accepted (and opened with the lowercase scheme).
      final opener = _Opener();
      await _openAccount(
        tester,
        config: const AppConfig(termsUrl: 'HTTPS://example.com/terms'),
        opener: opener,
      );

      await tester.tap(find.text(Strings.terms));
      await tester.pump();

      expect(opener.opened, hasLength(1));
      expect(opener.opened.single.scheme, 'https');
      expect(find.text(Strings.linkOpenFailed), findsNothing);
    });
  });

  group('sign out', () {
    testWidgets('sign_out_asks_nothing_twice_and_returns_to_sign_in', (
      tester,
    ) async {
      final store = MemorySessionStore('tok');
      final h = AppHarness(store: store);
      h.adapter
        ..reply(200, testMe)
        ..reply(200, <String, dynamic>{});
      await h.pump(tester);
      await tester.tap(find.text(Strings.tabAccount));
      await settle(tester);

      // Hand-off holders that a person could have left behind.
      final container = h.container(tester);
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
      container
          .read(pendingResetProvider.notifier)
          .set(
            const PendingReset(
              challenge: CodeChallenge(
                verificationId: 'v',
                resendAfterSecs: 30,
              ),
              phone: '0803 123 4567',
            ),
          );

      // One tap, no dialog. A second tap in the same moment does nothing more.
      await tester.tap(find.text(Strings.signOut));
      await tester.tap(find.text(Strings.signOut), warnIfMissed: false);
      expect(find.byType(AlertDialog), findsNothing);
      await settle(tester);

      expect(
        h.adapter.requests.where((r) => r.path == '/auth/sign-out'),
        hasLength(1),
      );
      expect(container.read(sessionProvider).value, isA<SessionSignedOut>());
      expect(store.token, isNull);
      expect(container.read(pendingVerificationProvider), isNull);
      expect(container.read(pendingResetProvider), isNull);
      expect(find.text(Strings.signInTitle), findsOneWidget);
      expect(
        container
            .read(routerProvider)
            .routerDelegate
            .currentConfiguration
            .last
            .matchedLocation,
        '/sign-in',
      );
    });
  });

  group('the four states', () {
    testWidgets('the_account_page_has_its_four_states', (tester) async {
      var calls = 0;
      final loading = Completer<SessionState>();
      final semantics = tester.ensureSemantics();
      await _pumpPage(tester, () {
        calls++;
        if (calls == 1) return loading.future;
        return Future.value(_signedIn());
      });
      await tester.pump();

      // Loading: a skeleton, announced once, and nothing else.
      expect(find.byType(Skeleton), findsWidgets);
      expect(find.bySemanticsLabel(Strings.loading), findsOneWidget);
      expect(find.byType(ErrorState), findsNothing);
      expect(find.byType(EmptyState), findsNothing);

      // Error: plain text with a retry AND a way out.
      loading.completeError(ApiError.offline());
      await tester.pump();
      await tester.pump();
      expect(find.byType(ErrorState), findsOneWidget);
      expect(find.text(Strings.offlineMessage), findsOneWidget);
      expect(find.text(Strings.retry), findsOneWidget);
      expect(find.text(Strings.signOut), findsOneWidget);
      expect(find.byType(Skeleton), findsNothing);
      expect(find.byType(EmptyState), findsNothing);

      // Retry asks for the session again; this time it answers.
      await tester.tap(find.text(Strings.retry));
      for (var i = 0; i < 4; i++) {
        await tester.pump();
      }

      // Data.
      expect(find.text('Ada'), findsOneWidget);
      expect(find.byType(ErrorState), findsNothing);
      expect(find.byType(Skeleton), findsNothing);
      // Empty is not applicable: a signed-in person always has a name.
      expect(find.byType(EmptyState), findsNothing);
      semantics.dispose();
    });

    testWidgets('sign_out_in_the_error_state_has_a_48dp_target', (
      tester,
    ) async {
      await _pumpPage(tester, () => Future.error(ApiError.offline()));
      await tester.pump();
      await tester.pump();

      final button = find.ancestor(
        of: find.text(Strings.signOut),
        matching: find.byType(FilledButton),
      );
      expect(tester.getSize(button.first).height, greaterThanOrEqualTo(48));
    });
  });

  testWidgets('text_at_200_percent_does_not_overflow_on_the_account_page', (
    tester,
  ) async {
    tester.platformDispatcher.textScaleFactorTestValue = 2.0;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    final h = AppHarness(store: MemorySessionStore('tok'), config: _fullConfig);
    h.adapter.reply(200, testMe);
    await h.pump(tester);
    tester.view.physicalSize = const Size(320, 640);
    await tester.tap(find.text(Strings.tabAccount));
    await settle(tester);

    expect(tester.takeException(), isNull);
    expect(find.text('Ada'), findsOneWidget);
  });

  testWidgets('rows_are_at_least_48dp_tall', (tester) async {
    await _openAccount(tester, config: _fullConfig);
    for (final label in [
      Strings.terms,
      Strings.privacy,
      Strings.contactSupport,
      Strings.addresses,
    ]) {
      final row = find.ancestor(
        of: find.text(label),
        matching: find.byType(InkWell),
      );
      expect(
        tester.getSize(row.first).height,
        greaterThanOrEqualTo(48),
        reason: label,
      );
    }
  });
}
