import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/config/app_gate.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/fakes.dart';

AppConfig _config({
  String? android,
  String? ios,
  bool maintenance = false,
  String? message,
}) => AppConfig(
  minVersionAndroid: android,
  minVersionIos: ios,
  maintenanceActive: maintenance,
  maintenanceMessage: message,
);

Map<String, Object?> _answer({
  String? android,
  String? ios,
  bool maintenance = false,
  String? message,
}) => {
  'min_version': {'android': android, 'ios': ios},
  'terms_url': null,
  'privacy_url': null,
  'support_email': null,
  'maintenance': {'active': maintenance, 'message': message},
};

ProviderContainer _container(
  FakeAdapter adapter, {
  String version = '1.0.0',
  String platform = 'android',
}) {
  final client = ApiClient(
    baseUrl: 'https://api.test',
    readToken: () async => null,
    onUnauthorized: () {},
    adapter: adapter,
  );
  final container = ProviderContainer(
    overrides: [
      apiClientProvider.overrideWithValue(client),
      packageVersionProvider.overrideWith((ref) async => version),
      appPlatformProvider.overrideWithValue(platform),
    ],
  );
  addTearDown(container.dispose);
  return container;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('compare_versions_orders_numerically', () {
    expect(compareVersions('1.10.0', '1.9.0'), greaterThan(0));
    expect(compareVersions('1.9.0', '1.10.0'), lessThan(0));
    expect(compareVersions('1.0.0', '1.0.0'), 0);
    expect(compareVersions('2.0.0', '1.99.99'), greaterThan(0));
    expect(compareVersions('1.0.1', '1.0.0'), greaterThan(0));
  });

  test('a_version_below_the_minimum_requires_an_update', () {
    final config = _config(android: '2.0.0', ios: '3.0.0');
    expect(
      evaluateGate(config, platform: 'android', version: '1.9.9'),
      Gate.updateRequired,
    );
    expect(
      evaluateGate(config, platform: 'android', version: '2.0.0'),
      Gate.open,
    );
    // iOS reads its own field: 2.5.0 passes android's 2.0.0 but not ios's 3.0.0.
    expect(
      evaluateGate(config, platform: 'ios', version: '2.5.0'),
      Gate.updateRequired,
    );
    expect(
      evaluateGate(
        _config(android: '9.0.0', ios: '1.0.0'),
        platform: 'ios',
        version: '1.0.0',
      ),
      Gate.open,
    );
  });

  test('a_null_minimum_never_blocks', () {
    expect(
      evaluateGate(AppConfig.empty, platform: 'android', version: '0.0.1'),
      Gate.open,
    );
    expect(
      evaluateGate(
        _config(android: '5.0.0'),
        platform: 'ios',
        version: '0.0.1',
      ),
      Gate.open,
    );
    expect(
      evaluateGate(
        _config(android: ''),
        platform: 'android',
        version: '1.0.0',
      ),
      Gate.open,
    );
  });

  test('maintenance_wins_over_everything_and_carries_its_message', () async {
    // A real server body goes through the repository's parse; the minimum
    // would block this version, yet maintenance wins.
    final adapter = FakeAdapter()
      ..reply(
        200,
        _answer(android: '9.0.0', maintenance: true, message: 'Back at 6pm'),
      );
    final container = _container(adapter);
    final config = await container.read(appConfigProvider.future);
    await container.read(packageVersionProvider.future);

    expect(config.maintenanceActive, isTrue);
    expect(config.maintenanceMessage, 'Back at 6pm');
    expect(container.read(gateProvider), Gate.maintenance);
  });

  test('cached maintenance on then a failed fetch gives Gate.open', () async {
    final first = FakeAdapter()
      ..reply(200, _answer(maintenance: true, message: 'Back at 6pm'));
    final c1 = _container(first);
    await c1.read(appConfigProvider.future);
    await c1.read(packageVersionProvider.future);
    expect(c1.read(gateProvider), Gate.maintenance);

    // A new launch with the server unreachable: the stored copy says
    // maintenance, but a failure never carries it.
    for (final fail in <void Function(FakeAdapter)>[
      (a) => a.failWithConnectionError(),
      (a) => a.failWithTimeout(),
      (a) => a.reply(503, {
        'error': {'code': 'unavailable', 'message': 'x'},
      }),
    ]) {
      final adapter = FakeAdapter();
      fail(adapter);
      final c2 = _container(adapter);
      final config = await c2.read(appConfigProvider.future);
      await c2.read(packageVersionProvider.future);
      expect(config.maintenanceActive, isFalse);
      expect(config.maintenanceMessage, isNull);
      expect(c2.read(gateProvider), Gate.open);
    }
  });

  test(
    'cached maintenance on then a fresh answer that says active still blocks',
    () async {
      final first = FakeAdapter()
        ..reply(200, _answer(maintenance: true, message: 'old'));
      final c1 = _container(first);
      await c1.read(appConfigProvider.future);

      final second = FakeAdapter()
        ..reply(200, _answer(maintenance: true, message: 'Back at 6pm'));
      final c2 = _container(second);
      final config = await c2.read(appConfigProvider.future);
      await c2.read(packageVersionProvider.future);

      expect(config.maintenanceMessage, 'Back at 6pm');
      expect(c2.read(gateProvider), Gate.maintenance);
    },
  );

  test(
    'cached update-required plus a failed fetch still requires an update',
    () async {
      final first = FakeAdapter()..reply(200, _answer(android: '2.0.0'));
      final c1 = _container(first);
      await c1.read(appConfigProvider.future);

      final second = FakeAdapter()..failWithConnectionError();
      final c2 = _container(second);
      await c2.read(appConfigProvider.future);
      await c2.read(packageVersionProvider.future);
      expect(c2.read(gateProvider), Gate.updateRequired);
    },
  );

  test(
    'a failed refresh after a maintenance answer does not keep maintenance',
    () async {
      final adapter = FakeAdapter()
        ..reply(200, _answer(maintenance: true, message: 'm'));
      final container = _container(adapter);
      await container.read(appConfigProvider.future);
      await container.read(packageVersionProvider.future);
      expect(container.read(gateProvider), Gate.maintenance);

      adapter.failWithTimeout();
      await container.read(appConfigProvider.notifier).refresh();
      expect(container.read(gateProvider), Gate.open);
    },
  );

  test('a_malformed_version_in_the_config_does_not_block', () {
    for (final bad in ['abc', '1.2', '1.2.3.4', '1.x.0', '-1.0.0', ' ']) {
      expect(
        evaluateGate(
          _config(android: bad),
          platform: 'android',
          version: '1.0.0',
        ),
        Gate.open,
        reason: bad,
      );
    }
  });

  test('the running apps own malformed version means open', () {
    final config = _config(android: '2.0.0');
    for (final bad in ['', 'dev', '1.0']) {
      expect(
        evaluateGate(config, platform: 'android', version: bad),
        Gate.open,
        reason: bad,
      );
    }
  });

  test('an unknown platform means open', () {
    expect(
      evaluateGate(
        _config(android: '9.0.0', ios: '9.0.0'),
        platform: 'web',
        version: '1.0.0',
      ),
      Gate.open,
    );
  });

  test(
    'an_unreachable_server_fails_open_with_the_last_good_config_or_empty',
    () async {
      final adapter = FakeAdapter()..failWithConnectionError();
      final container = _container(adapter);

      // Nothing stored yet: empty config, gate open.
      expect(await container.read(appConfigProvider.future), AppConfig.empty);
      expect(container.read(gateProvider), Gate.open);

      // A good answer that blocks this version, then a failure: the last good
      // answer still stands (it is a real answer, not a failure), and a failure
      // alone never produces a block.
      adapter.reply(200, _answer(android: '2.0.0'));
      await container.read(appConfigProvider.notifier).refresh();
      expect(container.read(gateProvider), Gate.updateRequired);

      adapter.failWithTimeout();
      await container.read(appConfigProvider.notifier).refresh();
      expect(
        container.read(appConfigProvider).requireValue.minVersionAndroid,
        '2.0.0',
      );

      // Fresh container, nothing stored, every kind of failure: always open.
      SharedPreferences.setMockInitialValues({});
      for (final queue in <void Function(FakeAdapter)>[
        (a) => a.failWithTimeout(),
        (a) => a.reply(503, {
          'error': {'code': 'unavailable', 'message': 'x'},
        }),
        (a) => a.reply(200, 'not json', 'text/html'),
      ]) {
        final a = FakeAdapter();
        queue(a);
        final c = _container(a);
        await c.read(appConfigProvider.future);
        expect(c.read(gateProvider), Gate.open);
      }
    },
  );

  test('the gate is open while the config and version are still loading', () {
    final container = _container(FakeAdapter()..reply(200, _answer()));
    expect(container.read(gateProvider), Gate.open);
  });

  test('the gate reads platform and version from their providers', () async {
    final adapter = FakeAdapter()
      ..reply(200, _answer(android: '9.0.0', ios: '1.0.0'));
    final container = _container(adapter, platform: 'ios', version: '1.2.0');
    await container.read(appConfigProvider.future);
    await container.read(packageVersionProvider.future);
    expect(container.read(gateProvider), Gate.open);
  });

  test('a failing version lookup leaves the gate open', () async {
    final adapter = FakeAdapter()..reply(200, _answer(android: '9.0.0'));
    final client = ApiClient(
      baseUrl: 'https://api.test',
      readToken: () async => null,
      onUnauthorized: () {},
      adapter: adapter,
    );
    final container = ProviderContainer(
      overrides: [
        apiClientProvider.overrideWithValue(client),
        packageVersionProvider.overrideWith(
          (ref) async => throw StateError('no platform channel'),
        ),
        appPlatformProvider.overrideWithValue('android'),
      ],
    );
    addTearDown(container.dispose);
    await container.read(appConfigProvider.future);
    await Future<void>.delayed(Duration.zero);
    expect(container.read(gateProvider), Gate.open);
  });

  test(
    'returning to the foreground fetches again and replaces the answer',
    () async {
      final adapter = FakeAdapter()..reply(200, _answer(android: '1.5.0'));
      final container = _container(adapter, version: '1.0.0');
      await container.read(appConfigProvider.future);
      await container.read(packageVersionProvider.future);
      expect(container.read(gateProvider), Gate.updateRequired);

      final refresher = container.read(appForegroundRefresherProvider);
      adapter.reply(200, _answer());

      // Going to the background does not fetch.
      refresher.didChangeAppLifecycleState(AppLifecycleState.paused);
      expect(adapter.requests, hasLength(1));

      refresher.didChangeAppLifecycleState(AppLifecycleState.resumed);
      // The refresh runs in the background; wait until it has landed.
      for (var i = 0; i < 100; i++) {
        if (container.read(appConfigProvider).value?.minVersionAndroid ==
            null) {
          break;
        }
        await Future<void>.delayed(const Duration(milliseconds: 10));
      }

      expect(adapter.requests, hasLength(2));
      expect(
        container.read(appConfigProvider).requireValue.minVersionAndroid,
        isNull,
      );
      expect(container.read(gateProvider), Gate.open);
    },
  );

  test('the foreground trigger can be injected and is detached on dispose', () {
    var calls = 0;
    final refresher = AppForegroundRefresher(() => calls++);
    refresher.didChangeAppLifecycleState(AppLifecycleState.resumed);
    refresher.didChangeAppLifecycleState(AppLifecycleState.inactive);
    refresher.didChangeAppLifecycleState(AppLifecycleState.paused);
    expect(calls, 1);
    refresher.attach();
    refresher.detach();
  });
}
