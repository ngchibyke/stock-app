import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market_app/core/api/api_client.dart';
import 'package:market_app/core/api/api_error.dart';
import 'package:market_app/core/api/session.dart';
import 'package:market_app/core/money.dart';

/// Answers every request from a function, and remembers what it was asked.
class FakeAdapter implements HttpClientAdapter {
  FakeAdapter(this.reply);
  final (int, Object?) Function(RequestOptions) reply;
  final List<RequestOptions> seen = [];
  Object? failWith;

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    seen.add(options);
    if (failWith != null) throw failWith!;
    final (status, body) = reply(options);
    return ResponseBody.fromString(
      body == null ? '' : (body is String ? body : jsonEncode(body)),
      status,
      headers: {
        Headers.contentTypeHeader: ['application/json'],
      },
    );
  }

  @override
  void close({bool force = false}) {}
}

ApiClient clientFor(
  FakeAdapter adapter,
  SessionStore session, {
  void Function()? onUnauthorized,
}) {
  final dio = Dio()..httpClientAdapter = adapter;
  return ApiClient(
    baseUrl: 'http://api.test',
    session: session,
    dio: dio,
    onUnauthorized: onUnauthorized,
  );
}

void main() {
  group('ApiClient', () {
    test('sends the bearer token and a JSON body', () async {
      final adapter = FakeAdapter((_) => (200, {'ok': true}));
      final c = clientFor(adapter, MemorySessionStore('tok'));
      await c.post('/cart/items', body: {'a': 1});
      final o = adapter.seen.single;
      expect(o.headers['Authorization'], 'Bearer tok');
      expect(o.headers['Content-Type'], 'application/json');
      expect(o.uri.toString(), 'http://api.test/cart/items');
    });

    test('sends no token for an unauthenticated call', () async {
      final adapter = FakeAdapter((_) => (200, {}));
      final c = clientFor(adapter, MemorySessionStore('tok'));
      await c.post('/auth/sign-in', body: {}, auth: false);
      expect(adapter.seen.single.headers.containsKey('Authorization'), isFalse);
    });

    test('leaves empty query values out', () async {
      final adapter = FakeAdapter((_) => (200, []));
      final c = clientFor(adapter, MemorySessionStore());
      await c.get('/products', query: {'q': '', 'category': null, 'limit': 20});
      expect(adapter.seen.single.uri.query, 'limit=20');
    });

    test(
      'turns the error envelope into an ApiError and keeps extra fields',
      () async {
        final adapter = FakeAdapter(
          (_) => (
            403,
            {
              'error': {
                'code': 'phone_unverified',
                'message': 'Verify your phone number to continue',
              },
              'verification_id': 'v-1',
            },
          ),
        );
        final c = clientFor(adapter, MemorySessionStore());
        final e = await c
            .post('/auth/sign-in', auth: false)
            .then<ApiError?>((_) => null)
            .catchError((Object e) => e as ApiError);
        expect(e!.status, 403);
        expect(e.code, 'phone_unverified');
        expect(e.message, 'Verify your phone number to continue');
        expect(e.extra['verification_id'], 'v-1');
      },
    );

    test('falls back when the body is not JSON', () async {
      final adapter = FakeAdapter((_) => (502, '<html>'));
      final c = clientFor(adapter, MemorySessionStore());
      final e = await c
          .get('/me')
          .then<ApiError?>((_) => null)
          .catchError((Object e) => e as ApiError);
      expect(e, isA<ApiError>());
      expect(e!.status, 502);
      expect(e.code, 'unknown');
    });

    test(
      'a 401 on an authenticated call clears the session and signs out',
      () async {
        var signedOut = 0;
        final session = MemorySessionStore('tok');
        final adapter = FakeAdapter(
          (_) => (
            401,
            {
              'error': {'code': 'unauthorized', 'message': 'no'},
            },
          ),
        );
        final c = clientFor(
          adapter,
          session,
          onUnauthorized: () => signedOut++,
        );
        await c.get('/me').catchError((Object _) => null);
        expect(signedOut, 1);
        expect(await session.read(), isNull);
      },
    );

    test('a 401 from sign-in itself does not sign out', () async {
      var signedOut = 0;
      final session = MemorySessionStore('tok');
      final adapter = FakeAdapter(
        (_) => (
          401,
          {
            'error': {'code': 'invalid_credentials', 'message': 'bad'},
          },
        ),
      );
      final c = clientFor(adapter, session, onUnauthorized: () => signedOut++);
      await c.post('/auth/sign-in', auth: false).catchError((Object _) => null);
      expect(signedOut, 0);
      expect(await session.read(), 'tok');
    });

    test('reports an unreachable server plainly', () async {
      final adapter = FakeAdapter((_) => (200, {}))
        ..failWith = DioException(
          requestOptions: RequestOptions(path: '/'),
          type: DioExceptionType.connectionError,
        );
      final c = clientFor(adapter, MemorySessionStore());
      final e = await c
          .get('/me')
          .then<ApiError?>((_) => null)
          .catchError((Object e) => e as ApiError);
      expect(e!.isNetwork, isTrue);
      expect(e.message, contains('Could not reach the server'));
    });

    test('returns null for an empty 204', () async {
      final adapter = FakeAdapter((_) => (204, null));
      final c = clientFor(adapter, MemorySessionStore('t'));
      expect(await c.post('/auth/sign-out'), isNull);
    });
  });

  group('formatKobo', () {
    test('formats naira with grouping', () {
      expect(formatKobo(123450), '₦1,234.50');
      expect(formatKobo(5), '₦0.05');
      expect(formatKobo(100000000), '₦1,000,000.00');
      expect(formatKobo(-250000), '-₦2,500.00');
      expect(formatKobo(null), '–');
    });
  });
}
