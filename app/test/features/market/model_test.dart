import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/market/model/market.dart';
import 'package:market/features/market/model/room.dart';
import 'package:material_ui/material_ui.dart';

Map<String, dynamic> storeJson(String slug, {Object? rating = 4.5}) => {
  'id': 'id-$slug',
  'slug': slug,
  'name': 'Store $slug',
  'logo_url': null,
  'rating_average': rating,
  'rating_count': 3,
};

Map<String, dynamic> productJson(String id, {bool inStock = true}) => {
  'id': id,
  'name': 'Product $id',
  'category': 'fashion',
  'condition': 'new',
  'price_from_kobo': 150000,
  'photo_url': null,
  'in_stock': inStock,
  'store': storeJson('ada'),
};

Map<String, dynamic> marketJson() => {
  'version': 'p1',
  'world': {'w': 2048, 'h': 2048},
  'tile_size': 256,
  'max_level': 3,
  'base_image': 'base.jpg',
  'districts': <Object?>[
    {'id': 'fashion', 'name': 'Fashion'},
    {'id': 'food', 'name': 'Food and groceries'},
  ],
  'plots': <Object?>[
    {'id': 'p1', 'district': 'fashion', 'x': 10, 'y': 10, 'w': 40, 'h': 30},
    {'id': 'p2', 'district': 'food', 'x': 100, 'y': 10, 'w': 40, 'h': 30},
  ],
  'placements': <Object?>[
    {'plot': 'p1', 'store_type': 'boutique', 'store': storeJson('ada')},
  ],
};

Map<String, dynamic> typeJson() => {
  'id': 'boutique',
  'room_image': 'room.jpg',
  'depth_image': 'depth.png',
  'light': {'tint': '#FFE0B0', 'shadow_opacity': 0.4},
  'slots': <Object?>[
    {
      'id': 's1',
      'x': 0.1,
      'y': 0.2,
      'w': 0.2,
      'h': 0.15,
      'depth': 0.6,
      'tilt_y': 0.1,
    },
    {
      'id': 's2',
      'x': 0.5,
      'y': 0.2,
      'w': 0.2,
      'h': 0.15,
      'depth': 0.4,
      'tilt_y': 0,
    },
  ],
};

void main() {
  group('market', () {
    test('parses_the_full_market_json', () {
      final snap = MarketSnapshot.fromJson(marketJson());
      final layout = snap.layout;
      expect(layout.version, 'p1');
      expect(layout.world, const Size(2048, 2048));
      expect(layout.tileSize, 256);
      expect(layout.maxLevel, 3);
      expect(layout.districts.map((d) => d.id), ['fashion', 'food']);
      expect(layout.plots.first.rect, const Rect.fromLTWH(10, 10, 40, 30));
      expect(snap.placementFor('p1')!.store.slug, 'ada');
      expect(snap.placementFor('p1')!.storeType, 'boutique');
      expect(snap.placementFor('p2'), isNull);
      expect(snap.fromCache, isFalse);
    });

    test('skips_plots_with_missing_or_negative_sizes_and_keeps_the_rest', () {
      final json = marketJson();
      (json['plots'] as List).addAll([
        {'id': 'bad1', 'district': 'food', 'x': 1, 'y': 1, 'w': -4, 'h': 3},
        {'id': 'bad2', 'district': 'food', 'x': 1, 'y': 1},
        'not a plot',
      ]);
      final layout = MarketLayout.fromJson(json);
      expect(layout.plots.map((p) => p.id), ['p1', 'p2']);
    });

    test('unusable_world_throws_format_exception', () {
      final json = marketJson()..['world'] = {'w': 0, 'h': 10};
      expect(() => MarketLayout.fromJson(json), throwsFormatException);
    });

    test(
      'skips_placements_for_unknown_plots_and_duplicate_plots_keeps_first',
      () {
        final json = marketJson();
        (json['placements'] as List).addAll([
          {
            'plot': 'nowhere',
            'store_type': 'boutique',
            'store': storeJson('x'),
          },
          {'plot': 'p1', 'store_type': 'grocery', 'store': storeJson('y')},
        ]);
        final snap = MarketSnapshot.fromJson(json);
        expect(snap.placements, hasLength(1));
        expect(snap.placementFor('p1')!.store.slug, 'ada');
      },
    );

    test('null_rating_is_kept_as_null', () {
      final json = marketJson();
      (json['placements'] as List)
        ..clear()
        ..add({
          'plot': 'p1',
          'store_type': 'boutique',
          'store': storeJson('z', rating: null),
        });
      final snap = MarketSnapshot.fromJson(json);
      expect(snap.placementFor('p1')!.store.ratingAverage, isNull);
    });

    test('round_trips_through_json', () {
      final snap = MarketSnapshot.fromJson(marketJson());
      final again = MarketSnapshot.fromJson(snap.toJson());
      expect(again.layout.plots.length, 2);
      expect(again.placementFor('p1')!.store.name, 'Store ada');
    });
  });

  group('room', () {
    test('slot_rects_outside_0_1_are_dropped', () {
      final json = typeJson();
      (json['slots'] as List).add({
        'id': 's3',
        'x': 0.9,
        'y': 0.2,
        'w': 0.3,
        'h': 0.1,
        'depth': 0.5,
        'tilt_y': 0,
      });
      final type = StoreType.fromJson(json);
      expect(type.slots.map((s) => s.id), ['s1', 's2']);
      expect(type.slots.first.rect, const Rect.fromLTWH(0.1, 0.2, 0.2, 0.15));
      expect(type.slots.first.depth, 0.6);
    });

    test('bad_tint_falls_back_to_white_and_shadow_is_clamped_0_to_1', () {
      final json = typeJson()
        ..['light'] = {'tint': 'orange', 'shadow_opacity': 7};
      final type = StoreType.fromJson(json);
      expect(type.lightTint, const Color(0xFFFFFFFF));
      expect(type.shadowOpacity, 1.0);
      expect(StoreType.fromJson(typeJson()).lightTint, const Color(0xFFFFE0B0));
    });

    test('room_drops_placed_products_that_are_out_of_stock', () {
      final room = RoomView.fromJson({
        'store': storeJson('ada'),
        'store_type': typeJson(),
        'placed': [
          {'slot': 's1', 'product': productJson('a')},
          {'slot': 's2', 'product': productJson('b', inStock: false)},
        ],
        'has_more': false,
      });
      expect(room.placed.map((p) => p.product.id), ['a']);
    });

    test('room_drops_placements_for_unknown_slots', () {
      final room = RoomView.fromJson({
        'store': storeJson('ada'),
        'store_type': typeJson(),
        'placed': [
          {'slot': 'nope', 'product': productJson('a')},
          {'slot': 's2', 'product': productJson('b')},
        ],
        'has_more': true,
      });
      expect(room.placed.single.slotId, 's2');
      expect(room.hasMore, isTrue);
    });
  });
}
