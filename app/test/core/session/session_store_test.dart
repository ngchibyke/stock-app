import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/session/session_store.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/fakes.dart';

void main() {
  group('the keychain item', () {
    test('it_stays_on_this_device_and_is_never_synced', () {
      // A token that roams to a new phone or a restored backup would sign
      // the next owner into the previous person's account.
      final options = SecureSessionStore.defaultStorage.iOptions;
      expect(
        options.accessibility,
        KeychainAccessibility.first_unlock_this_device,
      );
      expect(options.synchronizable, isFalse);
    });
  });

  group('a fresh install', () {
    setUp(() => SharedPreferences.setMockInitialValues({}));

    test('it_clears_a_token_left_by_an_earlier_install', () async {
      // The iOS keychain outlives an uninstall; the app's preferences do not.
      final store = MemorySessionStore('left-behind');
      final prefs = await SharedPreferences.getInstance();

      await clearSessionLeftByEarlierInstall(prefs, store);

      expect(store.token, isNull);
      expect(store.clearCount, 1);
      expect(prefs.getBool(installedFlagKey), isTrue);
    });

    test('it_keeps_the_token_on_every_later_start', () async {
      final store = MemorySessionStore('current');
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(installedFlagKey, true);

      await clearSessionLeftByEarlierInstall(prefs, store);

      expect(store.token, 'current');
      expect(store.clearCount, 0);
    });

    test('a_failed_clear_does_not_stop_the_app_and_is_tried_again', () async {
      final store = _FailingClearStore();
      final prefs = await SharedPreferences.getInstance();

      await clearSessionLeftByEarlierInstall(prefs, store);

      // The flag is only set once the clear went through.
      expect(prefs.getBool(installedFlagKey), isNull);
    });
  });
}

class _FailingClearStore implements SessionStore {
  @override
  Future<String?> read() async => 'left-behind';

  @override
  Future<void> write(String token) async {}

  @override
  Future<void> clear() async => throw Exception('keystore unavailable');
}
