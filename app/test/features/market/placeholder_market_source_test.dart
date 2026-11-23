import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/market/data/json_cache.dart';
import 'package:market/features/market/data/placeholder_market_source.dart';

import '../../support/fake_catalog_repository.dart';

// The real bundled layout and rooms, read from disk (tests run from app/).
Future<String> _disk(String path) async => File(path).readAsString();

void main() {
  late FakeCatalogRepository catalog;
  late MemoryJsonCache cache;
  late PlaceholderMarketSource source;

  setUp(() {
    catalog = FakeCatalogRepository();
    cache = MemoryJsonCache();
    source = PlaceholderMarketSource(
      catalog: catalog,
      cache: cache,
      loadAsset: _disk,
    );
  });

  test('fills_plots_in_order_by_slug', () async {
    catalog.storeList = [
      fakeStore('zara'),
      fakeStore('ada'),
      fakeStore('musa'),
    ];
    final snap = await source.market();
    final plots = snap.layout.plots;
    expect(snap.placementFor(plots[0].id)!.store.slug, 'ada');
    expect(snap.placementFor(plots[1].id)!.store.slug, 'musa');
    expect(snap.placementFor(plots[2].id)!.store.slug, 'zara');
    expect(snap.placementFor(plots[3].id), isNull);
    expect(snap.fromCache, isFalse);
  });

  test('more_stores_than_plots_keeps_the_first_60', () async {
    catalog.storeList = [
      for (var i = 0; i < 75; i++)
        fakeStore('s${i.toString().padLeft(3, '0')}'),
    ];
    final snap = await source.market();
    expect(snap.placements, hasLength(60));
    expect(snap.placements.last.store.slug, 's059');
  });

  test('no_stores_gives_an_all_coming_soon_market', () async {
    final snap = await source.market();
    expect(snap.layout.plots, hasLength(60));
    expect(snap.placements, isEmpty);
  });

  test('type_follows_district', () async {
    catalog.storeList = [
      for (var i = 0; i < 60; i++)
        fakeStore('s${i.toString().padLeft(2, '0')}'),
    ];
    final snap = await source.market();
    String typeIn(String district) => snap.placements
        .firstWhere((p) => snap.plotOf(p)!.districtId == district)
        .storeType;
    expect(typeIn('fashion'), 'boutique');
    expect(typeIn('phones-electronics'), 'phone_shop');
    expect(typeIn('food-groceries'), 'grocery');
    expect(typeIn('home-beauty'), 'boutique');
  });

  test('offline_answers_the_cache_with_fromCache_true', () async {
    catalog.storeList = [fakeStore('ada')];
    await source.market();
    catalog.offline = true;
    final fresh = PlaceholderMarketSource(
      catalog: catalog,
      cache: cache,
      loadAsset: _disk,
    );
    final snap = await fresh.market();
    expect(snap.fromCache, isTrue);
    expect(snap.placements.single.store.slug, 'ada');
  });

  test('offline_with_no_cache_rethrows', () async {
    catalog.offline = true;
    await expectLater(
      source.market(),
      throwsA(isA<ApiError>().having((e) => e.isOffline, 'offline', isTrue)),
    );
  });

  test('room_places_only_in_stock_products_in_slot_order', () async {
    catalog.storeList = [fakeStore('ada')];
    catalog.productList = [
      fakeCard('a'),
      fakeCard('b', inStock: false),
      fakeCard('c'),
      fakeCard('other', store: 'musa'),
    ];
    await source.market();
    final room = await source.room('ada');
    expect(room.type.id, 'boutique');
    expect(room.placed.map((p) => p.product.id), ['a', 'c']);
    expect(room.placed.map((p) => p.slotId), [
      room.type.slots[0].id,
      room.type.slots[1].id,
    ]);
    expect(room.hasMore, isFalse);
  });

  test('room_sets_hasMore_when_products_exceed_slots', () async {
    catalog.storeList = [fakeStore('ada')];
    catalog.productList = [for (var i = 0; i < 12; i++) fakeCard('p$i')];
    final room = await source.room('ada');
    expect(room.placed, hasLength(room.type.slots.length));
    expect(room.hasMore, isTrue);
  });

  test('room_for_a_missing_store_throws_store_not_found', () async {
    await expectLater(
      source.room('gone'),
      throwsA(isA<ApiError>().having((e) => e.code, 'code', 'store_not_found')),
    );
  });

  test('room_offline_answers_its_cache', () async {
    catalog.storeList = [fakeStore('ada')];
    catalog.productList = [fakeCard('a')];
    await source.room('ada');
    catalog.offline = true;
    final room = await source.room('ada');
    expect(room.placed.single.product.id, 'a');
  });

  test('nothing_personal_is_cached', () async {
    catalog.storeList = [fakeStore('ada')];
    catalog.productList = [fakeCard('a')];
    await source.market();
    await source.room('ada');
    expect(cache.keys, {'market.v1', 'room.v1.ada'});
  });

  test('store_type_loads_from_the_bundle_without_the_network', () async {
    catalog.offline = true;
    final type = await source.storeType('grocery');
    expect(type.id, 'grocery');
    expect(type.slots.length, 9);
    expect(catalog.calls, isEmpty);
  });
}
