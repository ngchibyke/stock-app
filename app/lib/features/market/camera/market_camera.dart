import 'dart:math' as math;
import 'dart:ui';

/// Where the buyer is looking in the market. Pure maths, no widgets.
///
/// [zoom] is relative to the whole market fitting the screen (1.0).
class MarketCamera {
  const MarketCamera({
    required this.world,
    required this.viewport,
    required this.center,
    required this.zoom,
  });

  /// The market picture's size, in world units (pixels of the deepest tiles).
  final Size world;

  /// The screen area, in logical pixels.
  final Size viewport;

  /// The world point at the middle of the screen.
  final Offset center;
  final double zoom;

  /// The smallest zoom never caps below this, so the 3.4 commit stays reachable.
  static const minMaxZoom = 4.5;

  double get fitScale =>
      math.min(viewport.width / world.width, viewport.height / world.height);

  /// Logical pixels per world unit.
  double get scale => fitScale * zoom;

  Offset get _mid => viewport.center(Offset.zero);

  Offset worldToScreen(Offset w) => (w - center) * scale + _mid;

  Offset screenToWorld(Offset s) => (s - _mid) / scale + center;

  Rect get visibleWorld => Rect.fromCenter(
    center: center,
    width: viewport.width / scale,
    height: viewport.height / scale,
  );

  MarketCamera copyWith({Offset? center, double? zoom, Size? viewport}) =>
      MarketCamera(
        world: world,
        viewport: viewport ?? this.viewport,
        center: center ?? this.center,
        zoom: zoom ?? this.zoom,
      );

  /// Zoom kept in [1, maxZoom]; on each axis the view stays inside the world,
  /// or is centred on it when the view is the larger of the two.
  MarketCamera clamped({required double maxZoom}) {
    final z = zoom.clamp(1.0, math.max(1.0, maxZoom)).toDouble();
    final s = fitScale * z;
    double axis(double c, double view, double size) {
      final half = view / s / 2;
      if (half * 2 >= size) return size / 2;
      return c.clamp(half, size - half);
    }

    return copyWith(
      zoom: z,
      center: Offset(
        axis(center.dx, viewport.width, world.width),
        axis(center.dy, viewport.height, world.height),
      ),
    );
  }

  MarketCamera panBy(Offset screenDelta) =>
      copyWith(center: center - screenDelta / scale);

  /// Zooms by [factor] keeping the world point under [focalScreen] still.
  MarketCamera zoomAround(Offset focalScreen, double factor) {
    final anchor = screenToWorld(focalScreen);
    final z = zoom * factor;
    final s = fitScale * z;
    return copyWith(zoom: z, center: anchor - (focalScreen - _mid) / s);
  }

  /// The deepest zoom worth allowing: the deepest tiles shown no larger than
  /// 1.5 device pixels per tile pixel, but never below [minMaxZoom].
  static double maxZoomFor(Size world, Size viewport, double dpr) {
    final fit = math.min(
      viewport.width / world.width,
      viewport.height / world.height,
    );
    return math.max(minMaxZoom, 1.5 / (fit * dpr));
  }
}
