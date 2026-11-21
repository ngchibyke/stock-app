import 'dart:async';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';

import '../../support/fakes.dart';

void main() {
  late FakeAdapter adapter;
  late int unauthorizedCalls;
  String? token;

  ApiClient build({String baseUrl = 'https://api.example.test'}) => ApiClient(
    baseUrl: baseUrl,
    readToken: () async => token,
    onUnauthorized: () => unauthorizedCalls++,
    adapter: adapter,
  );

  setUp(() {
    adapter = FakeAdapter();
    unauthorizedCalls = 0;
    token = 'tok-123';
  });

  test('sends_the_bearer_header_only_for_authenticated_calls', () async {
    final client = build();
    adapter.reply(200, {'ok': true});
    adapter.reply(200, {'ok': true});
    await client.get('/a');
    await client.get('/b', auth: false);
    expect(adapter.requests[0].headers['Authorization'], 'Bearer tok-123');
    expect(adapter.requests[0].headers['Accept'], 'application/json');
    expect(adapter.requests[1].headers.containsKey('Authorization'), isFalse);
  });

  test('error_envelope_becomes_an_api_error_with_code_and_message', () async {
    final client = build();
    adapter.reply(409, {
      'error': {'code': 'conflict', 'message': 'Already exists'},
    });
    await expectLater(
      client.post('/x', body: {'a': 1}),
      throwsA(
        isA<ApiError>()
            .having((e) => e.status, 'status', 409)
            .having((e) => e.code, 'code', 'conflict')
            .having((e) => e.message, 'message', 'Already exists')
            .having((e) => e.retryAfterSecs, 'retryAfterSecs', isNull),
      ),
    );
  });

  test('retry_after_secs_is_read_from_the_error_object', () async {
    final client = build();
    adapter.reply(429, {
      'error': {
        'code': 'rate_limited',
        'message': 'Slow down',
        'retry_after_secs': 42,
      },
    });
    await expectLater(
      client.get('/x'),
      throwsA(isA<ApiError>().having((e) => e.retryAfterSecs, 'secs', 42)),
    );
  });

  test('verification_id_is_read_from_the_top_level_of_a_403', () async {
    final client = build();
    adapter.reply(403, {
      'error': {'code': 'phone_unverified', 'message': 'Verify your phone'},
      'verification_id': 'ver-9',
    });
    await expectLater(
      client.get('/x'),
      throwsA(
        isA<ApiError>()
            .having((e) => e.code, 'code', 'phone_unverified')
            .having((e) => e.verificationId, 'verificationId', 'ver-9'),
      ),
    );
  });

  test('a_401_on_an_authenticated_call_signs_out_once', () async {
    final client = build();
    adapter.reply(401, {
      'error': {'code': 'unauthorized', 'message': 'Sign in'},
    });
    adapter.reply(401, {
      'error': {'code': 'unauthorized', 'message': 'Sign in'},
    });
    final results = await Future.wait([
      client.get('/a').then<Object?>((_) => null).catchError((Object e) => e),
      client.get('/b').then<Object?>((_) => null).catchError((Object e) => e),
    ]);
    expect(results, everyElement(isA<ApiError>()));
    expect(unauthorizedCalls, 1);
  });

  test('a_401_on_an_unauthenticated_call_does_not_sign_out', () async {
    final client = build();
    adapter.reply(401, {
      'error': {'code': 'invalid_credentials', 'message': 'Wrong password'},
    });
    await expectLater(
      client.post('/auth/login', body: {}, auth: false),
      throwsA(isA<ApiError>().having((e) => e.status, 'status', 401)),
    );
    expect(unauthorizedCalls, 0);
  });

  test('a_204_is_null', () async {
    final client = build();
    adapter.reply(204);
    expect(await client.delete('/x'), isNull);
  });

  test(
    'no_connection_is_an_offline_error_and_a_timeout_is_a_timeout_error',
    () async {
      final client = build();
      adapter.failWithConnectionError();
      adapter.failWithTimeout();
      await expectLater(
        client.get('/x'),
        throwsA(
          isA<ApiError>()
              .having((e) => e.isOffline, 'isOffline', isTrue)
              .having((e) => e.code, 'code', 'offline')
              .having((e) => e.status, 'status', 0),
        ),
      );
      await expectLater(
        client.get('/x'),
        throwsA(
          isA<ApiError>()
              .having((e) => e.isTimeout, 'isTimeout', isTrue)
              .having((e) => e.code, 'code', 'timeout'),
        ),
      );
    },
  );

  test('a_non_json_error_body_still_gives_an_api_error', () async {
    final client = build();
    adapter.reply(502, '<html>Bad Gateway</html>');
    await expectLater(
      client.get('/x'),
      throwsA(
        isA<ApiError>()
            .having((e) => e.status, 'status', 502)
            .having((e) => e.isServerDown, 'isServerDown', isTrue)
            .having((e) => e.message, 'message', isNotEmpty),
      ),
    );
  });

  test('requests_time_out_after_15_seconds', () {
    final client = build();
    expect(client.dio.options.connectTimeout, const Duration(seconds: 15));
    expect(client.dio.options.receiveTimeout, const Duration(seconds: 15));
    expect(client.dio.options.followRedirects, isFalse);
  });

  test(
    'a_non_https_base_url_is_refused_outside_debug_builds_and_localhost_hosts',
    () {
      ApiClient make(String url, {required bool debug}) => ApiClient(
        baseUrl: url,
        readToken: () async => null,
        onUnauthorized: () {},
        adapter: adapter,
        debugMode: debug,
      );
      // https is always fine.
      expect(
        () => make('https://api.example.test', debug: false),
        returnsNormally,
      );
      // Plain http for the dev hosts, debug only.
      for (final host in ['10.0.2.2', 'localhost', '127.0.0.1']) {
        expect(() => make('http://$host:8080', debug: true), returnsNormally);
        expect(
          () => make('http://$host:8080', debug: false),
          throwsArgumentError,
        );
      }
      // Plain http to any other host is refused even in debug.
      expect(
        () => make('http://api.example.test', debug: true),
        throwsArgumentError,
      );
      expect(
        () => make('http://192.168.1.5:8080', debug: true),
        throwsArgumentError,
      );
      // Other schemes and junk are refused.
      expect(() => make('ftp://localhost', debug: true), throwsArgumentError);
      expect(() => make('not a url', debug: true), throwsArgumentError);
    },
  );

  test('no_request_or_response_content_is_logged', () async {
    final client = build();
    expect(client.dio.interceptors.whereType<LogInterceptor>(), isEmpty);
    final printed = <String>[];
    adapter.reply(200, {'secret': 'body'});
    await runZoned(
      () => client.post('/x', body: {'password': 'hunter2'}),
      zoneSpecification: ZoneSpecification(
        print: (self, parent, zone, line) => printed.add(line),
      ),
    );
    expect(printed, isEmpty);
  });
}
