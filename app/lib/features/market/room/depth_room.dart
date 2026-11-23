import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:material_ui/material_ui.dart';

import '../perf/render_tier.dart';

/// Largest shift of the shader, as a share of the picture (spec: keep it small).
const depthStrength = 0.025;

/// The flat room's drift, as a share of the screen width.
const flatDrift = 0.01;

/// Extra zoom so a shifted edge never shows.
const roomZoom = 1.05;

/// Where a room picture of [image] size is drawn in [box]: covering it,
/// centred, 5% larger.
Rect roomImageRect(Size box, Size image) {
  final scale =
      math.max(box.width / image.width, box.height / image.height) * roomZoom;
  return Rect.fromCenter(
    center: box.center(Offset.zero),
    width: image.width * scale,
    height: image.height * scale,
  );
}

Future<ui.FragmentProgram> _loadShader() =>
    ui.FragmentProgram.fromAsset('shaders/depth_parallax.frag');

/// A store's room: its picture, shifted by its depth map as the buyer drags,
/// with [overlay] (the shelf cards) moving by the same [look].
///
/// The `full` tier uses the shader; anything else, or a shader that fails to
/// load, draws a flat picture that drifts as one layer. With reduced motion the
/// room does not move.
class DepthRoom extends StatefulWidget {
  const DepthRoom({
    super.key,
    required this.room,
    required this.depth,
    required this.tier,
    required this.overlay,
    this.loadShader = _loadShader,
  });

  final ImageProvider room;
  final ImageProvider depth;
  final RenderTier tier;

  /// Builds what sits on the room, given the view offset and where the
  /// picture is drawn.
  final Widget Function(Offset look, Rect imageRect) overlay;
  final Future<ui.FragmentProgram> Function() loadShader;

  @override
  State<DepthRoom> createState() => _DepthRoomState();
}

class _DepthRoomState extends State<DepthRoom>
    with SingleTickerProviderStateMixin {
  late final AnimationController _settle = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 400),
  )..addListener(_onSettle);
  Offset _look = Offset.zero;
  Offset _settleFrom = Offset.zero;
  double _width = 1;

  ui.FragmentShader? _shader;
  bool _shaderTried = false;
  ui.Image? _roomImage;
  ui.Image? _depthImage;
  ImageStream? _roomStream;
  ImageStream? _depthStream;
  late final ImageStreamListener _roomListener = ImageStreamListener((info, _) {
    if (mounted) setState(() => _roomImage = info.image);
  });
  late final ImageStreamListener _depthListener = ImageStreamListener((
    info,
    _,
  ) {
    if (mounted) setState(() => _depthImage = info.image);
  });

  bool get _reduceMotion => MediaQuery.disableAnimationsOf(context);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final config = createLocalImageConfiguration(context);
    _roomStream = _swap(
      _roomStream,
      widget.room.resolve(config),
      _roomListener,
    );
    if (widget.tier == RenderTier.full) {
      _depthStream = _swap(
        _depthStream,
        widget.depth.resolve(config),
        _depthListener,
      );
      _tryShader();
    }
    if (_reduceMotion) _look = Offset.zero;
  }

  ImageStream _swap(ImageStream? old, ImageStream next, ImageStreamListener l) {
    if (old?.key == next.key) return old!;
    old?.removeListener(l);
    next.addListener(l);
    return next;
  }

  void _tryShader() {
    if (_shaderTried) return;
    _shaderTried = true;
    widget.loadShader().then(
      (program) {
        if (mounted) setState(() => _shader = program.fragmentShader());
      },
      onError: (Object _) {
        // No shader on this phone: the flat room stays.
      },
    );
  }

  @override
  void dispose() {
    _roomStream?.removeListener(_roomListener);
    _depthStream?.removeListener(_depthListener);
    _settle.dispose();
    _shader?.dispose();
    super.dispose();
  }

  void _onSettle() {
    setState(
      () => _look = Offset.lerp(
        _settleFrom,
        Offset.zero,
        Curves.easeOut.transform(_settle.value),
      )!,
    );
  }

  void _onDrag(DragUpdateDetails d) {
    if (_reduceMotion) return;
    _settle.stop();
    final dx = (_look.dx + d.delta.dx / (_width / 2)).clamp(-1.0, 1.0);
    setState(() => _look = Offset(dx, 0));
  }

  void _onDragEnd(DragEndDetails _) {
    if (_reduceMotion || _look == Offset.zero) return;
    _settleFrom = _look;
    _settle.forward(from: 0);
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final size = constraints.biggest;
        _width = size.width;
        final image = _roomImage;
        final imageSize = image == null
            ? const Size(720, 1280)
            : Size(image.width.toDouble(), image.height.toDouble());
        final rect = roomImageRect(size, imageSize);
        final shader = _shader;
        final useShader =
            widget.tier == RenderTier.full &&
            shader != null &&
            image != null &&
            _depthImage != null;

        final Widget picture = useShader
            ? CustomPaint(
                key: const Key('room-shader'),
                size: size,
                painter: _DepthPainter(
                  shader: shader,
                  room: image,
                  depth: _depthImage!,
                  look: _look,
                ),
              )
            : Transform.translate(
                offset: Offset(_look.dx * flatDrift * size.width, 0),
                child: Stack(
                  children: [
                    Positioned.fromRect(
                      rect: rect,
                      child: Image(
                        key: const Key('room-flat'),
                        image: widget.room,
                        fit: BoxFit.fill,
                        gaplessPlayback: true,
                        excludeFromSemantics: true,
                      ),
                    ),
                  ],
                ),
              );

        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onHorizontalDragUpdate: _onDrag,
          onHorizontalDragEnd: _onDragEnd,
          child: Stack(
            fit: StackFit.expand,
            children: [
              ClipRect(child: picture),
              widget.overlay(_look, rect),
            ],
          ),
        );
      },
    );
  }
}

class _DepthPainter extends CustomPainter {
  _DepthPainter({
    required this.shader,
    required this.room,
    required this.depth,
    required this.look,
  });

  final ui.FragmentShader shader;
  final ui.Image room;
  final ui.Image depth;
  final Offset look;

  @override
  void paint(Canvas canvas, Size size) {
    shader
      ..setFloat(0, size.width)
      ..setFloat(1, size.height)
      ..setFloat(2, look.dx)
      ..setFloat(3, look.dy)
      ..setFloat(4, depthStrength)
      ..setFloat(5, room.width.toDouble())
      ..setFloat(6, room.height.toDouble())
      ..setImageSampler(0, room)
      ..setImageSampler(1, depth);
    canvas.drawRect(Offset.zero & size, Paint()..shader = shader);
  }

  @override
  bool shouldRepaint(_DepthPainter old) =>
      old.look != look ||
      old.room != room ||
      old.depth != depth ||
      old.shader != shader;
}
