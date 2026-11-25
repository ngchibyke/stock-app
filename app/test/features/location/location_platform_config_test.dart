import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// Only an approximate position is ever needed, so neither platform may
/// offer to hand over a precise one.
void main() {
  test('iOS asks for approximate location only', () {
    final plist = File('ios/Runner/Info.plist').readAsStringSync();
    expect(
      plist,
      matches(RegExp(r'<key>NSLocationDefaultAccuracyReduced</key>\s*<true/>')),
    );
  });

  test('Android asks for coarse location only', () {
    final manifest = File('android/app/src/main/AndroidManifest.xml')
        .readAsStringSync();
    expect(manifest, contains('android.permission.ACCESS_COARSE_LOCATION'));
    expect(manifest, isNot(contains('ACCESS_FINE_LOCATION')));
  });
}
