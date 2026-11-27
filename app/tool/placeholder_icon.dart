// Draws the placeholder app icon, assets/brand/icon.png, from shapes: a dusk gradient
// and a block "M". It looks like a placeholder. The real logo replaces it. Run from app/:
//
//   dart run tool/placeholder_icon.dart
//   dart run tool/apply_brand.dart
import 'dart:io';

import 'package:image/image.dart' as img;

void main() {
  const n = 1024;
  final icon = img.Image(width: n, height: n, numChannels: 3);
  // Abuja Dusk, top to bottom.
  for (var y = 0; y < n; y++) {
    final t = y / (n - 1);
    final c = img.ColorRgb8(
      (0x2B + (0x6B - 0x2B) * t).round(),
      (0x1B + (0x2F - 0x1B) * t).round(),
      (0x4D + (0x5C - 0x4D) * t).round(),
    );
    for (var x = 0; x < n; x++) {
      icon.setPixel(x, y, c);
    }
  }
  final gold = img.ColorRgb8(0xF4, 0xB9, 0x42);
  void poly(List<List<int>> v) => img.fillPolygon(
    icon,
    vertices: [for (final p in v) img.Point(p[0], p[1])],
    color: gold,
  );
  // An "M": two posts and a V between them.
  poly([
    [250, 250],
    [370, 250],
    [370, 774],
    [250, 774],
  ]);
  poly([
    [654, 250],
    [774, 250],
    [774, 774],
    [654, 774],
  ]);
  poly([
    [250, 250],
    [370, 250],
    [512, 520],
    [512, 700],
  ]);
  poly([
    [774, 250],
    [654, 250],
    [512, 520],
    [512, 700],
  ]);
  Directory('assets/brand').createSync(recursive: true);
  File('assets/brand/icon.png').writeAsBytesSync(img.encodePng(icon));
  stdout.writeln('wrote assets/brand/icon.png');
}
