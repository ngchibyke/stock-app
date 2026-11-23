import 'dart:ui';

import '../../../core/json_read.dart';
import '../../catalog/catalog_models.dart';

/// A part of the market that holds one kind of store.
class District {
  const District({required this.id, required this.name});

  final String id;
  final String name;
}

/// A place a store can stand, in world units (pixels of the deepest tiles).
class Plot {
  const Plot({required this.id, required this.districtId, required this.rect});

  final String id;
  final String districtId;
  final Rect rect;
}

/// A store standing on a plot, and the room it opens into.
class Placement {
  const Placement({
    required this.plotId,
    required this.store,
    required this.storeType,
  });

  final String plotId;
  final StoreSummary store;
  final String storeType;
}

/// The fixed shape of the market: the picture and the plots drawn over it.
class MarketLayout {
  const MarketLayout({
    required this.version,
    required this.world,
    required this.tileSize,
    required this.maxLevel,
    required this.baseImage,
    required this.districts,
    required this.plots,
  });

  /// Throws [FormatException] only when the picture itself is unusable; a bad
  /// district or plot row is skipped.
  factory MarketLayout.fromJson(Map<String, dynamic> json) {
    final world = json['world'];
    final w = world is Map ? readDouble(world['w']) : null;
    final h = world is Map ? readDouble(world['h']) : null;
    final tile = readInt(json['tile_size']);
    final maxLevel = readInt(json['max_level']);
    if (w == null ||
        h == null ||
        w <= 0 ||
        h <= 0 ||
        tile == null ||
        tile <= 0 ||
        maxLevel == null ||
        maxLevel < 0) {
      throw const FormatException('market layout has no usable picture');
    }
    final districts = <District>[
      for (final d in readObjects(json['districts']))
        if ((readString(d['id']), readString(d['name'])) case (
          final id?,
          final name?,
        ))
          District(id: id, name: name),
    ];
    final plots = <Plot>[];
    final seen = <String>{};
    for (final p in readObjects(json['plots'])) {
      final id = readString(p['id']);
      final district = readString(p['district']);
      final x = readDouble(p['x']), y = readDouble(p['y']);
      final pw = readDouble(p['w']), ph = readDouble(p['h']);
      if (id == null ||
          district == null ||
          x == null ||
          y == null ||
          pw == null ||
          ph == null ||
          pw <= 0 ||
          ph <= 0 ||
          !seen.add(id)) {
        continue;
      }
      plots.add(
        Plot(id: id, districtId: district, rect: Rect.fromLTWH(x, y, pw, ph)),
      );
    }
    return MarketLayout(
      version: readString(json['version']) ?? '0',
      world: Size(w, h),
      tileSize: tile,
      maxLevel: maxLevel,
      baseImage: readString(json['base_image']) ?? 'base.jpg',
      districts: districts,
      plots: plots,
    );
  }

  final String version;
  final Size world;
  final int tileSize;
  final int maxLevel;
  final String baseImage;
  final List<District> districts;
  final List<Plot> plots;

  District? district(String id) {
    for (final d in districts) {
      if (d.id == id) return d;
    }
    return null;
  }

  Map<String, dynamic> toJson() => {
    'version': version,
    'world': {'w': world.width, 'h': world.height},
    'tile_size': tileSize,
    'max_level': maxLevel,
    'base_image': baseImage,
    'districts': [
      for (final d in districts) {'id': d.id, 'name': d.name},
    ],
    'plots': [
      for (final p in plots)
        {
          'id': p.id,
          'district': p.districtId,
          'x': p.rect.left,
          'y': p.rect.top,
          'w': p.rect.width,
          'h': p.rect.height,
        },
    ],
  };
}

/// The market as the buyer sees it now: its layout and who stands where.
class MarketSnapshot {
  MarketSnapshot({
    required this.layout,
    required this.placements,
    this.fromCache = false,
  }) : _byPlot = {for (final p in placements) p.plotId: p};

  /// `MarketJson`. Placements on unknown plots, or a second one on a plot, are
  /// skipped.
  factory MarketSnapshot.fromJson(
    Map<String, dynamic> json, {
    bool fromCache = false,
  }) {
    final layout = MarketLayout.fromJson(json);
    final plotIds = {for (final p in layout.plots) p.id};
    final placements = <Placement>[];
    final taken = <String>{};
    for (final row in readObjects(json['placements'])) {
      final plot = readString(row['plot']);
      final store = StoreSummary.fromJson(row['store']);
      if (plot == null ||
          store == null ||
          !plotIds.contains(plot) ||
          !taken.add(plot)) {
        continue;
      }
      placements.add(
        Placement(
          plotId: plot,
          store: store,
          storeType: readString(row['store_type']) ?? 'boutique',
        ),
      );
    }
    return MarketSnapshot(
      layout: layout,
      placements: placements,
      fromCache: fromCache,
    );
  }

  final MarketLayout layout;
  final List<Placement> placements;

  /// True when this came from the phone's cache because the server could not
  /// be reached.
  final bool fromCache;
  final Map<String, Placement> _byPlot;

  Placement? placementFor(String plotId) => _byPlot[plotId];

  Plot? plotOf(Placement p) {
    for (final plot in layout.plots) {
      if (plot.id == p.plotId) return plot;
    }
    return null;
  }

  MarketSnapshot asCached() =>
      MarketSnapshot(layout: layout, placements: placements, fromCache: true);

  Map<String, dynamic> toJson() => {
    ...layout.toJson(),
    'placements': [
      for (final p in placements)
        {
          'plot': p.plotId,
          'store_type': p.storeType,
          'store': p.store.toJson(),
        },
    ],
  };
}
