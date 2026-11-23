import 'dart:math' as math;

import 'market_camera.dart';

/// One tile of the market picture. Level L has 2^L by 2^L tiles.
class TileKey {
  const TileKey(this.level, this.x, this.y);

  final int level;
  final int x;
  final int y;

  @override
  bool operator ==(Object other) =>
      other is TileKey && other.level == level && other.x == x && other.y == y;

  @override
  int get hashCode => Object.hash(level, x, y);

  @override
  String toString() => 'TileKey($level, $x, $y)';
}

/// The shallowest level whose pixels are at least as dense as the screen's.
/// At level L one world unit is 2^(L - maxLevel) tile pixels.
int levelFor(MarketCamera c, {required int maxLevel, required double dpr}) {
  final devicePerWorld = c.scale * dpr;
  final level = (maxLevel + math.log(devicePerWorld) / math.ln2).ceil();
  return level.clamp(0, maxLevel);
}

/// The tiles of [level] that overlap the view, plus one tile all round, so a
/// short pan never shows a gap.
List<TileKey> visibleTiles(
  MarketCamera c, {
  required int level,
  required int maxLevel,
}) {
  final n = 1 << level;
  final tw = c.world.width / n, th = c.world.height / n;
  final v = c.visibleWorld;
  const edge = 1e-9;
  final x0 = ((v.left / tw).floor() - 1).clamp(0, n - 1);
  final x1 = (((v.right - edge) / tw).floor() + 1).clamp(0, n - 1);
  final y0 = ((v.top / th).floor() - 1).clamp(0, n - 1);
  final y1 = (((v.bottom - edge) / th).floor() + 1).clamp(0, n - 1);
  return [
    for (var x = x0; x <= x1; x++)
      for (var y = y0; y <= y1; y++) TileKey(level, x, y),
  ];
}
