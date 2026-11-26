import 'dart:io';

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/market/data/asset_resolver.dart';
import 'package:market/features/market/model/room.dart';
import 'package:material_ui/material_ui.dart' show ImageProvider;

/// The art tool (`server art build`) and this resolver must agree on file names. The server
/// builds a small pack and compares its files with `art_files.txt`; this test compares the
/// addresses the resolver can ask for with the same file, so neither side can drift alone.
/// It reads `../server`, so it needs the whole repository checked out.
void main() {
  const base = 'https://cdn.test/market';
  const version = 'v1';
  const resolver = RemoteAssetResolver(baseUrl: base, version: version);

  String path(ImageProvider p) {
    final url = (p as CachedNetworkImageProvider).url;
    expect(url, startsWith('$base/$version/'));
    return url.substring('$base/$version/'.length);
  }

  test('the_resolver_asks_for_exactly_the_pictures_the_tool_builds', () {
    final golden = File('../server/tests/fixtures/art_files.txt')
        .readAsLinesSync()
        .where((l) => l.isNotEmpty)
        .toSet();

    // The fixture pack: levels 0 to 3 and one room type.
    const maxLevel = 3;
    final asked = <String>{path(resolver.base())};
    for (var level = 0; level <= maxLevel; level++) {
      final n = 1 << level;
      for (var x = 0; x < n; x++) {
        for (var y = 0; y < n; y++) {
          asked.add(path(resolver.tile(level, x, y)));
        }
      }
    }
    const boutique = StoreType(
      id: 'boutique',
      roomImage: 'room.jpg',
      depthImage: 'depth.png',
      slots: [],
    );
    asked
      ..add(path(resolver.room(boutique)))
      ..add(path(resolver.depth(boutique)));

    // Everything the app asks for is built, and every picture that is built is asked for.
    final pictures = golden
        .where((f) => f.endsWith('.jpg') || f.endsWith('.png'))
        .toSet();
    expect(asked.difference(golden), isEmpty, reason: 'asked but not built');
    expect(
      pictures.difference(asked),
      isEmpty,
      reason: 'built but never asked',
    );
    // The rest are data files: the manifest, the layout and each room's json.
    expect(golden.difference(pictures), {
      'manifest.json',
      'layout.json',
      'rooms/boutique/room.json',
    });
  });
}
