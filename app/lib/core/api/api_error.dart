/// A failed API call. The server answers errors with
/// `{ "error": { "code": "...", "message": "..." } }` and sometimes extra
/// top-level fields (`verification_id` on `phone_unverified`, `muted_until` on
/// `muted`); those are kept in [extra].
class ApiError implements Exception {
  const ApiError(this.status, this.code, this.message, {this.extra = const {}});

  /// 0 when the server could not be reached at all.
  final int status;
  final String code;

  /// The server's own wording, safe to show to the person.
  final String message;
  final Map<String, dynamic> extra;

  bool get isNetwork => status == 0;
  bool get isUnauthorized => status == 401;

  @override
  String toString() => 'ApiError($status $code)';
}
