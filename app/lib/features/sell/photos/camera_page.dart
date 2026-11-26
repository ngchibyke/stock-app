import 'dart:async';

import 'package:camera/camera.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../../../core/theme/tokens.dart';
import '../../../core/ui/pressable.dart';
import '../sell_strings.dart';

typedef CameraFactory = Future<CameraController> Function();

Future<CameraController> _defaultFactory() async {
  final cameras = await availableCameras();
  final back = cameras.firstWhere(
    (c) => c.lensDirection == CameraLensDirection.back,
    orElse: () => cameras.first,
  );
  final controller = CameraController(
    back,
    ResolutionPreset.high,
    enableAudio: false,
    imageFormatGroup: ImageFormatGroup.jpeg,
  );
  await controller.initialize();
  return controller;
}

/// A square frame with thirds, drawn over the live picture.
class FramePainter extends CustomPainter {
  const FramePainter(this.color);

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final side = size.width < size.height ? size.width : size.height;
    final rect = Rect.fromCenter(
      center: size.center(Offset.zero),
      width: side,
      height: side,
    );
    final line = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2;
    canvas.drawRect(rect, line);
    final thin = Paint()
      ..color = color.withValues(alpha: 0.4)
      ..strokeWidth = 1;
    for (var i = 1; i < 3; i++) {
      final dx = rect.left + rect.width * i / 3;
      final dy = rect.top + rect.height * i / 3;
      canvas.drawLine(Offset(dx, rect.top), Offset(dx, rect.bottom), thin);
      canvas.drawLine(Offset(rect.left, dy), Offset(rect.right, dy), thin);
    }
  }

  @override
  bool shouldRepaint(FramePainter old) => old.color != color;
}

/// The guided camera: a square frame, a tip for the shot, tap to focus, flash and
/// a shutter. Pops the picture's bytes. A denied camera shows a plain message
/// and lets the person go back to the gallery.
class GuidedCameraPage extends StatefulWidget {
  const GuidedCameraPage({
    super.key,
    required this.shotIndex,
    this.factory = _defaultFactory,
  });

  final int shotIndex;
  final CameraFactory factory;

  @override
  State<GuidedCameraPage> createState() => _GuidedCameraPageState();
}

class _GuidedCameraPageState extends State<GuidedCameraPage> {
  CameraController? _controller;
  bool _denied = false;
  bool _flash = false;
  bool _taking = false;

  @override
  void initState() {
    super.initState();
    unawaited(_start());
  }

  Future<void> _start() async {
    try {
      final c = await widget.factory();
      if (!mounted) {
        await c.dispose();
        return;
      }
      setState(() => _controller = c);
    } on Object {
      if (mounted) setState(() => _denied = true);
    }
  }

  @override
  void dispose() {
    unawaited(_controller?.dispose());
    super.dispose();
  }

  Future<void> _shoot() async {
    final c = _controller;
    if (c == null || _taking) return;
    setState(() => _taking = true);
    try {
      final file = await c.takePicture();
      final Uint8List bytes = await file.readAsBytes();
      if (mounted) Navigator.of(context).pop(bytes);
    } on Object {
      if (mounted) setState(() => _taking = false);
    }
  }

  Future<void> _toggleFlash() async {
    final c = _controller;
    if (c == null) return;
    final next = !_flash;
    await c.setFlashMode(next ? FlashMode.auto : FlashMode.off);
    if (mounted) setState(() => _flash = next);
  }

  Future<void> _focus(TapDownDetails d, Size size) async {
    final c = _controller;
    if (c == null) return;
    final p = Offset(
      d.localPosition.dx / size.width,
      d.localPosition.dy / size.height,
    );
    try {
      await c.setFocusPoint(p);
      await c.setExposurePoint(p);
    } on Object {
      // Not every camera focuses on a point.
    }
  }

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    const black = Color(0xFF000000);
    const white = Color(0xFFFFFFFF);
    final tip =
        SellStrings.shotTips[widget.shotIndex % SellStrings.shotTips.length];
    return Scaffold(
      backgroundColor: black,
      appBar: AppBar(
        backgroundColor: black,
        foregroundColor: white,
        surfaceTintColor: black,
        actions: [
          if (_controller != null)
            IconButton(
              tooltip: SellStrings.flash,
              onPressed: _toggleFlash,
              icon: Icon(_flash ? Icons.flash_auto : Icons.flash_off),
            ),
        ],
      ),
      body: SafeArea(
        child: _denied
            ? Padding(
                padding: const EdgeInsets.all(Tokens.s5),
                child: Center(
                  child: Text(
                    SellStrings.cameraDenied,
                    style: Theme.of(context).textTheme.bodyLarge
                        ?.copyWith(color: white),
                    textAlign: TextAlign.center,
                  ),
                ),
              )
            : _controller == null
            ? const Center(child: CircularProgressIndicator())
            : Column(
                children: [
                  Expanded(
                    child: LayoutBuilder(
                      builder: (context, box) => GestureDetector(
                        onTapDown: (d) => _focus(d, box.biggest),
                        child: Stack(
                          fit: StackFit.expand,
                          children: [
                            ClipRect(child: CameraPreview(_controller!)),
                            const CustomPaint(painter: FramePainter(white)),
                            Align(
                              alignment: Alignment.topCenter,
                              child: Padding(
                                padding: const EdgeInsets.all(Tokens.s4),
                                child: DecoratedBox(
                                  decoration: BoxDecoration(
                                    color: black.withValues(alpha: 0.55),
                                    borderRadius: BorderRadius.circular(
                                      Tokens.radius,
                                    ),
                                  ),
                                  child: Padding(
                                    padding: const EdgeInsets.symmetric(
                                      horizontal: Tokens.s4,
                                      vertical: Tokens.s2,
                                    ),
                                    child: Text(
                                      tip,
                                      style: Theme.of(context)
                                          .textTheme
                                          .titleMedium
                                          ?.copyWith(color: white),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                  Padding(
                    padding: const EdgeInsets.all(Tokens.s5),
                    child: Pressable(
                      haptic: true,
                      semanticLabel: SellStrings.shutter,
                      radius: 40,
                      onTap: _taking ? null : _shoot,
                      child: SizedBox(
                        width: 72,
                        height: 72,
                        child: DecoratedBox(
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: _taking ? tokens.mute : white,
                            border: Border.all(color: tokens.accent, width: 4),
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
      ),
    );
  }
}

/// The tips shown the first time, and again on request.
Future<void> showPhotoGuide(BuildContext context) => showModalBottomSheet<void>(
  context: context,
  isScrollControlled: true,
  showDragHandle: true,
  builder: (context) => SafeArea(
    child: Padding(
      padding: const EdgeInsets.fromLTRB(Tokens.s5, 0, Tokens.s5, Tokens.s5),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            SellStrings.guideTitle,
            style: Theme.of(context).textTheme.titleLarge,
          ),
          const SizedBox(height: Tokens.s3),
          for (final tip in SellStrings.guideTips) ...[
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Icon(Icons.check_circle_outline, size: 20),
                const SizedBox(width: Tokens.s2),
                Expanded(
                  child: Text(
                    tip,
                    style: Theme.of(context).textTheme.bodyLarge,
                  ),
                ),
              ],
            ),
            const SizedBox(height: Tokens.s2),
          ],
          const SizedBox(height: Tokens.s3),
          SizedBox(
            width: double.infinity,
            child: FilledButton(
              onPressed: () {
                HapticFeedback.selectionClick();
                Navigator.of(context).pop();
              },
              child: const Text(SellStrings.guideGotIt),
            ),
          ),
        ],
      ),
    ),
  ),
);
