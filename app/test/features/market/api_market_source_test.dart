import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/market/data/api_market_source.dart';
import 'package:market/features/market/data/json_cache.dart';

import '../../support/fakes.dart';

// The real bundled rooms, read from disk (tests run from app/).
Future<String> _disk(String path) async => File(path).readAsString();

Map<String, Object?> _store(String slug) => {
  'id': 'id-$slug',
  'slug': slug,
  'name': 'Store $slug',
  'logo_key': 'logos/$slug.png',
  'logo_url': 'https://cdn.test/logos/$slug.png',
  'rating_average': 4.5,
  'rating_count': 12,
  'product_count': 3,
};

Map<String, Object?> _card(String id, {String store = 'ada'}) => {
  'id': id,
  'name': 'Thing $id',
  'category': 'fashion',
  'condition': 'new',
  'price_from_kobo': 250000,
  'photo_url': null,
  'in_stock': true,
  'store': _store(store),
};

Map<String, Object?> _market(List<Object?> placements) => {
  'version': 'placeholder-1',
  'world': {'w': 2048, 'h': 2048},
  'tile_size': 256,
  'max_level': 3,
  'base_image': 'base.jpg',
  'districts': [
    {'id': 'fashion', 'name': 'Fashion', 'store_type': 'boutique'},
  ],
  'plots': [
    {
      'id': 'fashion-1',
      'district': 'fashion',
      'x': 88,
      'y': 88,
      'w': 182,
      'h': 177,
    },
    {
      'id': 'fashion-2',
      'district': 'fashion',
      'x': 318,
      'y': 88,
      'w': 182,
      'h': 177,
    },
  ],
  'placements': placements,
};

Future<Map<String, dynamic>> _boutique() async => jsonDecode(
  await File('assets/market/rooms/boutique/room.json').readAsString(),
) as Map<String, dynamic>;

void main() {
  late FakeAdapter adapter;
  late MemoryJsonCache cache;
  late ApiMarketSource source;

  ApiMarketSource build() => ApiMarketSource(
    api: ApiClient(
      baseUrl: 'https://api.test',
      readToken: () async => null,
      onUnauthorized: () {},
      adapter: adapter,
    ),
    cache: cache,
    loadAsset: _disk,
  );

  setUp(() {
    adapter = FakeAdapter();
    cache = MemoryJsonCache();
    source = build();
  });

  test('market_parses_the_servers_answer_including_store_type', () async {
    adapter.reply(
      200,
      _market([
        {
          'plot': 'fashion-1',
          'store_type': 'phone_shop',
          'store': _store('ada'),
        },
      ]),
    );
    final snap = await source.market();
    expect(adapter.requests.single.path, '/market');
    expect(snap.layout.plots, hasLength(2));
    expect(snap.layout.district('fashion')!.name, 'Fashion');
    final p = snap.placementFor('fashion-1')!;
    expect(p.store.slug, 'ada');
    expect(p.store.logoUrl, 'https://cdn.test/logos/ada.png');
    expect(p.storeType, 'phone_shop');
    expect(snap.fromCache, isFalse);
    expect(cache.keys, {'market.v1'});
  });

  test('market_falls_back_to_the_cache_when_offline', () async {
    adapter.reply(
      200,
      _market([
        {'plot': 'fashion-1', 'store_type': 'boutique', 'store': _store('ada')},
      ]),
    );
    await source.market();
    adapter.failWithConnectionError();
    final snap = await build().market();
    expect(snap.fromCache, isTrue);
    expect(snap.placements.single.store.slug, 'ada');
  });

  test('market_offline_with_no_cache_rethrows', () async {
    adapter.failWithTimeout();
    await expectLater(
      source.market(),
      throwsA(isA<ApiError>().having((e) => e.isTimeout, 'timeout', isTrue)),
    );
  });

  test('market_with_zero_placements_parses', () async {
    adapter.reply(200, _market([]));
    final snap = await source.market();
    expect(snap.layout.plots, hasLength(2));
    expect(snap.placements, isEmpty);
  });

  test('a_malformed_placement_is_skipped_not_fatal', () async {
    adapter.reply(
      200,
      _market([
        {'plot': 'fashion-1', 'store_type': 'boutique', 'store': 'oops'},
        {'plot': 'nowhere', 'store_type': 'boutique', 'store': _store('bad')},
        {'plot': 'fashion-2', 'store_type': 'boutique', 'store': _store('ada')},
      ]),
    );
    final snap = await source.market();
    expect(snap.placements.map((p) => p.store.slug), ['ada']);
  });

  test('room_parses_placed_products_and_has_more', () async {
    final type = await _boutique();
    final slots = (type['slots'] as List).cast<Map<String, dynamic>>();
    adapter.reply(200, {
      'store': _store('ada'),
      'store_type': type,
      'placed': [
        {'slot': slots[0]['id'], 'product': _card('a')},
        {'slot': slots[1]['id'], 'product': _card('b')},
      ],
      'has_more': true,
    });
    final room = await source.room('ada');
    expect(adapter.requests.single.path, '/stores/ada/room');
    expect(room.type.id, 'boutique');
    expect(room.placed.map((p) => p.product.id), ['a', 'b']);
    expect(room.hasMore, isTrue);
    expect(cache.keys, {'room.v1.ada'});
  });

  test('room_offline_answers_its_cache', () async {
    adapter.reply(200, {
      'store': _store('ada'),
      'store_type': await _boutique(),
      'placed': <Object?>[],
      'has_more': false,
    });
    await source.room('ada');
    adapter.failWithConnectionError();
    final room = await source.room('ada');
    expect(room.store.slug, 'ada');
  });

  test('a_missing_store_throws_store_not_found', () async {
    adapter.reply(404, {
      'error': {'code': 'store_not_found', 'message': 'No such store.'},
    });
    await expectLater(
      source.room('gone'),
      throwsA(isA<ApiError>().having((e) => e.code, 'code', 'store_not_found')),
    );
  });

  test('a_slug_is_one_encoded_path_segment', () async {
    adapter.reply(404, {
      'error': {'code': 'store_not_found', 'message': 'No such store.'},
    });
    await expectLater(source.room('a/b?c'), throwsA(isA<ApiError>()));
    expect(adapter.requests.single.path, '/stores/a%2Fb%3Fc/room');
  });

  test('store_type_loads_from_the_bundle_without_the_network', () async {
    final type = await source.storeType('grocery');
    expect(type.id, 'grocery');
    expect(adapter.requests, isEmpty);
  });
}
