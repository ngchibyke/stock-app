import 'dart:async';

import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../catalog/catalog_repository.dart';
import '../model/market.dart';
import '../model/room.dart';
import 'json_cache.dart';
import 'placeholder_market_source.dart';

/// Where the market and its rooms come from. The shapes are the contract the
/// server's `GET /market` and `GET /stores/{slug}/room` will answer (plan 7d).
abstract class MarketSource {
  Future<MarketSnapshot> market();

  /// Throws an `ApiError` with code `store_not_found` when the store is gone.
  Future<RoomView> room(String slug);

  /// A store type's room, from the app bundle: no network, so the room's
  /// picture can show before its products arrive.
  Future<StoreType> storeType(String id);
}

final jsonCacheProvider = FutureProvider<JsonCache>(
  (ref) async => PrefsJsonCache(await SharedPreferences.getInstance()),
);

/// The placeholder source until the server has the market routes.
final marketSourceProvider = FutureProvider<MarketSource>(
  (ref) async => PlaceholderMarketSource(
    catalog: ref.watch(catalogRepositoryProvider),
    cache: await ref.watch(jsonCacheProvider.future),
    loadAsset: rootBundle.loadString,
  ),
);

final marketProvider = FutureProvider<MarketSnapshot>(
  (ref) async => (await ref.watch(marketSourceProvider.future)).market(),
);

/// How long a room stays loaded with nobody looking at it.
const roomKeepAlive = Duration(minutes: 5);

/// A room stays loaded for [roomKeepAlive] after the last listener leaves, so
/// a prefetch made on the way in is still there when the camera lands. A
/// prefetch nobody opens goes after the same time; coming back resets it.
final roomProvider = FutureProvider.autoDispose.family<RoomView, String>((
  ref,
  slug,
) async {
  final link = ref.keepAlive();
  Timer? timer;
  void countDown() {
    timer?.cancel();
    timer = Timer(roomKeepAlive, link.close);
  }

  countDown();
  ref
    ..onResume(() => timer?.cancel())
    ..onCancel(countDown)
    ..onDispose(() => timer?.cancel());
  return (await ref.watch(marketSourceProvider.future)).room(slug);
});

/// A store type, kept for the session (there are only a few).
final storeTypeProvider = FutureProvider.family<StoreType, String>(
  (ref, id) async =>
      (await ref.watch(marketSourceProvider.future)).storeType(id),
);
