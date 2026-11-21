/// A failed API call, mapped from the server's error envelope
/// `{"error": {"code", "message", "retry_after_secs"?}}`.
///
/// The 403 `phone_unverified` error carries `verification_id` at the TOP LEVEL
/// of the body, not inside `error`.
class ApiError implements Exception {
  ApiError({
    required this.status,
    required this.code,
    required this.message,
    this.retryAfterSecs,
    this.verificationId,
  });

  factory ApiError.fromResponse(int status, Object? body) {
    var code = 'unknown';
    var message = 'Something went wrong. Please try again.';
    int? retryAfterSecs;
    String? verificationId;

    if (body is Map) {
      final error = body['error'];
      if (error is Map) {
        final c = error['code'];
        final m = error['message'];
        final r = error['retry_after_secs'];
        if (c is String && c.isNotEmpty) code = c;
        if (m is String && m.isNotEmpty) message = m;
        if (r is num) retryAfterSecs = r.toInt();
      }
      final v = body['verification_id'];
      if (v is String && v.isNotEmpty) verificationId = v;
    }

    return ApiError(
      status: status,
      code: code,
      message: message,
      retryAfterSecs: retryAfterSecs,
      verificationId: verificationId,
    );
  }

  /// No connection to the server at all.
  factory ApiError.offline() => ApiError(
    status: 0,
    code: 'offline',
    message: 'No internet connection. Check your network and try again.',
  );

  /// The server did not answer in time.
  factory ApiError.timeout() => ApiError(
    status: 0,
    code: 'timeout',
    message: 'The request took too long. Please try again.',
  );

  final int status;
  final String code;
  final String message;
  final int? retryAfterSecs;
  final String? verificationId;

  bool get isOffline => code == 'offline';
  bool get isTimeout => code == 'timeout';
  bool get isServerDown => status >= 500;

  // Deliberately omits any request or response content.
  @override
  String toString() => 'ApiError($status, $code)';
}
