// Generates the placeholder market art in assets/market/. Run from app/:
//
//   dart run tool/placeholder_art.dart
//
// Deterministic (seeded). It is placeholder art and looks like it: flat roofs,
// roads and rooms drawn from shapes. The real art replaces it (plan 7f).
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:image/image.dart' as img;

const world = 2048;
const tileSize = 256;
const maxLevel = 3;
const out = 'assets/market';

class District {
  const District(
    this.id,
    this.name,
    this.x,
    this.y,
    this.w,
    this.h,
    this.cols,
    this.rows,
    this.ground,
  );
  final String id, name;
  final int x, y, w, h, cols, rows;
  final int ground;
}

// 60 plots: 16 / 12 / 12 / 12 / 8.
const districts = [
  District('fashion', 'Fashion', 64, 64, 920, 900, 4, 4, 0xD9B58C),
  District(
    'phones-electronics',
    'Phones and electronics',
    1064,
    64,
    920,
    660,
    4,
    3,
    0xC9B79A,
  ),
  District(
    'home-beauty',
    'Home and beauty',
    64,
    1044,
    920,
    660,
    4,
    3,
    0xD4AE92,
  ),
  District(
    'food-groceries',
    'Food and groceries',
    1064,
    804,
    920,
    660,
    4,
    3,
    0xC7B07C,
  ),
  District(
    'everything-else',
    'Everything else',
    1064,
    1544,
    920,
    440,
    4,
    2,
    0xCDB9A0,
  ),
];

const roofs = [0x8C4A2F, 0x6E7B7F, 0xA65E3A, 0x4F6D6A, 0x9A8F84, 0xB5734A];
const umbrellas = [0xE2B13C, 0xC4452D, 0x2F6E8E, 0x3E8A55, 0xE7E2D6];

img.ColorRgb8 c(int hex) =>
    img.ColorRgb8((hex >> 16) & 255, (hex >> 8) & 255, hex & 255);

void main() {
  final rnd = Random(7);
  Directory(out).createSync(recursive: true);
  final plots = <Map<String, Object>>[];
  final market = img.Image(width: world, height: world);

  // Ground, then the roads between districts.
  img.fill(market, color: c(0xBFA98C));
  for (var i = 0; i < 9000; i++) {
    final x = rnd.nextInt(world), y = rnd.nextInt(world);
    img.fillCircle(
      market,
      x: x,
      y: y,
      radius: 1 + rnd.nextInt(3),
      color: c(rnd.nextBool() ? 0xB39C7E : 0xC8B396),
    );
  }
  for (final d in districts) {
    img.fillRect(
      market,
      x1: d.x - 16,
      y1: d.y - 16,
      x2: d.x + d.w + 16,
      y2: d.y + d.h + 16,
      color: c(0x56544F),
    );
  }
  for (final d in districts) {
    img.fillRect(
      market,
      x1: d.x,
      y1: d.y,
      x2: d.x + d.w,
      y2: d.y + d.h,
      color: c(d.ground),
    );
    final cw = d.w ~/ d.cols, ch = d.h ~/ d.rows;
    for (var row = 0; row < d.rows; row++) {
      for (var col = 0; col < d.cols; col++) {
        final px = d.x + col * cw + 24, py = d.y + row * ch + 24;
        final pw = cw - 48, ph = ch - 48;
        plots.add({
          'id':
              '${d.id}-${plots.where((p) => p['district'] == d.id).length + 1}',
          'district': d.id,
          'x': px,
          'y': py,
          'w': pw,
          'h': ph,
        });
        // Shadow, roof, corrugation, a few umbrellas in front.
        img.fillRect(
          market,
          x1: px + 8,
          y1: py + 10,
          x2: px + pw + 8,
          y2: py + ph + 10,
          color: c(0x3C342C),
        );
        final roof = roofs[rnd.nextInt(roofs.length)];
        img.fillRect(
          market,
          x1: px,
          y1: py,
          x2: px + pw,
          y2: py + ph,
          color: c(roof),
        );
        for (var s = py; s < py + ph; s += 6) {
          img.drawLine(
            market,
            x1: px,
            y1: s,
            x2: px + pw,
            y2: s,
            color: c(_shade(roof, 0.86)),
          );
        }
        for (var u = 0; u < 3; u++) {
          img.fillCircle(
            market,
            x: px + 18 + rnd.nextInt(max(1, pw - 36)),
            y: py + ph + 16,
            radius: 9,
            color: c(umbrellas[rnd.nextInt(umbrellas.length)]),
          );
        }
      }
    }
  }
  // Road markings.
  for (var y = 0; y < world; y += 40) {
    img.fillRect(
      market,
      x1: 1020,
      y1: y,
      x2: 1028,
      y2: y + 20,
      color: c(0xD8D2C2),
    );
  }

  // The tile pyramid: level 3 is native, lower levels are downscaled.
  for (var level = 0; level <= maxLevel; level++) {
    final n = 1 << level;
    final scaled = img.copyResize(
      market,
      width: tileSize * n,
      height: tileSize * n,
      interpolation: img.Interpolation.average,
    );
    final dir = Directory('$out/tiles/$level')..createSync(recursive: true);
    for (var x = 0; x < n; x++) {
      for (var y = 0; y < n; y++) {
        final tile = img.copyCrop(
          scaled,
          x: x * tileSize,
          y: y * tileSize,
          width: tileSize,
          height: tileSize,
        );
        File('${dir.path}/${x}_$y.jpg')
            .writeAsBytesSync(img.encodeJpg(tile, quality: 70));
      }
    }
  }
  File('$out/base.jpg').writeAsBytesSync(
    img.encodeJpg(img.copyResize(market, width: 256, height: 256), quality: 70),
  );

  File('$out/layout.json').writeAsStringSync(
    const JsonEncoder.withIndent('  ').convert({
      'version': 'placeholder-1',
      'world': {'w': world, 'h': world},
      'tile_size': tileSize,
      'max_level': maxLevel,
      'base_image': 'base.jpg',
      'districts': [
        for (final d in districts) {'id': d.id, 'name': d.name},
      ],
      'plots': plots,
    }),
  );

  _room(
    'boutique',
    wall: 0xE9D6C0,
    floor: 0x9C6B47,
    shelf: 0x6B4A33,
    tint: '#FFE3C2',
    shadow: 0.35,
  );
  _room(
    'phone_shop',
    wall: 0xDCE3EA,
    floor: 0x8E9399,
    shelf: 0xF4F6F8,
    tint: '#E6EEFF',
    shadow: 0.3,
  );
  _room(
    'grocery',
    wall: 0xEFE4C8,
    floor: 0xA88A5E,
    shelf: 0x7C6A4E,
    tint: '#FFF1CC',
    shadow: 0.3,
  );
  stdout.writeln('placeholder art written to $out');
}

int _shade(int hex, double f) {
  int ch(int s) => (((hex >> s) & 255) * f).round().clamp(0, 255);
  return (ch(16) << 16) | (ch(8) << 8) | ch(0);
}

// Three shelf rows of three slots; the side columns turn toward the middle.
const _rows = [(0.26, 0.35), (0.45, 0.5), (0.64, 0.7)];
const _cols = [(0.08, 0.15), (0.38, 0.0), (0.68, -0.15)];

void _room(
  String id, {
  required int wall,
  required int floor,
  required int shelf,
  required String tint,
  required double shadow,
}) {
  const w = 720, h = 1280;
  final room = img.Image(width: w, height: h);
  img.fill(room, color: c(wall));
  img.fillRect(
    room,
    x1: 0,
    y1: (h * 0.82).round(),
    x2: w,
    y2: h,
    color: c(floor),
  );
  img.fillRect(
    room,
    x1: 0,
    y1: (h * 0.82).round() - 6,
    x2: w,
    y2: (h * 0.82).round(),
    color: c(_shade(floor, 0.7)),
  );

  // Depth: 0 far (dark) .. 255 near (light). Wall far, floor ramps nearer.
  const dw = 90, dh = 160;
  final depth = img.Image(width: dw, height: dh, numChannels: 1);
  for (var y = 0; y < dh; y++) {
    final v = y < dh * 0.82
        ? 60
        : (60 + (y - dh * 0.82) / (dh * 0.18) * 195).round();
    for (var x = 0; x < dw; x++) {
      depth.setPixelRgb(x, y, v, v, v);
    }
  }

  final slots = <Map<String, Object>>[];
  var n = 0;
  for (final (top, d) in _rows) {
    final shelfY = ((top + 0.15) * h).round();
    img.fillRect(
      room,
      x1: 30,
      y1: shelfY,
      x2: w - 30,
      y2: shelfY + 14,
      color: c(shelf),
    );
    img.fillRect(
      room,
      x1: 30,
      y1: shelfY + 14,
      x2: w - 30,
      y2: shelfY + 22,
      color: c(_shade(shelf, 0.6)),
    );
    for (final (left, tilt) in _cols) {
      n++;
      slots.add({
        'id': 's$n',
        'x': left,
        'y': top,
        'w': 0.24,
        'h': 0.14,
        'depth': d,
        'tilt_y': tilt,
      });
      img.fillRect(
        room,
        x1: (left * w).round(),
        y1: (top * h).round(),
        x2: ((left + 0.24) * w).round(),
        y2: shelfY,
        color: c(_shade(wall, 0.94)),
      );
      final v = (60 + d * 160).round();
      img.fillRect(
        depth,
        x1: (left * dw).round(),
        y1: (top * dh).round(),
        x2: ((left + 0.24) * dw).round(),
        y2: ((top + 0.16) * dh).round(),
        color: img.ColorRgb8(v, v, v),
      );
    }
  }

  final dir = Directory('$out/rooms/$id')..createSync(recursive: true);
  File('${dir.path}/room.jpg')
      .writeAsBytesSync(img.encodeJpg(room, quality: 72));
  File('${dir.path}/depth.png').writeAsBytesSync(img.encodePng(depth));
  File('${dir.path}/room.json').writeAsStringSync(
    const JsonEncoder.withIndent('  ').convert({
      'id': id,
      'room_image': 'room.jpg',
      'depth_image': 'depth.png',
      'light': {'tint': tint, 'shadow_opacity': shadow},
      'slots': slots,
    }),
  );
}
