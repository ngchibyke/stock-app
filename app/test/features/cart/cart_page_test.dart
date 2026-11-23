import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/format/money.dart';
import 'package:market/core/paths.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/cart/cart.dart';
import 'package:market/features/cart/cart_button.dart';
import 'package:market/features/cart/cart_repository.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/app_harness.dart';
import '../../support/fake_cart_repository.dart';
import '../../support/fakes.dart';

/// Opens the app signed in on the cart page, with [repo] as the server.
Future<AppHarness> _open(WidgetTester tester, FakeCartRepository repo) async {
  final h = AppHarness(store: MemorySessionStore('tok'), cart: repo);
  h.adapter.reply(200, testMe);
  await h.pump(tester);
  h.container(tester).read(routerProvider).push(cartPath);
  await settle(tester);
  return h;
}

Finder _inLine(String variantId, Finder f) => find.descendant(
  of: find.byKey(ValueKey('cart-line-$variantId')),
  matching: f,
);

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  testWidgets('page_groups_by_store', (tester) async {
    await _open(
      tester,
      FakeCartRepository([
        fakeLine('a', product: 'Ankara dress'),
        fakeLine(
          'b',
          product: 'iPhone 12',
          storeId: 's2',
          store: 'Bola Phones',
        ),
        fakeLine('c', product: 'Head tie'),
      ]),
    );
    expect(find.text('Ada Fashion'), findsOneWidget);
    expect(find.text('Bola Phones'), findsOneWidget);
    // Ada's two lines sit under her heading, before Bola's.
    final ada = tester.getTopLeft(find.text('Ada Fashion')).dy;
    final bola = tester.getTopLeft(find.text('Bola Phones')).dy;
    expect(
      tester.getTopLeft(find.text('Head tie')).dy,
      inExclusiveRange(ada, bola),
    );
    expect(tester.getTopLeft(find.text('iPhone 12')).dy, greaterThan(bola));
    expect(find.text('Size: M'), findsNWidgets(3));
  });

  testWidgets('stepper_updates_via_the_server_answer', (tester) async {
    final repo = FakeCartRepository([
      fakeLine('a', unit: 1500000, quantity: 1),
    ]);
    await _open(tester, repo);
    await tester.tap(
      _inLine('a', find.bySemanticsLabel(Strings.increaseQuantity)),
    );
    await settle(tester);
    expect(repo.sets, [('a', 2)]);
    expect(_inLine('a', find.text('2')), findsOneWidget);
    expect(_inLine('a', find.text(formatNaira(3000000))), findsOneWidget);
  });

  testWidgets('minus_at_1_confirms_removal', (tester) async {
    final repo = FakeCartRepository([fakeLine('a', product: 'Ankara dress')]);
    await _open(tester, repo);
    await tester.tap(
      _inLine('a', find.bySemanticsLabel(Strings.decreaseQuantity)),
    );
    await settle(tester);
    expect(
      find.text(Strings.removeItemQuestion('Ankara dress')),
      findsOneWidget,
    );
    expect(repo.sets, isEmpty);

    await tester.tap(find.text(Strings.cancel));
    await settle(tester);
    expect(repo.sets, isEmpty);
    expect(find.text('Ankara dress'), findsOneWidget);

    await tester.tap(
      _inLine('a', find.bySemanticsLabel(Strings.decreaseQuantity)),
    );
    await settle(tester);
    await tester.tap(find.widgetWithText(FilledButton, Strings.remove));
    await settle(tester);
    expect(repo.sets, [('a', 0)]);
    expect(find.text(Strings.cartEmptyTitle), findsOneWidget);
  });

  testWidgets('unavailable_line_is_marked_and_removable', (tester) async {
    final repo = FakeCartRepository([
      fakeLine('a', product: 'Ankara dress'),
      fakeLine('b', product: 'Old bag', available: false),
    ]);
    await _open(tester, repo);
    expect(_inLine('b', find.text(Strings.noLongerAvailable)), findsOneWidget);
    expect(_inLine('a', find.text(Strings.noLongerAvailable)), findsNothing);
    // No stepper on a line that cannot be bought.
    expect(
      _inLine('b', find.bySemanticsLabel(Strings.increaseQuantity)),
      findsNothing,
    );

    await tester.tap(_inLine('b', find.text(Strings.remove)));
    await settle(tester);
    expect(repo.sets, [('b', 0)]);
    expect(find.text('Old bag'), findsNothing);
  });

  testWidgets('total_is_the_server_total', (tester) async {
    final repo = FakeCartRepository([fakeLine('a', unit: 1500000, quantity: 2)])
      // Not what the lines add up to: the page must show the server's number.
      ..totalFor = (_) => 2999900;
    await _open(tester, repo);
    expect(find.text(formatNaira(2999900)), findsOneWidget);
    expect(find.text(Strings.checkoutSoon), findsOneWidget);
  });

  testWidgets('empty_state_leads_to_the_market', (tester) async {
    await _open(tester, FakeCartRepository());
    expect(find.text(Strings.cartEmptyTitle), findsOneWidget);
    await tester.tap(find.text(Strings.goToMarket));
    await settle(tester);
    expect(find.text(Strings.cartEmptyTitle), findsNothing);
  });

  testWidgets('badge_label_reads_the_count', (tester) async {
    Future<void> show(Cart cart) async {
      await tester.pumpWidget(
        ProviderScope(
          key: UniqueKey(),
          overrides: [cartProvider.overrideWith(() => _Fixed(cart))],
          child: MaterialApp(
            theme: lightTheme(),
            home: const Scaffold(body: Center(child: CartButton())),
          ),
        ),
      );
      await tester.pump();
    }

    await show(const Cart(items: [], totalKobo: 0));
    expect(find.bySemanticsLabel(Strings.cartLabel(0)), findsOneWidget);
    expect(find.byKey(const Key('cart-badge')), findsNothing);

    await show(
      Cart(
        items: [
          fakeLine('a', quantity: 2),
          fakeLine('b', available: false),
          fakeLine('c'),
        ],
        totalKobo: 0,
      ),
    );
    expect(find.bySemanticsLabel(Strings.cartLabel(3)), findsOneWidget);
    expect(find.text('3'), findsOneWidget);
    expect(
      tester.getSize(find.byType(CartButton)).height,
      greaterThanOrEqualTo(48),
    );
    expect(Strings.cartLabel(1), 'Cart, 1 item');
  });
}

class _Fixed extends CartNotifier {
  _Fixed(this.cart);
  final Cart cart;

  @override
  Future<Cart> build() async => cart;
}
