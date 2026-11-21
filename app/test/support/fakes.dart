import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';

/// One request as the [FakeAdapter] saw it.
class RecordedRequest {
  RecordedRequest({
    required this.method,
    required this.path,
    required this.headers,
    required this.body,
  });

  final String method;
  final String path;
  final Map<String, dynamic> headers;
  final Object? body;
}

class _Reply {
  _Reply(this.status, this.body, this.contentType);
  final int status;
  final Object? body;
  final String? contentType;
}

/// An [HttpClientAdapter] that answers queued responses and records requests.
class FakeAdapter implements HttpClientAdapter {
  final List<RecordedRequest> requests = [];
  final List<Object> _queue = [];

  /// Queue a response. A Map or List body is sent as JSON, a String as the
  /// given [contentType] (default text/html), and null as an empty body.
  void reply(int status, [Object? body, String? contentType]) {
    _queue.add(_Reply(status, body, contentType));
  }

  /// Queue a connection failure (no network).
  void failWithConnectionError() =>
      _queue.add(DioExceptionType.connectionError);

  /// Queue a timeout.
  void failWithTimeout() => _queue.add(DioExceptionType.receiveTimeout);

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    requests.add(
      RecordedRequest(
        method: options.method,
        path: options.uri.path,
        headers: Map<String, dynamic>.from(options.headers),
        body: options.data,
      ),
    );
    if (_queue.isEmpty) {
      throw StateError('FakeAdapter has no queued response for ${options.uri}');
    }
    final next = _queue.removeAt(0);
    if (next is DioExceptionType) {
      throw DioException(requestOptions: options, type: next);
    }
    final reply = next as _Reply;
    final body = reply.body;
    final headers = <String, List<String>>{};
    Uint8List bytes;
    if (body == null) {
      bytes = Uint8List(0);
    } else if (body is String) {
      bytes = Uint8List.fromList(utf8.encode(body));
      headers['content-type'] = [reply.contentType ?? 'text/html'];
    } else {
      bytes = Uint8List.fromList(utf8.encode(jsonEncode(body)));
      headers['content-type'] = ['application/json; charset=utf-8'];
    }
    return ResponseBody.fromBytes(bytes, reply.status, headers: headers);
  }

  @override
  void close({bool force = false}) {}
}
