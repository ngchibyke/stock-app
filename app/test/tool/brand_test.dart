import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;

import '../../tool/brand.dart';

const _files = [
  'brand.json',
  'assets/brand/icon.png',
  'android/app/build.gradle.kts',
  'android/app/src/main/AndroidManifest.xml',
  'ios/Runner/Info.plist',
  'ios/Runner.xcodeproj/project.pbxproj',
  'ios/Runner/Assets.xcassets/AppIcon.appiconset/Contents.json',
];

/// A copy of the files the tool manages, so a test can change them freely.
Directory _project() {
  final dir = Directory.systemTemp.createTempSync('brand_test');
  addTearDown(() => dir.deleteSync(recursive: true));
  for (final rel in _files) {
    final to = File('${dir.path}/$rel')..parent.createSync(recursive: true);
    File(rel).copySync(to.path);
  }
  return dir;
}

Brand _brand({
  String name = 'Soko',
  String id = 'ng.soko.app',
  String bundle = 'ng.soko.app',
}) => Brand(
  displayName: name,
  applicationId: id,
  bundleId: bundle,
  icon: 'assets/brand/icon.png',
);

String _read(Directory d, String rel) =>
    File('${d.path}/$rel').readAsStringSync();

void main() {
  test('the_real_project_matches_brand_json', () {
    // The same check CI runs: nothing in the platform files has drifted.
    expect(drifted('.', Brand.read('.')), isEmpty);
  });

  test('it_changes_every_place_the_name_and_ids_live', () {
    final d = _project();
    final changed = apply(d.path, _brand(name: 'Soko & Co'));
    expect(changed, contains('android/app/build.gradle.kts'));
    expect(
      _read(d, 'android/app/build.gradle.kts'),
      contains('applicationId = "ng.soko.app"'),
    );
    // The namespace is not the application id: it stays, so the Kotlin package does.
    expect(
      _read(d, 'android/app/build.gradle.kts'),
      contains('namespace = "ng.market.market"'),
    );
    expect(
      _read(d, 'android/app/src/main/AndroidManifest.xml'),
      contains('android:label="Soko &amp; Co"'),
    );
    expect(
      _read(d, 'ios/Runner/Info.plist'),
      matches(
        RegExp(r'CFBundleDisplayName</key>\s*<string>Soko &amp; Co</string>'),
      ),
    );
    final pbx = _read(d, 'ios/Runner.xcodeproj/project.pbxproj');
    expect(pbx, contains('PRODUCT_BUNDLE_IDENTIFIER = ng.soko.app;'));
    expect(
      pbx,
      contains('PRODUCT_BUNDLE_IDENTIFIER = ng.soko.app.RunnerTests;'),
    );
    expect(pbx, isNot(contains('ng.market.market')));
  });

  test('it_is_idempotent_and_check_notices_a_drifted_file', () {
    final d = _project();
    expect(apply(d.path, _brand()), isNotEmpty);
    expect(
      apply(d.path, _brand()),
      isEmpty,
      reason: 'a second run changes nothing',
    );
    expect(drifted(d.path, _brand()), isEmpty);
    // Someone edits the label by hand.
    final manifest = File('${d.path}/android/app/src/main/AndroidManifest.xml');
    manifest.writeAsStringSync(
      manifest.readAsStringSync().replaceFirst(
        'android:label="Soko"',
        'android:label="Other"',
      ),
    );
    expect(drifted(d.path, _brand()), [
      'android/app/src/main/AndroidManifest.xml',
    ]);
    // And a hand-edited icon.
    File('${d.path}/android/app/src/main/res/mipmap-hdpi/ic_launcher.png')
        .writeAsBytesSync([1, 2, 3]);
    expect(
      drifted(d.path, _brand()),
      contains('android/app/src/main/res/mipmap-hdpi/ic_launcher.png'),
    );
  });

  test('icons_come_in_every_size_and_the_ios_ones_have_no_transparency', () {
    final d = _project();
    apply(d.path, _brand());
    const android = {
      'mdpi': 48,
      'hdpi': 72,
      'xhdpi': 96,
      'xxhdpi': 144,
      'xxxhdpi': 192,
    };
    for (final e in android.entries) {
      final i = img.decodePng(
        File(
          '${d.path}/android/app/src/main/res/mipmap-${e.key}/ic_launcher.png',
        ).readAsBytesSync(),
      )!;
      expect((i.width, i.height), (e.value, e.value), reason: e.key);
    }
    const dir = 'ios/Runner/Assets.xcassets/AppIcon.appiconset';
    final contents =
        jsonDecode(_read(d, '$dir/Contents.json')) as Map<String, dynamic>;
    var count = 0;
    for (final e in (contents['images'] as List).cast<Map<String, dynamic>>()) {
      final name = e['filename'] as String;
      final i = img.decodePng(File('${d.path}/$dir/$name').readAsBytesSync())!;
      final m = RegExp(r'Icon-App-([\d.]+)x[\d.]+@(\d)x').firstMatch(name)!;
      final px = (double.parse(m[1]!) * int.parse(m[2]!)).round();
      expect(i.width, px, reason: name);
      expect(i.numChannels, 3, reason: '$name must have no alpha');
      count++;
    }
    expect(count, greaterThanOrEqualTo(15));
  });

  test('a_bad_brand_is_refused_with_a_reason', () {
    for (final (b, word) in [
      (_brand(name: ''), 'display_name'),
      (_brand(name: 'Has <tag>'), 'display_name'),
      (_brand(id: 'NG.Soko'), 'application_id'),
      (_brand(id: 'single'), 'application_id'),
      (_brand(bundle: 'bad id'), 'bundle_id'),
    ]) {
      expect(
        b.validate,
        throwsA(
          isA<FormatException>().having(
            (e) => e.message,
            'message',
            contains(word),
          ),
        ),
      );
    }
    expect(
      () => const Brand(
        displayName: 'X',
        applicationId: 'a.b',
        bundleId: 'a.b',
        icon: '../x.png',
      ).validate(),
      throwsA(isA<FormatException>()),
    );
  });

  test('a_platform_file_that_changed_shape_is_noticed_not_skipped', () {
    final d = _project();
    final gradle = File('${d.path}/android/app/build.gradle.kts');
    gradle.writeAsStringSync(
      gradle.readAsStringSync().replaceAll('applicationId', 'appId'),
    );
    expect(() => apply(d.path, _brand()), throwsA(isA<StateError>()));
  });

  test('a_small_or_missing_icon_is_refused', () {
    final d = _project();
    File('${d.path}/assets/brand/icon.png')
        .writeAsBytesSync(img.encodePng(img.Image(width: 100, height: 100)));
    expect(() => apply(d.path, _brand()), throwsA(isA<StateError>()));
  });
}
