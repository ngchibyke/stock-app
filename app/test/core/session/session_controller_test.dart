import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/session/session_store.dart';

import '../../support/fakes.dart';

const _me = {
  'id': 'u1',
  'display_name': 'Ada',
  'phone': '+2348012345678',
  'role': 'buyer',
  'is_seller': false,
};

const _unauthorized = {
  'error': {'code': 'unauthorized', 'message': 'Session expired.'},
};

const _token = 'secret-token-value-123';

ProviderContainer _container(SessionStore store, FakeAdapter adapter) {
  final container = ProviderContainer(
    overrides: [
      sessionStoreProvider.overrideWithValue(store),
      ...sessionOverrides,
      apiClientProvider.overrideWith(
        (ref) => ApiClient(
          baseUrl: 'https://api.test',
          readToken: () => ref.read(accessTokenReaderProvider)(),
          onUnauthorized: () => ref.read(unauthorizedHandlerProvider)(),
          adapter: adapter,
        ),
      ),
    ],
  );
  addTearDown(container.dispose);
  return container;
}

void main() {
  test('restores_a_signed_in_session_from_a_stored_token', () async {
    final adapter = FakeAdapter()..reply(200, _me);
    final store = MemorySessionStore(_token);
    final c = _container(store, adapter);

    expect(c.read(sessionProvider).isLoading, isTrue);
    final state = await c.read(sessionProvider.future);

    expect(state, isA<SessionSignedIn>());
    expect((state as SessionSignedIn).me.displayName, 'Ada');
    expect(state.me.isSeller, isFalse);
    expect(adapter.requests.single.path, '/me');
    expect(adapter.requests.single.headers['Authorization'], 'Bearer $_token');
  });

  test('no_stored_token_means_signed_out', () async {
    final adapter = FakeAdapter();
    final c = _container(MemorySessionStore(), adapter);

    expect(await c.read(sessionProvider.future), isA<SessionSignedOut>());
    expect(adapter.requests, isEmpty);
  });

  test(
    'a_corrupt_or_unreadable_store_means_signed_out_and_is_cleared',
    () async {
      final storage = ThrowingReadStorage(failDelete: true);
      final store = SecureSessionStore(storage);

      expect(await store.read(), isNull);
      expect(storage.deleted, ['session.token']);

      final adapter = FakeAdapter();
      final c = _container(store, adapter);
      expect(await c.read(sessionProvider.future), isA<SessionSignedOut>());
      expect(adapter.requests, isEmpty);
    },
  );

  test(
    'a_stored_token_the_server_rejects_means_signed_out_and_clears_it',
    () async {
      final adapter = FakeAdapter()..reply(401, _unauthorized);
      final store = MemorySessionStore(_token);
      final c = _container(store, adapter);

      expect(await c.read(sessionProvider.future), isA<SessionSignedOut>());
      expect(store.token, isNull);
    },
  );

  test('an_offline_failure_at_start_does_not_clear_the_stored_token', () async {
    final adapter = FakeAdapter()..failWithConnectionError();
    final store = MemorySessionStore(_token);
    final c = _container(store, adapter);

    await expectLater(c.read(sessionProvider.future), throwsA(isA<ApiError>()));
    expect(c.read(sessionProvider).hasError, isTrue);
    expect(store.token, _token);
    expect(store.clearCount, 0);

    // A retry (invalidate) restores the session once the network is back.
    adapter.reply(200, _me);
    c.invalidate(sessionProvider);
    expect(await c.read(sessionProvider.future), isA<SessionSignedIn>());
    expect(store.token, _token);
  });

  test('a_server_error_at_start_does_not_clear_the_stored_token', () async {
    final adapter = FakeAdapter()
      ..reply(503, {
        'error': {'code': 'unavailable', 'message': 'Try later.'},
      });
    final store = MemorySessionStore(_token);
    final c = _container(store, adapter);

    await expectLater(c.read(sessionProvider.future), throwsA(isA<ApiError>()));
    expect(store.token, _token);
  });

  test('sign_in_with_token_stores_it_and_loads_me', () async {
    final adapter = FakeAdapter()..reply(200, _me);
    final store = MemorySessionStore();
    final c = _container(store, adapter);
    await c.read(sessionProvider.future);

    await c.read(sessionProvider.notifier).signInWithToken(_token);

    expect(store.token, _token);
    final state = c.read(sessionProvider).requireValue;
    expect(state, isA<SessionSignedIn>());
    expect(adapter.requests.single.headers['Authorization'], 'Bearer $_token');
  });

  test('a_failed_me_call_after_sign_in_clears_the_token', () async {
    final adapter = FakeAdapter()..failWithConnectionError();
    final store = MemorySessionStore();
    final c = _container(store, adapter);
    await c.read(sessionProvider.future);

    await expectLater(
      c.read(sessionProvider.notifier).signInWithToken(_token),
      throwsA(isA<ApiError>()),
    );

    expect(store.token, isNull);
    expect(c.read(sessionProvider).requireValue, isA<SessionSignedOut>());
  });

  test('sign_out_clears_even_when_the_server_call_fails', () async {
    final adapter = FakeAdapter()
      ..reply(200, _me)
      ..failWithConnectionError();
    final store = MemorySessionStore(_token);
    final c = _container(store, adapter);
    await c.read(sessionProvider.future);

    await c.read(sessionProvider.notifier).signOut();

    expect(adapter.requests.last.method, 'POST');
    expect(adapter.requests.last.path, '/auth/sign-out');
    expect(adapter.requests.last.headers['Authorization'], 'Bearer $_token');
    expect(store.token, isNull);
    expect(c.read(sessionProvider).requireValue, isA<SessionSignedOut>());
  });

  test('an_unauthorized_response_from_any_call_signs_out_once', () async {
    final adapter = FakeAdapter()
      ..reply(200, _me)
      ..reply(401, _unauthorized)
      ..reply(401, _unauthorized);
    final store = MemorySessionStore(_token);
    final c = _container(store, adapter);
    await c.read(sessionProvider.future);
    final api = c.read(apiClientProvider);

    await expectLater(api.get('/orders'), throwsA(isA<ApiError>()));
    await expectLater(api.get('/orders'), throwsA(isA<ApiError>()));

    expect(c.read(sessionProvider).requireValue, isA<SessionSignedOut>());
    expect(store.clearCount, 1);
    // handleUnauthorized never calls the server.
    expect(adapter.requests.any((r) => r.path == '/auth/sign-out'), isFalse);
  });

  test('the_token_is_never_written_anywhere_but_the_store', () async {
    final logs = <String>[];
    final original = debugPrint;
    debugPrint = (String? m, {int? wrapWidth}) => logs.add(m ?? '');
    addTearDown(() => debugPrint = original);

    final adapter = FakeAdapter()
      ..reply(200, _me)
      ..reply(204);
    final store = MemorySessionStore();
    final c = _container(store, adapter);

    await runZoned(
      () async {
        await c.read(sessionProvider.future);
        await c.read(sessionProvider.notifier).signInWithToken(_token);
        await c.read(sessionProvider.notifier).signOut();
      },
      zoneSpecification: ZoneSpecification(
        print: (self, parent, zone, line) => logs.add(line),
      ),
    );

    for (final r in adapter.requests) {
      expect(r.path, isNot(contains(_token)));
      expect(r.body.toString(), isNot(contains(_token)));
    }
    expect(logs.join('\n'), isNot(contains(_token)));
    expect(store.writeCount, 1);
  });

  test('a_forgotten_override_fails_loudly', () {
    final c = ProviderContainer();
    addTearDown(c.dispose);
    expect(() => c.read(accessTokenReaderProvider), throwsA(anything));
    expect(() => c.read(unauthorizedHandlerProvider), throwsA(anything));
  });
}
