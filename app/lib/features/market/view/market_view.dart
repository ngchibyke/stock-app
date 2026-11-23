import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../camera/market_camera.dart';
import '../data/asset_resolver.dart';
import '../model/market.dart';
import 'plot_markers.dart';
import 'tile_layer.dart';

/// The market from above: drag to move, pinch to zoom, tap a glowing store.
///
/// The camera is owned by the parent: every gesture reports a new one through
/// [onCamera]. Pressing on a store asks for its room early ([onPrefetch]), so
/// it is ready when the camera arrives.
class MarketView extends ConsumerStatefulWidget {
  const MarketView({
    super.key,
    required this.snapshot,
    required this.camera,
    required this.onCamera,
    required this.onOpenStore,
    required this.onPrefetch,
  });

  final MarketSnapshot snapshot;
  final MarketCamera camera;
  final ValueChanged<MarketCamera> onCamera;
  final ValueChanged<Placement> onOpenStore;
  final ValueChanged<Placement> onPrefetch;

  @override
  ConsumerState<MarketView> createState() => _MarketViewState();
}

class _MarketViewState extends ConsumerState<MarketView> {
  late MarketCamera _current;
  double _maxZoom = MarketCamera.minMaxZoom;
  List<MarkerSpot> _spots = const [];
  String? _pressed;
  Offset _lastFocal = Offset.zero;
  double _lastScale = 1;

  void _open(MarkerSpot spot) {
    final placement = spot.placement;
    if (placement == null) return;
    HapticFeedback.selectionClick();
    widget.onOpenStore(placement);
  }

  void _onTapDown(TapDownDetails d) {
    final spot = markerAt(_spots, d.localPosition);
    if (spot?.placement == null) return;
    setState(() => _pressed = spot!.plot.id);
    widget.onPrefetch(spot!.placement!);
  }

  void _onTapUp(TapUpDetails d) {
    setState(() => _pressed = null);
    final spot = markerAt(_spots, d.localPosition);
    if (spot != null) _open(spot);
  }

  void _onScaleStart(ScaleStartDetails d) {
    _lastFocal = d.localFocalPoint;
    _lastScale = 1;
  }

  void _onScaleUpdate(ScaleUpdateDetails d) {
    var c = _current.panBy(d.localFocalPoint - _lastFocal);
    if (d.scale != _lastScale && _lastScale > 0) {
      c = c.zoomAround(d.localFocalPoint, d.scale / _lastScale);
    }
    _lastFocal = d.localFocalPoint;
    _lastScale = d.scale;
    _current = c.clamped(maxZoom: _maxZoom);
    widget.onCamera(_current);
  }

  @override
  Widget build(BuildContext context) {
    final resolver = ref.watch(assetResolverProvider);
    final layout = widget.snapshot.layout;
    return LayoutBuilder(
      builder: (context, constraints) {
        final size = constraints.biggest;
        _maxZoom = MarketCamera.maxZoomFor(
          layout.world,
          size,
          MediaQuery.devicePixelRatioOf(context),
        );
        _current = widget.camera
            .copyWith(viewport: size)
            .clamped(maxZoom: _maxZoom);
        _spots = markerSpots(widget.snapshot, _current);
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapDown: _onTapDown,
          onTapUp: _onTapUp,
          onTapCancel: () => setState(() => _pressed = null),
          onScaleStart: _onScaleStart,
          onScaleUpdate: _onScaleUpdate,
          child: ClipRect(
            child: Stack(
              children: [
                Positioned.fill(
                  child: ExcludeSemantics(
                    child: TileLayer(
                      layout: layout,
                      camera: _current,
                      resolver: resolver,
                    ),
                  ),
                ),
                Positioned.fill(
                  child: PlotMarkers(
                    spots: _spots,
                    zoom: _current.zoom,
                    pressedPlotId: _pressed,
                    onActivate: _open,
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}
