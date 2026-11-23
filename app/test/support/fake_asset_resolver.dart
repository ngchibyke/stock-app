import 'dart:convert';
import 'dart:typed_data';

import 'package:market/features/market/camera/tiles.dart';
import 'package:market/features/market/data/asset_resolver.dart';
import 'package:market/features/market/model/room.dart';
import 'package:material_ui/material_ui.dart';

/// A 1 x 1 transparent PNG.
final Uint8List onePixelPng = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==',
);

/// Serves the same tiny image for everything and records which tiles were
/// asked for.
class FakeAssetResolver implements AssetResolver {
  final Set<TileKey> tiles = {};
  final ImageProvider _image = MemoryImage(onePixelPng);
  late final ImageProvider baseImage = MemoryImage(onePixelPng);

  @override
  ImageProvider tile(int level, int x, int y) {
    tiles.add(TileKey(level, x, y));
    return _image;
  }

  @override
  ImageProvider base() => baseImage;

  @override
  ImageProvider room(StoreType t) => _image;

  @override
  ImageProvider depth(StoreType t) => _image;
}
