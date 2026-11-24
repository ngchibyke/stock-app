// Market wire check: the path a buyer takes through the market, driven with
// the app's real catalog, market and cart code against a REAL local server.
// The screens on this path are covered by widget tests; this proves the wire.
//
// Skipped unless WIRE_BASE_URL and WIRE_OTP_LOG are set (see "Wire check" in
// README.md). Needs test/wire/fixtures/market_seed.sql applied to the server's
// scratch database first. Run from the app folder: it reads the bundled
// market layout from assets/.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/cart/cart_repository.dart';
import 'package:market/features/catalog/catalog_repository.dart';
import 'package:market/features/market/data/api_market_source.dart';
import 'package:market/features/market/data/json_cache.dart';

import 'wire_support.dart';

const _store = 'flow-test-boutique';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  // flutter test blocks real network access by default.
  setUpAll(() => HttpOverrides.global = null);

  test('a_buyer_finds_a_product_adds_it_and_walks_into_its_store', () async {
    if (!isLoopbackBaseUrl(wireBaseUrl)) {
      fail('the wire check only talks to a loopback server');
    }
    String? token;
    final api = ApiClient(
      baseUrl: wireBaseUrl,
      readToken: () async => token,
      onUnauthorized: () {},
    );
    final auth = ApiAuthRepository(api);
    final catalog = ApiCatalogRepository(api);
    final cart = ApiCartRepository(api);

    // A fresh buyer.
    late CodeChallenge challenge;
    final code = await codeAfter(() async {
      challenge = await auth.signUp(
        phone: nextPhone(),
        password: freshPassword(),
        displayName: 'Market Wire Buyer',
      );
    });
    token = await auth.verifyPhone(
      verificationId: challenge.verificationId,
      code: code,
    );

    // The list view: search for the dress.
    final found = await catalog.products(q: 'Flowtest Ankara');
    final card = found.items.singleWhere(
      (p) => p.name == 'Flowtest Ankara Dress',
      orElse: () => fail(
        'the seeded dress is not found by search; '
        'was market_seed.sql applied?',
      ),
    );
    expect(card.store.slug, _store);
    expect(card.priceFromKobo, 1500000);
    expect(card.inStock, isTrue);

    // Its page: two sizes, L at its own price.
    final dress = await catalog.product(card.id);
    expect(dress.variants.map((v) => v.attributes['Size']).toSet(), {'M', 'L'});
    final large = dress.variants.singleWhere(
      (v) => v.attributes['Size'] == 'L',
    );
    final medium = dress.variants.singleWhere(
      (v) => v.attributes['Size'] == 'M',
    );
    expect(large.priceKobo, 1700000);
    expect(medium.priceKobo, 1500000, reason: 'M uses the base price');
    expect(large.availableStock, 3);

    // Add to cart, the way the product page does: one more than held.
    var held = await cart.get();
    expect(held.items, isEmpty);
    held = await cart.set(large.id, 1);
    expect(held.items.single.variantId, large.id);
    expect(held.items.single.unitPriceKobo, 1700000);
    expect(held.items.single.storeName, 'Flow Test Boutique');
    expect(held.items.single.attributes, {'Size': 'L'});
    expect(held.totalKobo, 1700000);
    held = await cart.set(large.id, 2);
    expect(held.totalKobo, 3400000);
    expect(held.count, 2);
    expect((await cart.get()).totalKobo, 3400000);

    // The market: the store stands on a plot.
    final source = ApiMarketSource(
      api: api,
      cache: MemoryJsonCache(),
      loadAsset: (path) => File(path).readAsString(),
    );
    final market = await source.market();
    final placement = market.placements.singleWhere(
      (p) => p.store.slug == _store,
      orElse: () => fail('the seeded store has no plot in the market'),
    );
    expect(market.plotOf(placement), isNotNull);
    expect(placement.storeType, isNotEmpty);
    expect(market.fromCache, isFalse);

    // Its room: the three products on the shelves.
    final room = await source.room(_store);
    expect(room.placed, isNotEmpty);
    expect(room.store.slug, _store);
    expect(room.placed.map((p) => p.product.name).toSet(), {
      'Flowtest Ankara Dress',
      'Flowtest Head Tie',
      'Flowtest Leather Sandals',
    });
    expect(room.hasMore, isFalse);
    for (final p in room.placed) {
      expect(room.type.slot(p.slotId), isNotNull, reason: p.slotId);
    }

    // Leave the cart as it was found.
    expect((await cart.set(large.id, 0)).items, isEmpty);
  }, skip: wireSkip);
}
