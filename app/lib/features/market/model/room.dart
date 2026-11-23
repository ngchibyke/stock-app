import 'dart:ui';

import '../../../core/json_read.dart';
import '../../catalog/catalog_models.dart';

/// A place on a room's shelves where one product card sits. [rect] is in
/// fractions (0 to 1) of the room picture.
class ShelfSlot {
  const ShelfSlot({
    required this.id,
    required this.rect,
    required this.depth,
    required this.tiltY,
  });

  final String id;
  final Rect rect;

  /// 0 is the far wall, 1 the nearest shelf.
  final double depth;

  /// Turn of the shelf, in radians, so the card faces along it.
  final double tiltY;
}

/// A kind of store room: its picture, depth map, slots and light.
class StoreType {
  const StoreType({
    required this.id,
    required this.roomImage,
    required this.depthImage,
    required this.slots,
    this.lightTint = const Color(0xFFFFFFFF),
    this.shadowOpacity = 0.3,
  });

  /// Null when the row has no id or pictures. Slots outside the picture are
  /// dropped.
  static StoreType? tryFromJson(Object? json) {
    if (json is! Map<String, dynamic>) return null;
    final id = readString(json['id']);
    final room = readString(json['room_image']);
    final depth = readString(json['depth_image']);
    if (id == null || room == null || depth == null) return null;
    final light = json['light'];
    final slots = <ShelfSlot>[];
    final seen = <String>{};
    for (final s in readObjects(json['slots'])) {
      final sid = readString(s['id']);
      final x = readDouble(s['x']), y = readDouble(s['y']);
      final w = readDouble(s['w']), h = readDouble(s['h']);
      if (sid == null || x == null || y == null || w == null || h == null) {
        continue;
      }
      final inside =
          x >= 0 && y >= 0 && w > 0 && h > 0 && x + w <= 1 && y + h <= 1;
      if (!inside || !seen.add(sid)) continue;
      slots.add(
        ShelfSlot(
          id: sid,
          rect: Rect.fromLTWH(x, y, w, h),
          depth: (readDouble(s['depth']) ?? 0.5).clamp(0.0, 1.0),
          tiltY: (readDouble(s['tilt_y']) ?? 0).clamp(-0.6, 0.6),
        ),
      );
    }
    return StoreType(
      id: id,
      roomImage: room,
      depthImage: depth,
      slots: slots,
      lightTint: light is Map ? _hex(light['tint']) : const Color(0xFFFFFFFF),
      shadowOpacity: light is Map
          ? (readDouble(light['shadow_opacity']) ?? 0.3).clamp(0.0, 1.0)
          : 0.3,
    );
  }

  factory StoreType.fromJson(Map<String, dynamic> json) {
    final t = tryFromJson(json);
    if (t == null) {
      throw const FormatException('store type has no id or pictures');
    }
    return t;
  }

  final String id;
  final String roomImage;
  final String depthImage;
  final List<ShelfSlot> slots;
  final Color lightTint;
  final double shadowOpacity;

  ShelfSlot? slot(String id) {
    for (final s in slots) {
      if (s.id == id) return s;
    }
    return null;
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'room_image': roomImage,
    'depth_image': depthImage,
    'light': {
      'tint':
          '#${(lightTint.toARGB32() & 0xFFFFFF).toRadixString(16).padLeft(6, '0')}',
      'shadow_opacity': shadowOpacity,
    },
    'slots': [
      for (final s in slots)
        {
          'id': s.id,
          'x': s.rect.left,
          'y': s.rect.top,
          'w': s.rect.width,
          'h': s.rect.height,
          'depth': s.depth,
          'tilt_y': s.tiltY,
        },
    ],
  };

  static Color _hex(Object? v) {
    final s = readString(v);
    final m = s == null ? null : RegExp(r'^#([0-9a-fA-F]{6})$').firstMatch(s);
    if (m == null) return const Color(0xFFFFFFFF);
    return Color(0xFF000000 | int.parse(m.group(1)!, radix: 16));
  }
}

class PlacedProduct {
  const PlacedProduct({required this.slotId, required this.product});

  final String slotId;
  final ProductCard product;
}

/// A store's room with the products on its shelves.
class RoomView {
  const RoomView({
    required this.store,
    required this.type,
    required this.placed,
    required this.hasMore,
  });

  /// `RoomJson`. Products that cannot be bought, or placed on unknown or
  /// already-filled slots, are dropped: the room never shows what cannot be
  /// bought.
  factory RoomView.fromJson(Map<String, dynamic> json) {
    final store = StoreSummary.fromJson(json['store']);
    final type = StoreType.tryFromJson(json['store_type']);
    if (store == null || type == null) {
      throw const FormatException('room has no store or store type');
    }
    final placed = <PlacedProduct>[];
    final filled = <String>{};
    for (final row in readObjects(json['placed'])) {
      final slot = readString(row['slot']);
      final product = ProductCard.fromJson(row['product']);
      if (slot == null || product == null || !product.inStock) continue;
      if (type.slot(slot) == null || !filled.add(slot)) continue;
      placed.add(PlacedProduct(slotId: slot, product: product));
    }
    return RoomView(
      store: store,
      type: type,
      placed: placed,
      hasMore: json['has_more'] == true,
    );
  }

  final StoreSummary store;
  final StoreType type;
  final List<PlacedProduct> placed;
  final bool hasMore;

  Map<String, dynamic> toJson() => {
    'store': store.toJson(),
    'store_type': type.toJson(),
    'placed': [
      for (final p in placed) {'slot': p.slotId, 'product': p.product.toJson()},
    ],
    'has_more': hasMore,
  };
}
