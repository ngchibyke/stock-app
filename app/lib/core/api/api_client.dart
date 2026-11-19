import 'dart:convert';

import 'package:dio/dio.dart';

import 'api_error.dart';
import 'session.dart';

/// The only thing in the app that speaks HTTP.
///
/// - Sends the bearer token when there is one (and `auth` is true).
/// - Turns the server's error envelope into an [ApiError].
/// - A 401 on an authenticated call means the session is gone: it clears the
///   token and calls [onUnauthorized] so the app can sign the person out.
/// - Never logs a request or a response body.
class ApiClient {
  ApiClient({
    required String baseUrl,
    required this._session,
    this.onUnauthorized,
    Dio? dio,
  }) : _dio = dio ?? Dio() {
    _dio.options
      ..baseUrl = baseUrl
      ..connectTimeout = const Duration(seconds: 15)
      ..receiveTimeout = const Duration(seconds: 30)
      // Statuses are handled here, not by dio's exceptions.
      ..validateStatus = (_) => true;
  }

  final Dio _dio;
  final SessionStore _session;
  void Function()? onUnauthorized;

  Future<dynamic> get(String path, {Map<String, dynamic>? query}) =>
      _send('GET', path, query: query);

  Future<dynamic> post(
    String path, {
    Object? body,
    bool auth = true,
    Map<String, String>? headers,
  }) =>
      _send('POST', path, body: body ?? const {}, auth: auth, headers: headers);

  Future<dynamic> put(String path, {Object? body}) =>
      _send('PUT', path, body: body ?? const {});

  Future<dynamic> patch(String path, {Object? body}) =>
      _send('PATCH', path, body: body ?? const {});

  Future<dynamic> delete(String path) => _send('DELETE', path);

  /// Query values that are null or empty are left out.
  static Map<String, dynamic>? cleanQuery(Map<String, dynamic>? query) {
    if (query == null) return null;
    final out = <String, dynamic>{};
    query.forEach((k, v) {
      if (v == null || (v is String && v.isEmpty)) return;
      out[k] = v;
    });
    return out.isEmpty ? null : out;
  }

  Future<dynamic> _send(
    String method,
    String path, {
    Object? body,
    Map<String, dynamic>? query,
    bool auth = true,
    Map<String, String>? headers,
  }) async {
    final token = auth ? await _session.read() : null;
    final Response<dynamic> res;
    try {
      res = await _dio.request<dynamic>(
        path,
        data: body,
        queryParameters: cleanQuery(query),
        options: Options(
          method: method,
          responseType: ResponseType.plain,
          headers: {
            'Accept': 'application/json',
            if (body != null) 'Content-Type': 'application/json',
            if (token != null) 'Authorization': 'Bearer $token',
            ...?headers,
          },
        ),
      );
    } on DioException catch (e) {
      if (e.type == DioExceptionType.cancel) rethrow;
      throw const ApiError(
        0,
        'network',
        'Could not reach the server. Check your connection and try again.',
      );
    }

    final text = res.data is String ? res.data as String : '';
    dynamic json;
    if (text.isNotEmpty) {
      try {
        json = jsonDecode(text);
      } catch (_) {
        json = null;
      }
    }
    final status = res.statusCode ?? 0;
    if (status >= 200 && status < 300) return json;

    final map = json is Map<String, dynamic> ? json : const <String, dynamic>{};
    final err = map['error'] is Map<String, dynamic>
        ? map['error'] as Map<String, dynamic>
        : const <String, dynamic>{};
    if (status == 401 && auth && token != null) {
      await _session.clear();
      onUnauthorized?.call();
    }
    throw ApiError(
      status,
      (err['code'] as String?) ?? 'unknown',
      (err['message'] as String?) ?? 'The server answered $status.',
      extra: {
        for (final e in map.entries)
          if (e.key != 'error') e.key: e.value,
      },
    );
  }
}
