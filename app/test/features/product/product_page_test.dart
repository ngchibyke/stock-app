import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/app.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/format/money.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/core/ui/states.dart';
import 'package:market/features/catalog/catalog_models.dart';
import 'package:market/features/product/product_page.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/app_harness.dart';
import '../../support/fake_catalog_repository.dart';
import '../../support/fakes.dart';

Variant _v(
  String id,
  Map<String, String> attrs, {
  int price = 1500000,
  int stock = 10,
}) =>
    Variant(id: id, attributes: attrs, availableStock: stock, priceKobo: price);

ProductDetail _product({
  List<Variant>? variants,
  List<String> photos = const [
    'https://cdn.test/a.jpg',
    'https://cdn.test/b.jpg',
  ],
}) => ProductDetail(
  id: 'p1',
  storeId: 'id-ada',
  name: 'Ankara dress',
  description: 'Hand-sewn in Wuse.',
  condition: 'new',
  brand: 'Ada Wears',
  photoUrls: photos,
  variants:
      variants ??
      [
        _v('v-m-blue', {'Size': 'M', 'Colour': 'Blue'}, price: 1500000),
        _v('v-l-blue', {'Size': 'L', 'Colour': 'Blue'}, price: 1700000),
        _v('v-m-red', {'Size': 'M', 'Colour': 'Red'}, price: 1600000, stock: 3),
        _v('v-l-red', {'Size': 'L', 'Colour': 'Red'}, price: 1650000, stock: 0),
      ],
);

class _Harness {
  _Harness() : h = AppHarness(store: MemorySessionStore('tok')) {
    catalog = h.catalog;
    h.adapter.reply(200, testMe);
  }

  final AppHarness h;
  late final FakeCatalogRepository catalog;

  Future<void> pump(WidgetTester tester, {double textScale = 1}) async {
    tester.view.physicalSize = const Size(400, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      ProviderScope(
        retry: noRetry,
        overrides: h.pageOverrides,
        child: MaterialApp(
          theme: lightTheme(),
          home: MediaQuery(
            data: MediaQueryData(
              size: const Size(400, 900),
              textScaler: TextScaler.linear(textScale),
            ),
            child: ProductPage(productId: 'p1', store: fakeStore('ada')),
          ),
        ),
      ),
    );
    await settle(tester);
  }
}

Finder get _add => find.widgetWithText(FilledButton, Strings.addToCart);

bool _enabled(WidgetTester tester) =>
    tester.widget<FilledButton>(_add).onPressed != null;

Future<void> _choose(WidgetTester tester, String value) async {
  await tester.ensureVisible(find.widgetWithText(ChoiceChip, value));
  await tester.tap(find.widgetWithText(ChoiceChip, value));
  await tester.pump();
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  testWidgets('shows_photos_name_and_the_cheapest_price_before_choosing', (
    tester,
  ) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    await t.pump(tester);
    expect(find.text('Ankara dress'), findsOneWidget);
    expect(find.text('Ada Wears'), findsOneWidget);
    expect(find.text('Hand-sewn in Wuse.'), findsOneWidget);
    expect(find.text('Store ada'), findsOneWidget);
    expect(find.text(Strings.fromPrice(formatNaira(1500000))), findsOneWidget);
    expect(find.byKey(const Key('product-photo-0')), findsOneWidget);
    expect(find.text(Strings.photoCount(1, 2)), findsOneWidget);
  });

  testWidgets('no_photos_shows_the_neutral_tile', (tester) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product(photos: const []);
    await t.pump(tester);
    expect(find.byKey(const Key('product-photo-0')), findsNothing);
    expect(find.byIcon(Icons.shopping_bag_outlined), findsWidgets);
  });

  testWidgets('choosing_size_and_colour_picks_the_variant_and_its_price', (
    tester,
  ) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    await t.pump(tester);
    // Choosers sorted by attribute: Colour comes before Size.
    expect(
      tester.getTopLeft(find.text('Colour')).dy,
      lessThan(tester.getTopLeft(find.text('Size')).dy),
    );
    await _choose(tester, 'L');
    await _choose(tester, 'Blue');
    expect(find.text(formatNaira(1700000)), findsOneWidget);
    expect(find.text(Strings.fromPrice(formatNaira(1500000))), findsNothing);
  });

  testWidgets('only_n_left_from_1_to_5_and_sold_out_at_0', (tester) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    await t.pump(tester);
    await _choose(tester, 'M');
    await _choose(tester, 'Red');
    expect(find.text(Strings.onlyLeft(3)), findsOneWidget);
    expect(_enabled(tester), isTrue);

    await _choose(tester, 'L');
    expect(find.text(Strings.soldOut), findsOneWidget);
    expect(_enabled(tester), isFalse);

    await _choose(tester, 'Blue');
    expect(find.text(Strings.soldOut), findsNothing);
    expect(find.textContaining('left'), findsNothing);
  });

  testWidgets('add_is_disabled_until_a_variant_is_chosen', (tester) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    await t.pump(tester);
    expect(_enabled(tester), isFalse);
    await _choose(tester, 'M');
    expect(_enabled(tester), isFalse);
    await _choose(tester, 'Blue');
    expect(_enabled(tester), isTrue);
  });

  testWidgets('single_variant_without_attributes_is_preselected', (
    tester,
  ) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product(
      variants: [_v('only', const {}, price: 900000)],
    );
    await t.pump(tester);
    expect(find.byType(ChoiceChip), findsNothing);
    expect(find.text(formatNaira(900000)), findsOneWidget);
    expect(_enabled(tester), isTrue);
  });

  testWidgets('add_calls_cart_with_the_variant_id_only', (tester) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    await t.pump(tester);
    await _choose(tester, 'M');
    await _choose(tester, 'Blue');
    await tester.tap(_add);
    await settle(tester, frames: 30);

    // Only the variant and how many: the price is never sent (the request
    // body itself is pinned by set_sends_variant_and_quantity_only).
    expect(t.h.cart.sets, [('v-m-blue', 1)]);
    expect(find.text(Strings.addedToCart), findsOneWidget);
  });

  testWidgets('unavailable_after_open_says_so', (tester) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    t.h.cart.failWith = ApiError(
      status: 422,
      code: 'product_unavailable',
      message: 'Product unavailable.',
    );
    await t.pump(tester);
    await _choose(tester, 'M');
    await _choose(tester, 'Blue');
    await tester.tap(_add);
    await settle(tester, frames: 30);
    expect(find.text(Strings.noLongerBuyable), findsOneWidget);
    expect(find.text(Strings.addedToCart), findsNothing);
  });

  testWidgets('a_stock_limit_shows_the_server_message', (tester) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    t.h.cart.failWith = ApiError(
      status: 422,
      code: 'insufficient_stock',
      message: 'Only 3 left in stock.',
    );
    await t.pump(tester);
    await _choose(tester, 'M');
    await _choose(tester, 'Red');
    await tester.tap(_add);
    await settle(tester, frames: 30);
    expect(find.text('Only 3 left in stock.'), findsOneWidget);
  });

  testWidgets('loading_error_with_retry', (tester) async {
    final t = _Harness();
    t.catalog.offline = true;
    await t.pump(tester);
    expect(find.byType(ErrorState), findsOneWidget);
    t.catalog
      ..offline = false
      ..details['p1'] = _product();
    await tester.tap(find.text(Strings.retry));
    await settle(tester);
    expect(find.text('Ankara dress'), findsOneWidget);
  });

  testWidgets('page_at_200_percent_does_not_overflow', (tester) async {
    final t = _Harness();
    t.catalog.details['p1'] = _product();
    await t.pump(tester, textScale: 2);
    expect(tester.takeException(), isNull);
    await tester.drag(find.byType(Scrollable).first, const Offset(0, -600));
    await settle(tester);
    expect(tester.takeException(), isNull);
  });
}
