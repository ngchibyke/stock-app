// The app's name, ids and icons live in brand.json. This applies them to every platform
// file, and checks that nothing has drifted. Run from app/:
//
//   dart run tool/apply_brand.dart           # write the files
//   dart run tool/apply_brand.dart --check   # fail if any file differs (CI)
import 'dart:convert';
import 'dart:io';

import 'package:image/image.dart' as img;

class Brand {
  const Brand({
    required this.displayName,
    required this.applicationId,
    required this.bundleId,
    required this.icon,
  });

  factory Brand.read(String root) {
    final json = jsonDecode(
      File('$root/brand.json').readAsStringSync(),
    ) as Map<String, dynamic>;
    final b = Brand(
      displayName: '${json['display_name']}',
      applicationId: '${json['application_id']}',
      bundleId: '${json['bundle_id']}',
      icon: '${json['icon']}',
    );
    b.validate();
    return b;
  }

  final String displayName;
  final String applicationId;
  final String bundleId;
  final String icon;

  /// Throws a [FormatException] saying what is wrong.
  void validate() {
    if (displayName.trim().isEmpty || displayName.length > 30) {
      throw const FormatException('display_name must be 1 to 30 characters');
    }
    if (RegExp(r'[<>&"\\]').hasMatch(displayName)) {
      throw const FormatException('display_name has a character not allowed');
    }
    if (!RegExp(r'^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$')
        .hasMatch(applicationId)) {
      throw const FormatException(
        'application_id must look like ng.yourbrand.app: lowercase letters, '
        'digits and _, at least two parts',
      );
    }
    if (!RegExp(r'^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$').hasMatch(bundleId)) {
      throw const FormatException(
        'bundle_id must look like ng.yourbrand.app: letters, digits and -',
      );
    }
    if (icon.contains('..') || icon.startsWith('/')) {
      throw const FormatException('icon must be a path inside the app folder');
    }
  }
}

const _androidSizes = {
  'mdpi': 48,
  'hdpi': 72,
  'xhdpi': 96,
  'xxhdpi': 144,
  'xxxhdpi': 192,
};

String _xml(String s) =>
    s.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

/// Replaces the one match of [pattern] in [text]; fails if there is not exactly one
/// (or, with [many], at least one), so a changed platform file is noticed, not skipped.
String _edit(
  String file,
  String text,
  RegExp pattern,
  String Function(Match) to, {
  bool many = false,
}) {
  final matches = pattern.allMatches(text).length;
  if (matches == 0 || (!many && matches != 1)) {
    throw StateError(
      '$file: expected ${many ? 'at least one' : 'one'} place to change, found '
      '$matches. The file has changed shape; update tool/brand.dart.',
    );
  }
  return text.replaceAllMapped(pattern, to);
}

/// What every managed file should contain, by path relative to [root].
Map<String, List<int>> render(String root, Brand b) {
  final out = <String, List<int>>{};
  String read(String rel) => File('$root/$rel').readAsStringSync();

  const gradle = 'android/app/build.gradle.kts';
  out[gradle] = utf8.encode(
    _edit(
      gradle,
      read(gradle),
      RegExp(r'applicationId = "[^"]*"'),
      (_) => 'applicationId = "${b.applicationId}"',
    ),
  );
  const manifest = 'android/app/src/main/AndroidManifest.xml';
  out[manifest] = utf8.encode(
    _edit(
      manifest,
      read(manifest),
      RegExp(r'android:label="[^"]*"'),
      (_) => 'android:label="${_xml(b.displayName)}"',
    ),
  );
  const plist = 'ios/Runner/Info.plist';
  out[plist] = utf8.encode(
    _edit(
      plist,
      read(plist),
      RegExp(r'(<key>CFBundleDisplayName</key>\s*<string>)[^<]*(</string>)'),
      (m) => '${m[1]}${_xml(b.displayName)}${m[2]}',
    ),
  );
  const pbx = 'ios/Runner.xcodeproj/project.pbxproj';
  out[pbx] = utf8.encode(
    _edit(
      pbx,
      read(pbx),
      RegExp(r'PRODUCT_BUNDLE_IDENTIFIER = ([^;]*);'),
      (m) => m[1]!.endsWith('.RunnerTests')
          ? 'PRODUCT_BUNDLE_IDENTIFIER = ${b.bundleId}.RunnerTests;'
          : 'PRODUCT_BUNDLE_IDENTIFIER = ${b.bundleId};',
      many: true,
    ),
  );

  final src = img.decodeImage(File('$root/${b.icon}').readAsBytesSync());
  if (src == null || src.width != src.height || src.width < 1024) {
    throw StateError('${b.icon} must be a square picture, at least 1024 px');
  }
  List<int> size(img.Image from, int px, {bool opaque = false}) {
    var r = img.copyResize(
      from,
      width: px,
      height: px,
      interpolation: img.Interpolation.cubic,
    );
    if (opaque) {
      // The App Store refuses an icon with transparency.
      final flat = img.Image(width: px, height: px, numChannels: 3);
      img.fill(flat, color: img.ColorRgb8(255, 255, 255));
      img.compositeImage(flat, r);
      r = flat;
    }
    return img.encodePng(r);
  }

  for (final e in _androidSizes.entries) {
    out['android/app/src/main/res/mipmap-${e.key}/ic_launcher.png'] = size(
      src,
      e.value,
    );
  }
  const iconDir = 'ios/Runner/Assets.xcassets/AppIcon.appiconset';
  final contents =
      jsonDecode(read('$iconDir/Contents.json')) as Map<String, dynamic>;
  for (final e in (contents['images'] as List).cast<Map<String, dynamic>>()) {
    final name = e['filename'] as String?;
    if (name == null) continue;
    final m = RegExp(r'^Icon-App-([\d.]+)x[\d.]+@(\d)x\.png$').firstMatch(name);
    if (m == null) {
      throw StateError('$iconDir/Contents.json: unexpected icon name $name');
    }
    final px = (double.parse(m[1]!) * int.parse(m[2]!)).round();
    out['$iconDir/$name'] = size(src, px, opaque: true);
  }
  return out;
}

/// Paths whose content is not what [render] says.
List<String> drifted(String root, Brand b) {
  final want = render(root, b);
  final bad = <String>[];
  want.forEach((rel, bytes) {
    final f = File('$root/$rel');
    if (!f.existsSync()) {
      bad.add(rel);
      return;
    }
    final have = f.readAsBytesSync();
    if (have.length != bytes.length) {
      bad.add(rel);
      return;
    }
    for (var i = 0; i < have.length; i++) {
      if (have[i] != bytes[i]) {
        bad.add(rel);
        return;
      }
    }
  });
  return bad..sort();
}

/// Writes every file that differs; returns their paths.
List<String> apply(String root, Brand b) {
  final want = render(root, b);
  final changed = drifted(root, b);
  for (final rel in changed) {
    File('$root/$rel')
      ..parent.createSync(recursive: true)
      ..writeAsBytesSync(want[rel]!);
  }
  return changed;
}
