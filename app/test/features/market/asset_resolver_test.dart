import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/features/market/data/asset_resolver.dart';
import 'package:market/features/market/model/room.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/app_harness.dart';

const _type = StoreType(
  id: 'boutique',
  roomImage: 'room.jpg',
  depthImage: 'depth.png',
  slots: [],
);

AssetResolver _resolverFor(AppConfig config) {
  final container = ProviderContainer(
    overrides: [
      appConfigProvider.overrideWith(() => FakeConfigNotifier(config)),
    ],
  );
  addTearDown(container.dispose);
  return container.read(assetResolverProvider);
}

void main() {
  test('resolver_is_bundled_without_assets_config', () async {
    final r = _resolverFor(AppConfig.empty);
    expect(r, isA<BundledAssetResolver>());
    expect(
      (r.tile(2, 1, 3) as AssetImage).assetName,
      'assets/market/tiles/2/1_3.jpg',
    );
    expect((r.base() as AssetImage).assetName, 'assets/market/base.jpg');
    expect(
      (r.room(_type) as AssetImage).assetName,
      'assets/market/rooms/boutique/room.jpg',
    );
    expect(
      (r.depth(_type) as AssetImage).assetName,
      'assets/market/rooms/boutique/depth.png',
    );
  });

  test('remote_tile_url_is_versioned', () {
    const r = RemoteAssetResolver(
      baseUrl: 'https://cdn.example/',
      version: 'v3',
    );
    final tile = r.tile(2, 1, 3) as CachedNetworkImageProvider;
    expect(tile.url, 'https://cdn.example/v3/tiles/2/1_3.jpg');
    expect(
      (r.base() as CachedNetworkImageProvider).url,
      'https://cdn.example/v3/base.jpg',
    );
  });

  test('remote_room_paths_use_the_type_files', () {
    const r = RemoteAssetResolver(
      baseUrl: 'https://cdn.example',
      version: 'v3',
    );
    expect(
      (r.room(_type) as CachedNetworkImageProvider).url,
      'https://cdn.example/v3/rooms/boutique/room.jpg',
    );
    expect(
      (r.depth(_type) as CachedNetworkImageProvider).url,
      'https://cdn.example/v3/rooms/boutique/depth.png',
    );
  });

  test('remote_is_chosen_only_when_both_values_are_set', () async {
    final both = AppConfig.fromJson({
      'assets': {'version': 'v3', 'base_url': 'https://cdn.example'},
    });
    final noVersion = AppConfig.fromJson({
      'assets': {'base_url': 'https://cdn.example'},
    });
    final container = ProviderContainer(
      overrides: [
        appConfigProvider.overrideWith(() => FakeConfigNotifier(both)),
      ],
    );
    addTearDown(container.dispose);
    await container.read(appConfigProvider.future);
    expect(container.read(assetResolverProvider), isA<RemoteAssetResolver>());
    final r = _resolverFor(noVersion);
    expect(r, isA<BundledAssetResolver>());
  });
}
