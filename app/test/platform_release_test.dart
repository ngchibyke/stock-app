import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// Guards for the platform files this part changed, so a later edit cannot quietly undo
/// them. The Gradle and Xcode builds themselves are proved by CI, not here.
void main() {
  String read(String path) => File(path).readAsStringSync();

  test('a_release_is_never_signed_with_the_debug_key', () {
    final gradle = read('android/app/build.gradle.kts');
    expect(gradle, isNot(contains('signingConfigs.getByName("debug")')));
    expect(gradle, contains('signingConfigs.getByName("release")'));
    // A release task without the upload key stops, naming what is missing.
    expect(gradle, contains('gradle.taskGraph.whenReady'));
    expect(
      gradle,
      contains('Release builds are never signed with the debug key'),
    );
    for (final name in [
      'ANDROID_KEYSTORE_PATH',
      'ANDROID_KEYSTORE_PASSWORD',
      'ANDROID_KEY_ALIAS',
      'ANDROID_KEY_PASSWORD',
    ]) {
      expect(gradle, contains(name));
    }
  });

  test('keys_and_firebase_files_are_ignored_by_git', () {
    final android = read('android/.gitignore');
    for (final p in [
      'key.properties',
      '**/*.keystore',
      '**/*.jks',
      'app/google-services.json',
    ]) {
      expect(android, contains(p));
    }
    expect(read('.gitignore'), contains('ios/Runner/GoogleService-Info.plist'));
  });

  test(
    'android_can_show_notifications_and_still_has_the_other_permissions',
    () {
      final m = read('android/app/src/main/AndroidManifest.xml');
      for (final p in [
        'INTERNET',
        'CAMERA',
        'ACCESS_COARSE_LOCATION',
        'POST_NOTIFICATIONS',
      ]) {
        expect(m, contains('android.permission.$p'));
      }
      // Rough location only, never precise.
      expect(m, isNot(contains('ACCESS_FINE_LOCATION')));
    },
  );

  test('ios_declares_push_and_standard_encryption_only', () {
    final plist = read('ios/Runner/Info.plist');
    expect(
      plist,
      matches(
        RegExp(
          r'UIBackgroundModes</key>\s*<array>\s*<string>remote-notification</string>',
        ),
      ),
    );
    expect(
      plist,
      matches(RegExp(r'ITSAppUsesNonExemptEncryption</key>\s*<false/>')),
    );
    // The release push entitlement is a separate file, passed only to a release archive.
    final ent = read('ios/Runner/Release.entitlements');
    expect(ent, contains('<key>aps-environment</key>'));
    expect(ent, contains('<string>production</string>'));
    expect(
      read('ios/Runner.xcodeproj/project.pbxproj'),
      isNot(contains('Release.entitlements')),
    );
  });
}
