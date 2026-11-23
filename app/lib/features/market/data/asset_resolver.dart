import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../../core/config/app_config.dart';
import '../model/room.dart';

/// Turns market art names into images: from the app bundle, or from the asset
/// host named in `/app/config` (cached on the phone).
abstract class AssetResolver {
  ImageProvider tile(int level, int x, int y);
  ImageProvider base();
  ImageProvider room(StoreType t);
  ImageProvider depth(StoreType t);
}

class BundledAssetResolver implements AssetResolver {
  const BundledAssetResolver();

  static const _root = 'assets/market';

  @override
  ImageProvider tile(int level, int x, int y) =>
      AssetImage('$_root/tiles/$level/${x}_$y.jpg');

  @override
  ImageProvider base() => const AssetImage('$_root/base.jpg');

  @override
  ImageProvider room(StoreType t) =>
      AssetImage('$_root/rooms/${t.id}/${t.roomImage}');

  @override
  ImageProvider depth(StoreType t) =>
      AssetImage('$_root/rooms/${t.id}/${t.depthImage}');
}

/// Art from the asset host. The version is part of every path, so new art
/// never mixes with old cached tiles.
class RemoteAssetResolver implements AssetResolver {
  const RemoteAssetResolver({required this.baseUrl, required this.version});

  final String baseUrl;
  final String version;

  ImageProvider _at(String path) {
    final root = baseUrl.endsWith('/')
        ? baseUrl.substring(0, baseUrl.length - 1)
        : baseUrl;
    final url = '$root/$version/$path';
    return CachedNetworkImageProvider(url, cacheKey: url);
  }

  @override
  ImageProvider tile(int level, int x, int y) =>
      _at('tiles/$level/${x}_$y.jpg');

  @override
  ImageProvider base() => _at('base.jpg');

  @override
  ImageProvider room(StoreType t) => _at(
    'rooms/${Uri.encodeComponent(t.id)}/${Uri.encodeComponent(t.roomImage)}',
  );

  @override
  ImageProvider depth(StoreType t) => _at(
    'rooms/${Uri.encodeComponent(t.id)}/${Uri.encodeComponent(t.depthImage)}',
  );
}

final assetResolverProvider = Provider<AssetResolver>((ref) {
  final config = ref.watch(appConfigProvider).value;
  final base = config?.assetsBaseUrl;
  final version = config?.assetsVersion;
  if (base == null || version == null) return const BundledAssetResolver();
  return RemoteAssetResolver(baseUrl: base, version: version);
});
