import 'package:dio/dio.dart';
import 'package:flutter/foundation.dart';

import 'api_error.dart';

const _authKey = 'auth';
const _plainHttpHosts = {'10.0.2.2', 'localhost', '127.0.0.1'};

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
        error = ApiError.fromResponse(
          response?.statusCode ?? 0,
          response?.data,
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
    try {
      final response = await dio.request<Object?>(
        path,
        data: body,
        queryParameters: query,
        options: Options(method: method, extra: {_authKey: auth}),
      );
      if (response.statusCode == 204) return null;
      final data = response.data;
      if (data is String && data.isEmpty) return null;
      return data;
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
