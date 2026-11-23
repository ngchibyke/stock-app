import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';
import '../../core/json_read.dart';
import 'catalog_models.dart';

/// The public catalog: categories, products, stores. Nothing here is personal,
/// and none of it is sent with the session token.
abstract class CatalogRepository {
  Future<List<Category>> categories();

  Future<Page<ProductCard>> products({
    String? q,
    String? category,
    String? store,
    String? sort,
    String? cursor,
    int limit = 24,
  });

  Future<Page<StoreSummary>> stores({
    String? q,
    String? sort,
    String? cursor,
    int limit = 24,
  });

  Future<StoreDetail> store(String slug);

  Future<ProductDetail> product(String id);
}

/// Longest search the server accepts.
const maxSearchLength = 80;

ApiError _badResponse() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);

String? _cleanQuery(String? q) {
  final t = q?.trim() ?? '';
  if (t.isEmpty) return null;
  return t.length > maxSearchLength ? t.substring(0, maxSearchLength) : t;
}

class ApiCatalogRepository implements CatalogRepository {
  ApiCatalogRepository(this._api);

  final ApiClient _api;

  Map<String, dynamic> _object(Object? json) {
    if (json is! Map<String, dynamic>) throw _badResponse();
    return json;
  }

  Page<T> _page<T>(Object? json, T? Function(Object?) parse) {
    final map = _object(json);
    if (map['items'] is! List) throw _badResponse();
    return Page(
      items: [
        for (final row in map['items'] as List)
          if (parse(row) case final T item) item,
      ],
      nextCursor: readString(map['next_cursor']),
    );
  }

  Map<String, Object?> _query(Map<String, Object?> values) => {
    for (final e in values.entries)
      if (e.value != null) e.key: '${e.value}',
  };

  @override
  Future<List<Category>> categories() async {
    final json = await _api.get('/categories', auth: false);
    if (json is! List) throw _badResponse();
    return [
      for (final row in readObjects(json))
        if ((readString(row['slug']), readString(row['name'])) case (
          final slug?,
          final name?,
        ))
          Category(slug: slug, name: name),
    ];
  }

  @override
  Future<Page<ProductCard>> products({
    String? q,
    String? category,
    String? store,
    String? sort,
    String? cursor,
    int limit = 24,
  }) async {
    final json = await _api.get(
      '/products',
      auth: false,
      query: _query({
        'q': _cleanQuery(q),
        'category': category,
        'store': store,
        'sort': sort,
        'cursor': cursor,
        'limit': limit,
      }),
    );
    return _page(json, ProductCard.fromJson);
  }

  @override
  Future<Page<StoreSummary>> stores({
    String? q,
    String? sort,
    String? cursor,
    int limit = 24,
  }) async {
    final json = await _api.get(
      '/stores',
      auth: false,
      query: _query({
        'q': _cleanQuery(q),
        'sort': sort,
        'cursor': cursor,
        'limit': limit,
      }),
    );
    return _page(json, StoreSummary.fromJson);
  }

  @override
  Future<StoreDetail> store(String slug) async {
    final json = _object(
      await _api.get('/stores/${Uri.encodeComponent(slug)}', auth: false),
    );
    final summary = StoreSummary.fromJson(json);
    if (summary == null) throw _badResponse();
    return StoreDetail(
      summary: summary,
      description: readString(json['description']) ?? '',
      bannerUrl: readString(json['banner_url']),
    );
  }

  @override
  Future<ProductDetail> product(String id) async {
    final json = _object(
      await _api.get('/products/${Uri.encodeComponent(id)}', auth: false),
    );
    final pid = readString(json['id']);
    final name = readString(json['name']);
    final base = readInt(json['base_price_kobo']);
    if (pid == null || name == null || base == null) throw _badResponse();
    return ProductDetail(
      id: pid,
      storeId: readString(json['store_id']) ?? '',
      name: name,
      description: json['description'] is String
          ? json['description'] as String
          : '',
      condition: readString(json['condition']) ?? 'new',
      brand: readString(json['brand']),
      photoUrls: [
        if (json['photo_urls'] is List)
          for (final u in json['photo_urls'] as List) ?readString(u),
      ],
      variants: [
        for (final v in readObjects(json['variants']))
          if (readString(v['id']) case final vid?)
            Variant(
              id: vid,
              attributes: {
                if (v['attributes'] is Map)
                  for (final e in (v['attributes'] as Map).entries)
                    if (e.key is String && e.value is String)
                      e.key as String: e.value as String,
              },
              availableStock: readInt(v['available_stock']) ?? 0,
              priceKobo: readInt(v['price_kobo']) ?? base,
            ),
      ],
    );
  }
}

final catalogRepositoryProvider = Provider<CatalogRepository>(
  (ref) => ApiCatalogRepository(ref.watch(apiClientProvider)),
);
