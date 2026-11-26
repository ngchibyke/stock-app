/// A release build must talk to the production server over https and nowhere else. This is
/// checked at start-up (and by the release workflow before it builds), so a build made with a
/// missing or development address stops at once with a plain message instead of shipping.
///
/// Debug and profile builds are not checked: they use the emulator's address by default.
void validateApiBaseUrl(String url, {required bool release}) {
  if (!release) return;
  if (url.trim().isEmpty) {
    throw StateError(
      'API_BASE_URL is empty. A release build needs the production address: '
      '--dart-define=API_BASE_URL=https://...',
    );
  }
  final uri = Uri.tryParse(url);
  if (uri == null || uri.scheme != 'https' || uri.host.isEmpty) {
    throw StateError(
      'API_BASE_URL must be an https address in a release build.',
    );
  }
  if (uri.userInfo.isNotEmpty) {
    throw StateError('API_BASE_URL must not carry a user name or password.');
  }
  if (_development(uri.host)) {
    throw StateError(
      'API_BASE_URL points at a development address, not the production server.',
    );
  }
}

bool _development(String host) {
  final h = host.toLowerCase();
  if (h == 'localhost' || h == '10.0.2.2' || h.endsWith('.local')) return true;
  if (h.startsWith('127.') || h.startsWith('192.168.') || h.startsWith('10.')) {
    return true;
  }
  final m = RegExp(r'^172\.(\d+)\.').firstMatch(h);
  if (m != null) {
    final second = int.parse(m[1]!);
    if (second >= 16 && second <= 31) return true;
  }
  return h == '::1' || h == '[::1]';
}
