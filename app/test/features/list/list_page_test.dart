import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/paths.dart';
import 'package:market/core/strings.dart';
import 'package:market/features/list/rows.dart';
import 'package:market/features/list/store_page.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/app_harness.dart';
import '../../support/fake_catalog_repository.dart';
import '../../support/fakes.dart';

/// Signed in, on the list view, over a catalog of [products] things.
Future<AppHarness> _open(WidgetTester tester, {int products = 5}) async {
  final h = AppHarness(store: MemorySessionStore('tok'));
  h.catalog
    ..storeList = [fakeStore('ada'), fakeStore('bola')]
    ..productList = [
      for (var i = 0; i < products; i++)
        fakeCard('p$i', name: i == 0 ? 'Ankara dress' : 'Thing $i'),
    ];
  h.adapter.reply(200, testMe);
  await h.pump(tester);
  h.container(tester).read(routerProvider).go(listPath);
  await settle(tester);
  return h;
}

List<String> _productCalls(AppHarness h) =>
    h.catalog.calls.where((c) => c.startsWith('products')).toList();

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  testWidgets('list_search_debounces_and_sends_q', (tester) async {
    final h = await _open(tester);
    h.catalog.calls.clear();
    final field = find.byType(TextField);
    for (final q in ['a', 'an', 'ank']) {
      await tester.enterText(field, q);
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(_productCalls(h), isEmpty);
    await tester.pump(const Duration(milliseconds: 300));
    await settle(tester);
    expect(_productCalls(h), [
      'products q=ank category=null store=null cursor=null',
    ]);
    expect(find.text('Ankara dress'), findsOneWidget);
    expect(find.text('Thing 1'), findsNothing);
    // The field holds no more than the server accepts.
    expect(tester.widget<TextField>(field).maxLength, 80);
  });

  testWidgets('category_chip_filters', (tester) async {
    final h = await _open(tester);
    h.catalog.calls.clear();
    await tester.tap(find.widgetWithText(ChoiceChip, 'Fashion'));
    await settle(tester);
    expect(_productCalls(h), [
      'products q=null category=fashion store=null cursor=null',
    ]);
  });

  testWidgets('paging_loads_the_next_cursor_once', (tester) async {
    final h = await _open(tester, products: 60);
    h.catalog.calls.clear();
    // Drag the product list itself (not the category strip above it).
    await tester.drag(find.text('Thing 1'), const Offset(0, -2500));
    await settle(tester);
    await tester.drag(find.byType(ProductRow).last, const Offset(0, -200));
    await settle(tester);
    final calls = _productCalls(h);
    expect(calls.where((c) => c.endsWith('cursor=24')).length, 1);
  });

  testWidgets('empty_search_offers_clear', (tester) async {
    await _open(tester);
    await tester.enterText(find.byType(TextField), 'zzz');
    await tester.pump(const Duration(milliseconds: 350));
    await settle(tester);
    expect(find.text(Strings.nothingMatches('zzz')), findsOneWidget);
    await tester.tap(find.text(Strings.clearSearch));
    await tester.pump(const Duration(milliseconds: 350));
    await settle(tester);
    expect(find.text('Ankara dress'), findsOneWidget);
  });

  testWidgets('store_row_opens_the_plain_store_page', (tester) async {
    await _open(tester);
    await tester.tap(find.text(Strings.storesTab));
    await settle(tester);
    await tester.tap(find.text('Store ada'));
    await settle(tester);
    expect(find.byType(StorePage), findsOneWidget);
    expect(find.text('About ada'), findsOneWidget);
    // Ada's products, no room and no flight.
    expect(find.text('Ankara dress'), findsOneWidget);
  });

  testWidgets('a_product_row_opens_the_product', (tester) async {
    final h = await _open(tester);
    await tester.tap(find.text('Ankara dress'));
    await settle(tester);
    expect(
      h.container(tester).read(routerProvider).state.uri.path,
      productPath('p0'),
    );
  });
}
