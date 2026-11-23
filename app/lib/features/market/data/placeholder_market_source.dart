import 'dart:convert';

import '../../../core/api/api_error.dart';
import '../../catalog/catalog_models.dart';
import '../../catalog/catalog_repository.dart';
import '../model/market.dart';
import '../model/room.dart';
import 'json_cache.dart';
import 'market_source.dart';

const _marketKey = 'market.v1';
String _roomKey(String slug) => 'room.v1.$slug';

/// The room each district's stores open into, until stores carry a type.
const _typeByDistrict = {
  'phones-electronics': 'phone_shop',
  'food-groceries': 'grocery',
};

bool _noNetwork(Object e) => e is ApiError && (e.isOffline || e.isTimeout);

/// Builds the market contract from the bundled layout and the routes the
/// server already has, until the server answers `GET /market` itself.
///
/// Stores carry no category yet, so a store's district here is only where it
/// happens to stand: active stores sorted by slug fill the plots in order.
class PlaceholderMarketSource implements MarketSource {
  PlaceholderMarketSource({
    required this._catalog,
    required this._cache,
    required this._loadAsset,
  });

  final CatalogRepository _catalog;
  final JsonCache _cache;
  final Future<String> Function(String path) _loadAsset;
  MarketSnapshot? _last;

  Future<Map<String, dynamic>> _asset(String path) async =>
      jsonDecode(await _loadAsset(path)) as Map<String, dynamic>;

  @override
  Future<MarketSnapshot> market() async {
    final layout = MarketLayout.fromJson(
      await _asset('assets/market/layout.json'),
    );
    final List<StoreSummary> stores;
    try {
      stores = await _activeStores(layout.plots.length);
    } on Object catch (e) {
      if (!_noNetwork(e)) rethrow;
      final cached = await _cache.read(_marketKey);
      if (cached == null) rethrow;
      return _last = MarketSnapshot.fromJson(cached, fromCache: true);
    }
    stores.sort((a, b) => a.slug.compareTo(b.slug));
    final placements = <Placement>[
      for (var i = 0; i < stores.length && i < layout.plots.length; i++)
        Placement(
          plotId: layout.plots[i].id,
          store: stores[i],
          storeType: _typeByDistrict[layout.plots[i].districtId] ?? 'boutique',
        ),
    ];
    final snap = MarketSnapshot(layout: layout, placements: placements);
    await _cache.write(_marketKey, snap.toJson());
    return _last = snap;
  }

  Future<List<StoreSummary>> _activeStores(int max) async {
    final all = <StoreSummary>[];
    String? cursor;
    do {
      final page = await _catalog.stores(
        sort: 'newest',
        cursor: cursor,
        limit: 50,
      );
      all.addAll(page.items);
      cursor = page.nextCursor;
    } while (cursor != null && all.length < max);
    // Sorted before the cut, so which stores stand is stable between loads.
    all.sort((a, b) => a.slug.compareTo(b.slug));
    return all.length > max ? all.sublist(0, max) : all;
  }

  Future<String> _typeOf(String slug) async {
    var snap = _last;
    if (snap == null) {
      final cached = await _cache.read(_marketKey);
      if (cached != null) snap = MarketSnapshot.fromJson(cached);
    }
    for (final p in snap?.placements ?? const <Placement>[]) {
      if (p.store.slug == slug) return p.storeType;
    }
    return 'boutique';
  }

  @override
  Future<RoomView> room(String slug) async {
    try {
      final StoreDetail detail;
      try {
        detail = await _catalog.store(slug);
      } on ApiError catch (e) {
        if (e.status == 404) {
          throw ApiError(
            status: 404,
            code: 'store_not_found',
            message: 'This store has closed.',
          );
        }
        rethrow;
      }
      final typeId = await _typeOf(slug);
      final type = StoreType.fromJson(
        await _asset('assets/market/rooms/$typeId/room.json'),
      );
      final page = await _catalog.products(
        store: slug,
        sort: 'newest',
        limit: type.slots.length,
      );
      final buyable = [
        for (final p in page.items)
          if (p.inStock) p,
      ];
      final room = RoomView(
        store: detail.summary,
        type: type,
        placed: [
          for (var i = 0; i < buyable.length && i < type.slots.length; i++)
            PlacedProduct(slotId: type.slots[i].id, product: buyable[i]),
        ],
        hasMore: page.nextCursor != null || buyable.length > type.slots.length,
      );
      await _cache.write(_roomKey(slug), room.toJson());
      return room;
    } on Object catch (e) {
      if (!_noNetwork(e)) rethrow;
      final cached = await _cache.read(_roomKey(slug));
      if (cached == null) rethrow;
      return RoomView.fromJson(cached);
    }
  }
}
