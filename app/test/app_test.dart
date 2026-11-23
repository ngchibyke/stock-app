import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/strings.dart';
import 'package:market/features/auth/auth_errors.dart';
import 'package:market/features/market/market_page.dart';
import 'package:material_ui/material_ui.dart';

import 'support/app_harness.dart';
import 'support/fakes.dart';

const _update = AppConfig(minVersionAndroid: '2.0.0');

void main() {
  group('the gate notice', () {
    testWidgets('an_update_required_gate_shows_a_full_screen_notice', (
      tester,
    ) async {
      final h = AppHarness(config: _update);
      await h.pump(tester);

      expect(find.text(Strings.updateRequiredTitle), findsOneWidget);
      expect(find.text(Strings.updateRequiredBody), findsOneWidget);
      // Nothing of the app is reachable behind it.
      expect(find.text(Strings.signInTitle), findsNothing);
      expect(find.byType(TextField), findsNothing);
    });

    testWidgets('maintenance_shows_the_servers_message', (tester) async {
      final h = AppHarness(
        config: const AppConfig(
          maintenanceActive: true,
          maintenanceMessage: 'Back at 6pm.',
        ),
      );
      await h.pump(tester);

      expect(find.text(Strings.maintenanceTitle), findsOneWidget);
      expect(find.text('Back at 6pm.'), findsOneWidget);
      expect(find.text(Strings.signInTitle), findsNothing);
    });

    testWidgets('maintenance_without_a_message_shows_plain_fallback_copy', (
      tester,
    ) async {
      for (final message in [null, '', '   ']) {
        final h = AppHarness(
          config: AppConfig(
            maintenanceActive: true,
            maintenanceMessage: message,
          ),
        );
        await h.pump(tester);
        expect(
          find.text(Strings.maintenanceFallback),
          findsOneWidget,
          reason: 'message: "$message"',
        );
        await tester.pumpWidget(const SizedBox());
      }
    });

    testWidgets('the_notice_goes_away_when_the_gate_opens', (tester) async {
      final h = AppHarness(config: _update);
      await h.pump(tester);
      expect(find.text(Strings.updateRequiredTitle), findsOneWidget);

      h.config.set(AppConfig.empty);
      await settle(tester);

      expect(find.text(Strings.updateRequiredTitle), findsNothing);
      expect(find.text(Strings.signInTitle), findsOneWidget);
    });

    testWidgets('the_notice_comes_back_if_maintenance_starts_later', (
      tester,
    ) async {
      final h = AppHarness();
      await h.pump(tester);
      expect(find.text(Strings.signInTitle), findsOneWidget);

      h.config.set(const AppConfig(maintenanceActive: true));
      await settle(tester);
      expect(find.text(Strings.maintenanceTitle), findsOneWidget);
      expect(find.text(Strings.signInTitle), findsNothing);

      h.config.set(AppConfig.empty);
      await settle(tester);
      expect(find.text(Strings.signInTitle), findsOneWidget);
    });
  });

  group('a failed session restore', () {
    testWidgets('shows_a_retry_and_a_sign_out_and_retry_recovers', (
      tester,
    ) async {
      final h = AppHarness(store: MemorySessionStore('tok'));
      h.adapter
        ..failWithConnectionError()
        ..reply(200, testMe);
      await h.pump(tester);

      expect(find.text(Strings.offlineMessage), findsOneWidget);
      expect(find.text(Strings.retry), findsOneWidget);
      expect(find.text(Strings.signOut), findsOneWidget);
      expect(find.text(Strings.signInTitle), findsNothing);

      await tester.tap(find.text(Strings.retry));
      await settle(tester);

      expect(find.byType(MarketPage), findsOneWidget);
      expect(h.container(tester).read(sessionProvider).value, isNotNull);
    });

    testWidgets('sign_out_is_a_way_out_of_a_persistent_failure', (
      tester,
    ) async {
      final store = MemorySessionStore('tok');
      final h = AppHarness(store: store);
      h.adapter
        ..reply(200, <String, dynamic>{'unexpected': true})
        ..reply(200, <String, dynamic>{});
      await h.pump(tester);

      expect(find.text(Strings.genericError), findsOneWidget);
      expect(find.text(Strings.signOut), findsOneWidget);

      await tester.tap(find.text(Strings.signOut));
      await settle(tester);

      expect(find.text(Strings.signInTitle), findsOneWidget);
      expect(store.token, isNull);
    });

    testWidgets('the_error_screen_never_shows_the_servers_own_words', (
      tester,
    ) async {
      final h = AppHarness(store: MemorySessionStore('tok'));
      h.adapter.reply(500, {
        'error': {'code': 'boom', 'message': 'raw server text'},
      });
      await h.pump(tester);

      expect(find.text(authErrorMessage(_serverDown)), findsOneWidget);
      expect(find.textContaining('raw server text'), findsNothing);
    });
  });

  group('the root wiring', () {
    testWidgets('the_api_client_has_its_session_hooks', (tester) async {
      final h = AppHarness();
      await h.pump(tester);

      final container = h.container(tester);
      // Without sessionOverrides these throw on purpose.
      expect(() => container.read(accessTokenReaderProvider), returnsNormally);
      expect(
        () => container.read(unauthorizedHandlerProvider),
        returnsNormally,
      );
    });

    testWidgets('a_failed_provider_is_not_retried_in_the_background', (
      tester,
    ) async {
      var builds = 0;
      final flaky = FutureProvider<int>((ref) async {
        builds++;
        throw ApiError.offline();
      });
      final h = AppHarness();
      await h.pump(tester);

      final container = h.container(tester);
      final sub = container.listen(flaky, (_, _) {});
      addTearDown(sub.close);
      await tester.pump(const Duration(seconds: 30));

      expect(builds, 1);
      expect(container.read(flaky).hasError, isTrue);
    });

    testWidgets(
      'the_config_is_refreshed_when_the_app_returns_to_the_foreground',
      (tester) async {
        final h = AppHarness();
        await h.pump(tester);
        expect(h.config.refreshCount, 0);

        for (final s in [
          AppLifecycleState.inactive,
          AppLifecycleState.hidden,
          AppLifecycleState.paused,
          AppLifecycleState.hidden,
          AppLifecycleState.inactive,
          AppLifecycleState.resumed,
        ]) {
          tester.binding.handleAppLifecycleStateChanged(s);
        }
        await tester.pump();

        expect(h.config.refreshCount, 1);
      },
    );

    testWidgets('it_is_one_material_app_using_a_router', (tester) async {
      final h = AppHarness();
      await h.pump(tester);

      expect(find.byType(MaterialApp), findsOneWidget);
      final app = tester.widget<MaterialApp>(find.byType(MaterialApp));
      expect(app.routerConfig, isNotNull);
      expect(app.theme, isNotNull);
      expect(app.darkTheme, isNotNull);
    });
  });
}

final _serverDown = ApiError(status: 500, code: 'boom', message: 'raw');
