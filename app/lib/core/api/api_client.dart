import 'dart:convert';

import 'package:dio/dio.dart';
import 'package:flutter/foundation.dart';

import 'api_error.dart';

const _authKey = 'auth';
const _plainHttpHosts = {'10.0.2.2', 'localhost', '127.0.0.1'};
const Object _notJson = Object();

/// Decodes a JSON string; returns [_notJson] when it is not valid JSON.
Object? _tryDecode(String raw) {
  try {
    return jsonDecode(raw);
  } on FormatException {
    return _notJson;
  }
}

/// JSON over HTTP for the server API. Throws [ApiError] on any failure.
///
/// Installs no logging interceptor, and nothing here prints request or
/// response content, headers, tokens or bodies.
class ApiClient {
  ApiClient({
    required String baseUrl,
    required this._readToken,
    required this._onUnauthorized,
    HttpClientAdapter? adapter,
    @visibleForTesting bool? debugMode,
  }) {
    _checkBaseUrl(baseUrl, debugMode ?? kDebugMode);
    dio = Dio(
      BaseOptions(
        baseUrl: baseUrl,
        connectTimeout: const Duration(seconds: 15),
        receiveTimeout: const Duration(seconds: 15),
        sendTimeout: const Duration(seconds: 15),
        followRedirects: false,
        headers: {'Accept': 'application/json'},
        contentType: Headers.jsonContentType,
      ),
    );
    if (adapter != null) dio.httpClientAdapter = adapter;
    dio.interceptors.add(
      InterceptorsWrapper(onRequest: _onRequest, onError: _onError),
    );
  }

  late final Dio dio;
  final Future<String?> Function() _readToken;
  final void Function() _onUnauthorized;

  // The token that last triggered a sign-out. Further 401s carrying that same
  // token are ignored; a different token read later re-arms the callback.
  String? _signedOutToken;
  bool _signedOut = false;

  static void _checkBaseUrl(String baseUrl, bool debug) {
    final uri = Uri.tryParse(baseUrl);
    final ok =
        uri != null &&
        uri.host.isNotEmpty &&
        (uri.scheme == 'https' ||
            (uri.scheme == 'http' &&
                debug &&
                _plainHttpHosts.contains(uri.host)));
    if (!ok) {
      throw ArgumentError.value(
        baseUrl,
        'baseUrl',
        'must be https (plain http only for 10.0.2.2, localhost and '
            '127.0.0.1 in debug builds)',
      );
    }
  }

  Future<void> _onRequest(
    RequestOptions options,
    RequestInterceptorHandler handler,
  ) async {
    if (options.extra[_authKey] != false) {
      final token = await _readToken();
      if (token != null && token.isNotEmpty) {
        options.headers['Authorization'] = 'Bearer $token';
        options.extra['token'] = token;
        if (_signedOut && token != _signedOutToken) _signedOut = false;
      }
    }
    handler.next(options);
  }

  void _onError(DioException e, ErrorInterceptorHandler handler) {
    final ApiError error;
    switch (e.type) {
      case DioExceptionType.connectionTimeout:
      case DioExceptionType.sendTimeout:
      case DioExceptionType.receiveTimeout:
      case DioExceptionType.transformTimeout:
        error = ApiError.timeout();
      case DioExceptionType.badResponse:
        final response = e.response;
        final data = response?.data;
        error = ApiError.fromResponse(
          response?.statusCode ?? 0,
          data is String ? _tryDecode(data) : data,
        );
        final sentAuth = e.requestOptions.extra[_authKey] != false;
        if (error.status == 401 && sentAuth && !_signedOut) {
          _signedOut = true;
          _signedOutToken = e.requestOptions.extra['token'] as String?;
          _onUnauthorized();
        }
      case DioExceptionType.connectionError:
      case DioExceptionType.badCertificate:
      case DioExceptionType.cancel:
      case DioExceptionType.unknown:
        error = ApiError.offline();
    }
    handler.reject(
      DioException(
        requestOptions: e.requestOptions,
        response: null,
        type: e.type,
        error: error,
      ),
    );
  }

  Future<Object?> _send(
    String method,
    String path, {
    Map<String, Object?>? query,
    Object? body,
    bool auth = true,
  }) async {
    // Dio ignores baseUrl for an absolute URL, which would send the bearer
    // token to another host (or over plain http). Only server-relative paths.
    final parsed = Uri.tryParse(path);
    if (parsed == null ||
        !path.startsWith('/') ||
        path.startsWith('//') ||
        parsed.hasScheme ||
        parsed.hasAuthority) {
      throw ApiError(
        status: 0,
        code: 'invalid_path',
        message: 'Invalid request path.',
      );
    }
    try {
      final response = await dio.request<String>(
        path,
        data: body,
        queryParameters: query,
        options: Options(
          method: method,
          extra: {_authKey: auth},
          // Decoded here so a malformed body is a typed error, not a dio
          // "unknown" exception that would read as being offline.
          responseType: ResponseType.plain,
        ),
      );
      final status = response.statusCode ?? 0;
      final raw = response.data;
      if (status == 204 || raw == null || raw.isEmpty) return null;
      final decoded = _tryDecode(raw);
      if (decoded is Map || decoded is List || decoded == null) {
        // A literal `null` body decodes to null; "not JSON" is the sentinel.
        if (!identical(decoded, _notJson)) return decoded;
      }
      throw ApiError(
        status: status,
        code: 'bad_response',
        message: 'The server sent an unexpected response.',
      );
    } on DioException catch (e) {
      final error = e.error;
      if (error is ApiError) throw error;
      throw ApiError.offline();
    }
  }

  Future<Object?> get(
    String path, {
    Map<String, Object?>? query,
    bool auth = true,
  }) => _send('GET', path, query: query, auth: auth);

  Future<Object?> post(String path, {Object? body, bool auth = true}) =>
      _send('POST', path, body: body, auth: auth);

  Future<Object?> put(String path, {Object? body}) =>
      _send('PUT', path, body: body);

  Future<Object?> delete(String path) => _send('DELETE', path);
}
