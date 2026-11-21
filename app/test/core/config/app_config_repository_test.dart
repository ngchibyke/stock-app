import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/config/app_config.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/fakes.dart';

const _body = {
  'min_version': {'android': '2.0.0', 'ios': '3.1.0'},
  'terms_url': 'https://example.com/terms',
  'privacy_url': 'https://example.com/privacy',
  'support_email': 'help@example.com',
  'maintenance': {'active': false, 'message': null},
};

Future<(AppConfigRepository, FakeAdapter)> _build() async {
  final adapter = FakeAdapter();
  final client = ApiClient(
    baseUrl: 'https://api.test',
    readToken: () async => 'secret-token',
    onUnauthorized: () {},
    adapter: adapter,
  );
  final prefs = await SharedPreferences.getInstance();
  return (AppConfigRepository(client, prefs), adapter);
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test(
    'parses the server answer and calls the public endpoint without auth',
    () async {
      final (repo, adapter) = await _build();
      adapter.reply(200, _body);

      final config = await repo.load();

      expect(config.minVersionAndroid, '2.0.0');
      expect(config.minVersionIos, '3.1.0');
      expect(config.termsUrl, 'https://example.com/terms');
      expect(config.privacyUrl, 'https://example.com/privacy');
      expect(config.supportEmail, 'help@example.com');
      expect(config.maintenanceActive, isFalse);
      expect(adapter.requests.single.path, '/app/config');
      expect(
        adapter.requests.single.headers.containsKey('Authorization'),
        isFalse,
      );
    },
  );

  test('every value may be null, empty or odd without crashing', () {
    expect(AppConfig.fromJson({}).maintenanceActive, isFalse);
    final odd = AppConfig.fromJson({
      'min_version': 'nope',
      'terms_url': 5,
      'privacy_url': '   ',
      'support_email': null,
      'maintenance': {'active': 'yes', 'message': 7},
    });
    expect(odd.minVersionAndroid, isNull);
    expect(odd.termsUrl, isNull);
    expect(odd.privacyUrl, isNull);
    expect(odd.maintenanceActive, isFalse);
    expect(odd.maintenanceMessage, isNull);
    expect(
      AppConfig.fromJson({'maintenance': null}).maintenanceActive,
      isFalse,
    );
  });

  test('a link that is not https is ignored', () {
    final config = AppConfig.fromJson({
      'terms_url': 'javascript:alert(1)',
      'privacy_url': 'http://example.com/privacy',
    });
    expect(config.termsUrl, isNull);
    expect(config.privacyUrl, isNull);
  });

  test('a good answer is stored and a fresh answer replaces it', () async {
    final (repo, adapter) = await _build();
    expect(repo.lastGood, isNull);

    adapter.reply(200, _body);
    await repo.load();
    expect(repo.lastGood?.minVersionAndroid, '2.0.0');

    adapter.reply(200, {
      ..._body,
      'min_version': {'android': '2.5.0', 'ios': null},
    });
    final fresh = await repo.load();

    expect(fresh.minVersionAndroid, '2.5.0');
    expect(repo.lastGood?.minVersionAndroid, '2.5.0');
    expect(repo.lastGood?.minVersionIos, isNull);
  });

  test('the stored copy survives a new repository (a new launch)', () async {
    final (repo, adapter) = await _build();
    adapter.reply(200, _body);
    await repo.load();

    final (relaunched, _) = await _build();
    expect(relaunched.lastGood?.minVersionIos, '3.1.0');
  });

  test('a corrupt stored copy is ignored', () async {
    SharedPreferences.setMockInitialValues({'app_config.v1': '{not json'});
    final (repo, _) = await _build();
    expect(repo.lastGood, isNull);
  });

  test('failures return the last good config, else empty', () async {
    final (repo, adapter) = await _build();

    adapter.failWithConnectionError();
    final first = await repo.load();
    expect(first.minVersionAndroid, isNull);
    expect(first.maintenanceActive, isFalse);

    adapter.reply(200, _body);
    await repo.load();

    adapter.failWithTimeout();
    expect((await repo.load()).minVersionAndroid, '2.0.0');
    adapter.reply(503, {
      'error': {'code': 'unavailable', 'message': 'down'},
    });
    expect((await repo.load()).minVersionAndroid, '2.0.0');
    adapter.reply(200, '<html>captive portal</html>', 'text/html');
    expect((await repo.load()).minVersionAndroid, '2.0.0');
    adapter.reply(200, [1, 2, 3]);
    expect((await repo.load()).minVersionAndroid, '2.0.0');
    adapter.reply(200);
    expect((await repo.load()).minVersionAndroid, '2.0.0');
  });
}
