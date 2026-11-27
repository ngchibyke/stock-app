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

  test('config_parses_assets_and_ignores_http_or_missing_base_url', () {
    final good = AppConfig.fromJson({
      'assets': {'version': 'v3', 'base_url': 'https://cdn.example'},
    });
    expect(good.assetsVersion, 'v3');
    expect(good.assetsBaseUrl, 'https://cdn.example');
    final http = AppConfig.fromJson({
      'assets': {'version': 'v3', 'base_url': 'http://cdn.example'},
    });
    expect(http.assetsBaseUrl, isNull);
    final odd = AppConfig.fromJson({
      'assets': {'version': '../x', 'base_url': 'https://cdn.example'},
    });
    expect(odd.assetsVersion, isNull);
    expect(AppConfig.fromJson({'assets': null}).assetsBaseUrl, isNull);
    final back = AppConfig.fromJson(good.toJson());
    expect(back.assetsVersion, 'v3');
    expect(good.withoutMaintenance().assetsBaseUrl, 'https://cdn.example');
  });

  test('a link that is not https is ignored', () {
    final config = AppConfig.fromJson({
      'terms_url': 'javascript:alert(1)',
      'privacy_url': 'http://example.com/privacy',
    });
    expect(config.termsUrl, isNull);
    expect(config.privacyUrl, isNull);
  });

  test('the_store_pages_are_read_kept_and_only_when_https', () {
    final config = AppConfig.fromJson({
      'store_url': {
        'android': 'https://play.google.com/store/apps/details?id=x',
        'ios': 'itms-apps://apps.apple.com/app/id1',
      },
    });
    expect(
      config.storeUrlAndroid,
      'https://play.google.com/store/apps/details?id=x',
    );
    expect(config.storeUrlIos, isNull, reason: 'not https');
    // They survive being stored, and a failed fetch.
    final back = AppConfig.fromJson(config.toJson());
    expect(back.storeUrlAndroid, config.storeUrlAndroid);
    expect(config.withoutMaintenance().storeUrlAndroid, config.storeUrlAndroid);
    // An older server that does not send them, or sends nonsense.
    expect(AppConfig.fromJson({}).storeUrlAndroid, isNull);
    expect(AppConfig.fromJson({'store_url': 'x'}).storeUrlIos, isNull);
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

  for (final (label, sent, expected) in <(String, Object?, String?)>[
    ('a message', 'Back at 6pm', 'Back at 6pm'),
    ('a null message', null, null),
    ('an empty message', '', null),
  ]) {
    test(
      'maintenance active with $label parses, is stored and reloads',
      () async {
        final (repo, adapter) = await _build();
        adapter.reply(200, {
          ..._body,
          'maintenance': {'active': true, 'message': sent},
        });

        final parsed = await repo.load();
        expect(parsed.maintenanceActive, isTrue);
        expect(parsed.maintenanceMessage, expected);

        final (relaunched, _) = await _build();
        final stored = relaunched.lastGood!;
        expect(stored.maintenanceActive, isTrue);
        expect(stored.maintenanceMessage, expected);
      },
    );
  }

  test('a failed fetch never carries a stored maintenance flag', () async {
    final (repo, adapter) = await _build();
    adapter.reply(200, {
      ..._body,
      'maintenance': {'active': true, 'message': 'Back at 6pm'},
    });
    await repo.load();

    adapter.failWithConnectionError();
    final fallback = await repo.load();
    expect(fallback.maintenanceActive, isFalse);
    expect(fallback.maintenanceMessage, isNull);
    // The stored minimum stands.
    expect(fallback.minVersionAndroid, '2.0.0');
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
