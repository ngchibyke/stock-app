import 'dart:convert';
import 'dart:io';
import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/market/model/market.dart';
import 'package:market/features/market/model/room.dart';

// Reads the generated placeholder art from disk (tests run from app/).
const _root = 'assets/market';

Map<String, dynamic> _json(String path) =>
    jsonDecode(File(path).readAsStringSync()) as Map<String, dynamic>;

void main() {
  final layout = MarketLayout.fromJson(_json('$_root/layout.json'));

  test('layout_has_60_plots_in_5_districts_with_the_agreed_counts', () {
    expect(layout.districts, hasLength(5));
    final counts = <String, int>{};
    for (final p in layout.plots) {
      counts[p.districtId] = (counts[p.districtId] ?? 0) + 1;
    }
    expect(layout.plots, hasLength(60));
    expect(counts, {
      'fashion': 16,
      'phones-electronics': 12,
      'home-beauty': 12,
      'food-groceries': 12,
      'everything-else': 8,
    });
  });

  test('plots_do_not_overlap_and_lie_inside_the_world', () {
    final world = Offset.zero & layout.world;
    for (final p in layout.plots) {
      expect(world.contains(p.rect.topLeft), isTrue, reason: p.id);
      expect(
        p.rect.right <= world.right && p.rect.bottom <= world.bottom,
        isTrue,
        reason: p.id,
      );
    }
    for (var i = 0; i < layout.plots.length; i++) {
      for (var j = i + 1; j < layout.plots.length; j++) {
        expect(
          layout.plots[i].rect.overlaps(layout.plots[j].rect),
          isFalse,
          reason: '${layout.plots[i].id} and ${layout.plots[j].id}',
        );
      }
    }
  });

  test('every_tile_of_every_level_exists', () {
    expect(layout.world, const Size(2048, 2048));
    expect(layout.tileSize, 256);
    expect(layout.maxLevel, 3);
    for (var level = 0; level <= layout.maxLevel; level++) {
      final n = 1 << level;
      for (var x = 0; x < n; x++) {
        for (var y = 0; y < n; y++) {
          expect(
            File('$_root/tiles/$level/${x}_$y.jpg').existsSync(),
            isTrue,
            reason: 'tile $level/$x/$y',
          );
        }
      }
    }
    expect(File('$_root/${layout.baseImage}').existsSync(), isTrue);
  });

  test('each_store_type_has_room_depth_and_at_least_8_slots', () {
    for (final id in ['boutique', 'phone_shop', 'grocery']) {
      final type = StoreType.fromJson(_json('$_root/rooms/$id/room.json'));
      expect(type.id, id);
      expect(type.slots.length, greaterThanOrEqualTo(8), reason: id);
      expect(File('$_root/rooms/$id/${type.roomImage}').existsSync(), isTrue);
      final depth = File('$_root/rooms/$id/${type.depthImage}');
      expect(depth.existsSync(), isTrue);
      // PNG header: width and height are big-endian at bytes 16..24.
      final b = depth.readAsBytesSync();
      final w = (b[16] << 24) | (b[17] << 16) | (b[18] << 8) | b[19];
      final h = (b[20] << 24) | (b[21] << 16) | (b[22] << 8) | b[23];
      expect(w <= 180 && h <= 320, isTrue, reason: '$id depth is ${w}x$h');
    }
  });

  test('bundled_art_is_at_most_4_MB', () {
    final total = Directory(_root)
        .listSync(recursive: true)
        .whereType<File>()
        .fold<int>(0, (sum, f) => sum + f.lengthSync());
    expect(total, lessThanOrEqualTo(4 * 1024 * 1024));
  });
}
