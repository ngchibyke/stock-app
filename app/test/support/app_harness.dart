import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart' show Override;
import 'package:market/app.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/config/app_gate.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/session/session_store.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_auth_repository.dart';
import 'fakes.dart';

const testMe = {
  'id': 'u1',
  'display_name': 'Ada',
  'phone': '+2348031234567',
  'role': 'buyer',
  'is_seller': false,
};

/// A config provider that never touches the network or storage: it answers
/// [initial], lets a test change it, and counts foreground refreshes.
class FakeConfigNotifier extends AppConfigNotifier {
  FakeConfigNotifier([this.initial = AppConfig.empty]);

  final AppConfig initial;
  int refreshCount = 0;

  @override
  Future<AppConfig> build() async => initial;

  @override
  Future<void> refresh() async => refreshCount++;

  void set(AppConfig config) => state = AsyncData(config);
}

/// A store that holds its first read until [open] is called, so a test can look
/// at the app while the session is still restoring.
class GatedSessionStore extends MemorySessionStore {
  GatedSessionStore([super.token]);

  final Completer<void> _gate = Completer<void>();

  void open() => _gate.complete();

  @override
  Future<String?> read() async {
    await _gate.future;
    return token;
  }
}

/// Everything a test of the whole app needs: a fake network, a session store,
/// a fake config and a fake auth repository.
class AppHarness {
  AppHarness({SessionStore? store, AppConfig config = AppConfig.empty})
    : store = store ?? MemorySessionStore(),
      config = FakeConfigNotifier(config);

  final FakeAdapter adapter = FakeAdapter();
  final SessionStore store;
  final FakeConfigNotifier config;
  final FakeAuthRepository repo = FakeAuthRepository();
  String version = '1.0.0';

  /// For [MarketRoot], which adds the session hooks itself.
  List<Override> get overrides => [
    sessionStoreProvider.overrideWithValue(store),
    apiClientProvider.overrideWith(
      (ref) => ApiClient(
        baseUrl: 'https://api.test',
        readToken: () => ref.read(accessTokenReaderProvider)(),
        onUnauthorized: () => ref.read(unauthorizedHandlerProvider)(),
        adapter: adapter,
      ),
    ),
    authRepositoryProvider.overrideWithValue(repo),
    appConfigProvider.overrideWith(() => config),
    packageVersionProvider.overrideWith((ref) async => version),
    appPlatformProvider.overrideWithValue('android'),
  ];

  /// For a page or a container built by hand: the same plus the session hooks.
  List<Override> get pageOverrides => [...sessionOverrides, ...overrides];

  Future<void> pump(
    WidgetTester tester, {
    List<Override> extra = const [],
    double textScale = 1.0,
  }) async {
    tester.view.physicalSize = const Size(400, 1000);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MarketRoot(overrides: [...overrides, ...extra]));
    await settle(tester);
  }

  ProviderContainer container(WidgetTester tester) =>
      ProviderScope.containerOf(tester.element(find.byType(MarketApp)));
}

/// Lets chained futures finish. Fixed pumps, because the splash animates for
/// ever and would never "settle".
Future<void> settle(WidgetTester tester, {int frames = 6}) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}
