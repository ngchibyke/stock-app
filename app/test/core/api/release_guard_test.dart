import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/release_guard.dart';

void main() {
  test('a_release_build_accepts_only_an_https_production_address', () {
    for (final ok in [
      'https://api.example.ng',
      'https://api.example.ng/',
      'https://api.example.ng:8443/v1',
      'https://172.32.0.1',
    ]) {
      expect(
        () => validateApiBaseUrl(ok, release: true),
        returnsNormally,
        reason: ok,
      );
    }
  });

  test('a_release_build_refuses_missing_plain_and_development_addresses', () {
    for (final bad in [
      '',
      '   ',
      'http://api.example.ng',
      'api.example.ng',
      'ftp://api.example.ng',
      'https://',
      'https://user:pw@api.example.ng',
      'http://10.0.2.2:8080',
      'https://localhost',
      'https://127.0.0.1:8080',
      'https://192.168.1.20',
      'https://10.1.2.3',
      'https://172.20.0.5',
      'https://printer.local',
    ]) {
      expect(
        () => validateApiBaseUrl(bad, release: true),
        throwsA(isA<StateError>()),
        reason: '"$bad"',
      );
    }
  });

  test('the_message_names_the_setting_and_never_echoes_the_address', () {
    try {
      validateApiBaseUrl('https://user:secret@api.example.ng', release: true);
      fail('expected an error');
    } on StateError catch (e) {
      expect(e.message, contains('API_BASE_URL'));
      expect(e.message, isNot(contains('secret')));
    }
  });

  test('debug_and_profile_builds_are_not_checked', () {
    for (final any in ['', 'http://10.0.2.2:8080', 'nonsense']) {
      expect(() => validateApiBaseUrl(any, release: false), returnsNormally);
    }
  });
}
