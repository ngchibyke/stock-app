import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/catalog/catalog_repository.dart';

import '../../support/fakes.dart';

CatalogRepository _repo(FakeAdapter adapter) => ApiCatalogRepository(
  ApiClient(
    baseUrl: 'https://api.test',
    readToken: () async => 'tok',
    onUnauthorized: () {},
    adapter: adapter,
  ),
);

Map<String, Object?> store(String slug) => {
  'id': 'id-$slug',
  'slug': slug,
  'name': 'Store $slug',
  'rating_average': null,
  'rating_count': 0,
};

Map<String, Object?> card(String id, {Object? photo}) => {
  'id': id,
  'name': 'Thing $id',
  'category': 'fashion',
  'condition': 'new',
  'price_from_kobo': 250000,
  'photo_url': photo,
  'in_stock': true,
  'store': store('ada'),
};

void main() {
  test('categories_parse_and_skip_bad_rows', () async {
    final a = FakeAdapter()
      ..reply(200, [
        {'slug': 'fashion', 'name': 'Fashion'},
        {'slug': '', 'name': 'Empty'},
        'nope',
        {'slug': 'other', 'name': 'Other'},
      ]);
    final list = await _repo(a).categories();
    expect(list.map((c) => c.slug), ['fashion', 'other']);
    expect(a.requests.single.path, '/categories');
  });

  test('products_send_q_as_a_query_value_trimmed_to_80_chars', () async {
    final a = FakeAdapter()
      ..reply(200, {
        'items': [card('p1')],
        'next_cursor': 'c2',
      });
    final long = '  ${'x' * 100}  ';
    final page = await _repo(a)
        .products(q: long, category: 'fashion', cursor: 'c1');
    final req = a.requests.single;
    expect(req.path, '/products');
    expect(req.query['q'], 'x' * 80);
    expect(req.query['category'], 'fashion');
    expect(req.query['cursor'], 'c1');
    expect(req.query['limit'], '24');
    expect(page.items.single.id, 'p1');
    expect(page.nextCursor, 'c2');
  });

  test('blank_q_is_not_sent', () async {
    final a = FakeAdapter()..reply(200, {'items': [], 'next_cursor': null});
    await _repo(a).products(q: '   ');
    expect(a.requests.single.query.containsKey('q'), isFalse);
  });

  test('slug_and_id_are_encoded_in_the_path', () async {
    final a = FakeAdapter()
      ..reply(200, {...store('a/b'), 'description': 'Hi', 'banner_url': null})
      ..reply(200, {
        'id': 'x/y',
        'store_id': 's1',
        'name': 'Dress',
        'description': '',
        'condition': 'new',
        'base_price_kobo': 100,
        'photo_urls': [],
        'variants': [],
      });
    await _repo(a).store('a/b');
    await _repo(a).product('x/y');
    expect(a.requests[0].path, '/stores/a%2Fb');
    expect(a.requests[1].path, '/products/x%2Fy');
  });

  test('variant_without_price_uses_base_price', () async {
    final a = FakeAdapter()
      ..reply(200, {
        'id': 'p1',
        'store_id': 's1',
        'name': 'Dress',
        'description': 'Ankara',
        'brand': null,
        'condition': 'new',
        'base_price_kobo': 500000,
        'photo_urls': ['https://cdn.test/a.jpg'],
        'variants': [
          {
            'id': 'v1',
            'attributes': {'size': 'M'},
            'available_stock': 2,
            'price_kobo': null,
          },
          {
            'id': 'v2',
            'attributes': {'size': 'L'},
            'available_stock': 0,
            'price_kobo': 550000,
          },
        ],
      });
    final p = await _repo(a).product('p1');
    expect(p.variants.map((v) => v.priceKobo), [500000, 550000]);
    expect(p.variants.first.attributes, {'size': 'M'});
    expect(p.photoUrls, ['https://cdn.test/a.jpg']);
  });

  test('product_with_null_photo_url_parses', () async {
    final a = FakeAdapter()
      ..reply(200, {
        'items': [card('p1', photo: null)],
        'next_cursor': null,
      });
    final page = await _repo(a).products();
    expect(page.items.single.photoUrl, isNull);
  });

  test('a_non_json_answer_is_bad_response', () async {
    final a = FakeAdapter()..reply(200, ['not', 'a', 'page']);
    await expectLater(
      _repo(a).stores(),
      throwsA(isA<ApiError>().having((e) => e.code, 'code', 'bad_response')),
    );
  });

  test('public_routes_send_no_token', () async {
    final a = FakeAdapter()..reply(200, {'items': [], 'next_cursor': null});
    await _repo(a).stores(sort: 'newest');
    expect(a.requests.single.headers.containsKey('Authorization'), isFalse);
    expect(a.requests.single.query['sort'], 'newest');
  });
}
