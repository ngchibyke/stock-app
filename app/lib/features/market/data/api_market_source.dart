import 'dart:convert';

import '../../../core/api/api_client.dart';
import '../../../core/api/api_error.dart';
import '../model/market.dart';
import '../model/room.dart';
import 'json_cache.dart';
import 'market_source.dart';

const _marketKey = 'market.v1';
String _roomKey(String slug) => 'room.v1.$slug';

bool _noNetwork(Object e) => e is ApiError && (e.isOffline || e.isTimeout);

/// The market and its rooms as the server answers them: `GET /market` and
/// `GET /stores/{slug}/room`. Both are public, so no token is sent. The last
/// good answers are cached so the market still opens without a network.
class ApiMarketSource implements MarketSource {
  ApiMarketSource({
    required this._api,
    required this._cache,
    required this._loadAsset,
  });

  final ApiClient _api;
  final JsonCache _cache;
  final Future<String> Function(String path) _loadAsset;

  @override
  Future<MarketSnapshot> market() async {
    try {
      final json = await _api.get('/market', auth: false);
      if (json is! Map<String, dynamic>) throw _badResponse();
      final snap = MarketSnapshot.fromJson(json);
      await _cache.write(_marketKey, json);
      return snap;
    } on Object catch (e) {
      if (!_noNetwork(e)) rethrow;
      final cached = await _cache.read(_marketKey);
      if (cached == null) rethrow;
      return MarketSnapshot.fromJson(cached, fromCache: true);
    }
  }

  @override
  Future<RoomView> room(String slug) async {
    try {
      final json = await _api.get(
        '/stores/${Uri.encodeComponent(slug)}/room',
        auth: false,
      );
      if (json is! Map<String, dynamic>) throw _badResponse();
      final room = RoomView.fromJson(json);
      await _cache.write(_roomKey(slug), json);
      return room;
    } on Object catch (e) {
      if (!_noNetwork(e)) rethrow;
      final cached = await _cache.read(_roomKey(slug));
      if (cached == null) rethrow;
      return RoomView.fromJson(cached);
    }
  }

  @override
  Future<StoreType> storeType(String id) async => StoreType.fromJson(
    jsonDecode(await _loadAsset('assets/market/rooms/$id/room.json'))
        as Map<String, dynamic>,
  );
}

ApiError _badResponse() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);
