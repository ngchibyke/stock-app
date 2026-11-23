import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/paths.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/ui/states.dart';
import 'package:market/features/list/list_page.dart';
import 'package:market/features/market/data/placeholder_market_source.dart';
import 'package:market/features/market/journey/journey_view.dart';
import 'package:market/features/market/market_page.dart';
import 'package:market/features/market/room/room_page.dart';
import 'package:market/features/market/view/market_view.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/app_harness.dart';
import '../../support/fake_catalog_repository.dart';
import '../../support/fakes.dart';

/// Signed in, on the Market tab, with Ada's store open in the market.
Future<AppHarness> _open(WidgetTester tester, {AppHarness? harness}) async {
  final h = harness ?? AppHarness(store: MemorySessionStore('tok'));
  h.catalog.storeList = [fakeStore('ada')];
  h.adapter.reply(200, testMe);
  await h.pump(tester);
  await settle(tester);
  return h;
}

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
    // The bundle keeps each loaded file as a future made inside one test's
    // fake clock; a later test would wait on it for ever.
    rootBundle.clear();
  });

  testWidgets('market_tab_shows_the_market', (tester) async {
    await _open(tester);
    expect(find.byType(MarketPage), findsOneWidget);
    expect(find.byType(MarketView), findsOneWidget);
    expect(find.text(Strings.searchMarket), findsOneWidget);
    expect(find.bySemanticsLabel(Strings.listView), findsOneWidget);
    expect(find.bySemanticsLabel(Strings.cartLabel(0)), findsOneWidget);
    expect(find.text(Strings.offlineMarket), findsNothing);
  });

  testWidgets('offline_snapshot_shows_the_offline_line', (tester) async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    // A market seen once while online, then the phone goes offline.
    final online = FakeCatalogRepository()..storeList = [fakeStore('ada')];
    await tester.runAsync(
      () => PlaceholderMarketSource(
        catalog: online,
        cache: h.cache,
        loadAsset: (p) => File(p).readAsString(),
      ).market(),
    );
    h.catalog.offline = true;
    await _open(tester, harness: h);
    expect(find.byType(MarketView), findsOneWidget);
    expect(find.text(Strings.offlineMarket), findsOneWidget);
  });

  testWidgets('market_load_error_has_retry', (tester) async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.catalog.offline = true;
    await _open(tester, harness: h);
    expect(find.byType(ErrorState), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsNothing);

    h.catalog.offline = false;
    await tester.tap(find.text(Strings.retry));
    await settle(tester);
    expect(find.byType(MarketView), findsOneWidget);
  });

  testWidgets('opening_a_store_prefetches_its_room', (tester) async {
    final h = await _open(tester);
    h.catalog.calls.clear();
    await tester.tap(
      find.bySemanticsLabel(Strings.openStoreLabel('Store ada', 4.2, 5)),
    );
    await tester.pump();
    expect(h.catalog.calls, contains('store ada'));
    // The flight has started: the search bar steps aside.
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.text(Strings.searchMarket), findsNothing);
  });

  testWidgets('list_button_and_search_open_the_list_with_no_flight', (
    tester,
  ) async {
    await _open(tester);
    await tester.tap(find.bySemanticsLabel(Strings.listView));
    await settle(tester);
    expect(find.byType(ListPage), findsOneWidget);
    expect(find.byType(JourneyView), findsNothing);

    await tester.pageBack();
    await settle(tester);
    await tester.tap(find.text(Strings.searchMarket));
    await settle(tester);
    final field = tester.widget<TextField>(find.byType(TextField));
    expect(field.focusNode?.hasFocus ?? field.autofocus, isTrue);
  });

  testWidgets('deep_link_to_a_store_opens_the_room', (tester) async {
    final h = await _open(tester);
    h.container(tester).read(routerProvider).go(storeRoomPath('ada'));
    await settle(tester);
    expect(find.byType(RoomPage), findsOneWidget);
    expect(find.byType(JourneyView), findsNothing);
    expect(find.text('Store ada'), findsOneWidget);
  });
}
