import 'package:market/core/api/api_error.dart';
import 'package:market/features/catalog/catalog_models.dart';
import 'package:market/features/catalog/catalog_repository.dart';

StoreSummary fakeStore(String slug, {double? rating = 4.2, int count = 5}) =>
    StoreSummary(
      id: 'id-$slug',
      slug: slug,
      name: 'Store $slug',
      ratingAverage: rating,
      ratingCount: count,
    );

ProductCard fakeCard(
  String id, {
  String store = 'ada',
  bool inStock = true,
  int price = 250000,
  String? photoUrl,
  String? name,
}) => ProductCard(
  id: id,
  name: name ?? 'Thing $id',
  category: 'fashion',
  condition: 'new',
  priceFromKobo: price,
  photoUrl: photoUrl,
  inStock: inStock,
  store: fakeStore(store),
);

/// A catalog held in memory. Set [offline] to make every call fail as if the
/// phone had no network; [calls] records what was asked.
class FakeCatalogRepository implements CatalogRepository {
  List<Category> categoryList = const [
    Category(slug: 'fashion', name: 'Fashion'),
    Category(slug: 'other', name: 'Other'),
  ];
  List<StoreSummary> storeList = [];
  List<ProductCard> productList = [];
  Map<String, ProductDetail> details = {};
  bool offline = false;
  Object? failWith;
  final List<String> calls = [];

  void _check() {
    if (failWith != null) throw failWith!;
    if (offline) throw ApiError.offline();
  }

  Page<T> _page<T>(List<T> all, String? cursor, int limit) {
    final start = cursor == null ? 0 : int.parse(cursor);
    final end = (start + limit).clamp(0, all.length);
    return Page(
      items: all.sublist(start.clamp(0, all.length), end),
      nextCursor: end < all.length ? '$end' : null,
    );
  }

  @override
  Future<List<Category>> categories() async {
    calls.add('categories');
    _check();
    return categoryList;
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
    calls.add('products q=$q category=$category store=$store cursor=$cursor');
    _check();
    var list = productList;
    if (store != null) {
      list = [
        for (final p in list)
          if (p.store.slug == store) p,
      ];
    }
    if (category != null) {
      list = [
        for (final p in list)
          if (p.category == category) p,
      ];
    }
    if (q != null && q.trim().isNotEmpty) {
      final needle = q.trim().toLowerCase();
      list = [
        for (final p in list)
          if (p.name.toLowerCase().contains(needle)) p,
      ];
    }
    return _page(list, cursor, limit);
  }

  @override
  Future<Page<StoreSummary>> stores({
    String? q,
    String? sort,
    String? cursor,
    int limit = 24,
  }) async {
    calls.add('stores cursor=$cursor');
    _check();
    return _page(storeList, cursor, limit);
  }

  @override
  Future<StoreDetail> store(String slug) async {
    calls.add('store $slug');
    _check();
    for (final s in storeList) {
      if (s.slug == slug) {
        return StoreDetail(summary: s, description: 'About $slug');
      }
    }
    throw ApiError(status: 404, code: 'not_found', message: 'Not found.');
  }

  @override
  Future<ProductDetail> product(String id) async {
    calls.add('product $id');
    _check();
    final d = details[id];
    if (d == null) {
      throw ApiError(status: 404, code: 'not_found', message: 'Not found.');
    }
    return d;
  }
}
