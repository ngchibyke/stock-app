import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/strings.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/app_harness.dart';
import '../../support/fakes.dart';

/// `/addresses` is built in a later task; here it is a stub so the navigation
/// can be asserted.
final _stub = GoRoute(
  path: '/addresses',
  builder: (context, state) => const Scaffold(body: Text('addresses stub')),
);

Future<AppHarness> _openApp(
  WidgetTester tester, {
  Size? size,
  AppConfig config = AppConfig.empty,
}) async {
  final h = AppHarness(store: MemorySessionStore('tok'), config: config);
  h.adapter.reply(200, testMe);
  await h.pump(
    tester,
    extra: [
      routerProvider.overrideWith((ref) {
        final router = buildRouter(ref.container, extraRoutes: [_stub]);
        ref.onDispose(router.dispose);
        return router;
      }),
    ],
  );
  if (size != null) tester.view.physicalSize = size;
  await settle(tester);
  return h;
}

void main() {
  testWidgets('the_shell_shows_market_first_with_a_placeholder_and_a_link', (
    tester,
  ) async {
    await _openApp(tester);

    expect(find.text(Strings.marketOpeningSoon), findsOneWidget);
    expect(find.byType(NavigationBar), findsOneWidget);

    await tester.tap(find.text(Strings.addresses));
    await settle(tester);
    expect(find.text('addresses stub'), findsOneWidget);
  });

  testWidgets('the_account_row_opens_the_address_book', (tester) async {
    await _openApp(tester);
    await tester.tap(find.text(Strings.tabAccount));
    await settle(tester);

    await tester.tap(find.text(Strings.addresses));
    await settle(tester);
    expect(find.text('addresses stub'), findsOneWidget);
  });

  testWidgets('the_shell_keeps_each_tabs_state_when_switching', (tester) async {
    await _openApp(
      tester,
      size: const Size(400, 300),
      config: const AppConfig(
        termsUrl: 'https://example.com/terms',
        privacyUrl: 'https://example.com/privacy',
        supportEmail: 'help@example.com',
      ),
    );
    await tester.tap(find.text(Strings.tabAccount));
    await settle(tester);

    final scrollable = find.byType(Scrollable).first;
    await tester.drag(scrollable, const Offset(0, -150));
    await settle(tester);
    final offset = tester.state<ScrollableState>(scrollable).position.pixels;
    expect(offset, greaterThan(0));

    await tester.tap(find.text(Strings.tabMarket));
    await settle(tester);
    expect(find.text(Strings.marketOpeningSoon), findsOneWidget);

    await tester.tap(find.text(Strings.tabAccount));
    await settle(tester);
    expect(
      tester
          .state<ScrollableState>(find.byType(Scrollable).first)
          .position
          .pixels,
      offset,
    );
  });

  testWidgets('tabs_have_labels_and_48dp_targets', (tester) async {
    final semantics = tester.ensureSemantics();
    await _openApp(tester);

    for (final label in [Strings.tabMarket, Strings.tabAccount]) {
      expect(find.text(label), findsOneWidget);
      final target = find.ancestor(
        of: find.text(label),
        matching: find.byType(NavigationDestination),
      );
      final size = tester.getSize(target.first);
      expect(size.height, greaterThanOrEqualTo(48), reason: label);
      expect(size.width, greaterThanOrEqualTo(48), reason: label);
      expect(find.bySemanticsLabel(RegExp(label)), findsWidgets);
    }
    semantics.dispose();
  });

  testWidgets('text_at_200_percent_does_not_overflow', (tester) async {
    tester.platformDispatcher.textScaleFactorTestValue = 2.0;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    await _openApp(tester, size: const Size(320, 640));

    expect(tester.takeException(), isNull);
    expect(find.text(Strings.marketOpeningSoon), findsOneWidget);
    await tester.tap(find.text(Strings.tabAccount));
    await settle(tester);
    expect(tester.takeException(), isNull);
  });
}
