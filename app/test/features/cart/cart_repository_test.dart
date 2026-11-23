import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/features/cart/cart.dart';
import 'package:market/features/cart/cart_repository.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/app_harness.dart';
import '../../support/fake_cart_repository.dart';
import '../../support/fakes.dart';

Map<String, dynamic> _line({
  String variant = 'v1',
  int quantity = 2,
  bool available = true,
}) => {
  'variant_id': variant,
  'product_id': 'p1',
  'product_name': 'Ankara dress',
  'attributes': {'Size': 'M', 'Colour': 'Blue'},
  'store_id': 's1',
  'store_name': 'Ada Fashion',
  'unit_price_kobo': 1500000,
  'quantity': quantity,
  'line_total_kobo': 1500000 * quantity,
  'available': available,
};

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('get_and_set_parse_the_cart', () async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.adapter
      ..reply(200, {
        'items': [_line()],
        'total_kobo': 3000000,
      })
      ..reply(200, {
        'items': [_line(quantity: 3)],
        'total_kobo': 4500000,
      });
    final container = ProviderContainer(overrides: h.pageOverrides);
    addTearDown(container.dispose);
    final repo = ApiCartRepository(container.read(apiClientProvider));

    final cart = await repo.get();
    expect(cart.items.single.productName, 'Ankara dress');
    expect(cart.items.single.attributes, {'Size': 'M', 'Colour': 'Blue'});
    expect(cart.items.single.lineTotalKobo, 3000000);
    expect(cart.totalKobo, 3000000);

    final after = await repo.set('v1', 3);
    expect(after.items.single.quantity, 3);
    expect(after.totalKobo, 4500000);
  });

  test('a_line_missing_its_variant_is_dropped_and_a_bad_body_throws', () {
    final cart = Cart.fromJson({
      'items': [
        _line(),
        {'product_name': 'no id'},
        'junk',
      ],
      'total_kobo': 3000000,
    });
    expect(cart.items.length, 1);
    expect(() => Cart.fromJson('nope'), throwsA(isA<FormatException>()));
  });

  test('set_sends_variant_and_quantity_only', () async {
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.adapter.reply(200, {'items': <Object?>[], 'total_kobo': 0});
    final container = ProviderContainer(overrides: h.pageOverrides);
    addTearDown(container.dispose);

    await ApiCartRepository(container.read(apiClientProvider)).set('v 1', 0);

    final r = h.adapter.requests.single;
    expect(r.method, 'PUT');
    expect(r.path, '/cart/items');
    expect(r.body, {'variant_id': 'v 1', 'quantity': 0});
  });

  test('count_ignores_unavailable_lines', () {
    final cart = Cart(
      items: [
        fakeLine('a', quantity: 2),
        fakeLine('b', quantity: 5, available: false),
        fakeLine('c', storeId: 's2', store: 'Bola Phones'),
      ],
      totalKobo: 0,
    );
    expect(cart.count, 3);
    expect(cart.byStore.keys, ['Ada Fashion', 'Bola Phones']);
    expect(cart.byStore['Ada Fashion']!.length, 2);
  });

  group('the cart notifier', () {
    Future<(ProviderContainer, AppHarness, FakeCartRepository)> start() async {
      final repo = FakeCartRepository([fakeLine('v1', quantity: 2)]);
      final h = AppHarness(store: MemorySessionStore('tok'), cart: repo);
      h.adapter.reply(200, testMe);
      final container = ProviderContainer(overrides: h.pageOverrides);
      addTearDown(container.dispose);
      await container.read(sessionProvider.future);
      final sub = container.listen(cartProvider, (_, _) {});
      addTearDown(sub.close);
      await container.read(cartProvider.future);
      return (container, h, repo);
    }

    test('add_asks_for_one_more_than_held', () async {
      final (container, _, repo) = await start();
      await container.read(cartProvider.notifier).add('v1');
      await container.read(cartProvider.notifier).add('v9');
      expect(repo.sets, [('v1', 3), ('v9', 1)]);
      expect(container.read(cartProvider).value!.count, 4);
    });

    test('a_failed_change_keeps_the_cart_and_throws', () async {
      final (container, _, repo) = await start();
      repo.failWith = ApiError(
        status: 422,
        code: 'validation_failed',
        message: 'x',
      );
      await expectLater(
        container.read(cartProvider.notifier).setQuantity('v1', 5),
        throwsA(isA<ApiError>()),
      );
      expect(container.read(cartProvider).value!.count, 2);
    });

    test('cart_is_dropped_on_sign_out', () async {
      final (container, h, repo) = await start();
      expect(container.read(cartProvider).value!.count, 2);

      h.adapter.reply(200, <String, dynamic>{});
      await container.read(sessionProvider.notifier).signOut();
      final after = await container.read(cartProvider.future);

      expect(after.items, isEmpty);
      expect(after.count, 0);
      // Signed out: the cart was not asked for again.
      expect(repo.gets, 1);
    });

    test('nothing_about_the_cart_is_written_to_preferences', () async {
      final (container, _, _) = await start();
      await container.read(cartProvider.notifier).add('v1');
      await container.read(cartProvider.notifier).setQuantity('v1', 0);
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getKeys().where((k) => k.contains('cart')), isEmpty);
    });
  });
}
