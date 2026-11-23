import 'package:material_ui/material_ui.dart';

import '../camera/market_camera.dart';
import '../camera/tiles.dart';
import '../data/asset_resolver.dart';
import '../model/market.dart';

/// The market picture: a small base image stretched under everything, then
/// the tiles for the current zoom on top, each fading in as it arrives.
class TileLayer extends StatelessWidget {
  const TileLayer({
    super.key,
    required this.layout,
    required this.camera,
    required this.resolver,
  });

  final MarketLayout layout;
  final MarketCamera camera;
  final AssetResolver resolver;

  @override
  Widget build(BuildContext context) {
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final level = levelFor(camera, maxLevel: layout.maxLevel, dpr: dpr);
    final n = 1 << level;
    final tw = layout.world.width / n, th = layout.world.height / n;

    Rect onScreen(Rect world) => Rect.fromPoints(
      camera.worldToScreen(world.topLeft),
      camera.worldToScreen(world.bottomRight),
    );

    return Stack(
      children: [
        Positioned.fromRect(
          rect: onScreen(Offset.zero & layout.world),
          child: Image(
            image: resolver.base(),
            fit: BoxFit.fill,
            filterQuality: FilterQuality.low,
            gaplessPlayback: true,
          ),
        ),
        for (final t in visibleTiles(
          camera,
          level: level,
          maxLevel: layout.maxLevel,
        ))
          Positioned.fromRect(
            key: ValueKey(t),
            // Half a pixel of overlap hides seams between neighbours.
            rect: onScreen(Rect.fromLTWH(t.x * tw, t.y * th, tw, th))
                .inflate(0.5),
            child: Image(
              image: resolver.tile(t.level, t.x, t.y),
              fit: BoxFit.fill,
              filterQuality: FilterQuality.medium,
              gaplessPlayback: true,
              excludeFromSemantics: true,
              frameBuilder: (context, child, frame, wasSync) {
                if (wasSync || reduceMotion) return child;
                return AnimatedOpacity(
                  opacity: frame == null ? 0 : 1,
                  duration: const Duration(milliseconds: 150),
                  child: child,
                );
              },
            ),
          ),
      ],
    );
  }
}
