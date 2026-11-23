// Shared by the wire checks: they talk to a real local server, sign people
// up and read one-time codes from its log.
import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';

final wireBaseUrl = Platform.environment['WIRE_BASE_URL'] ?? '';
final wireOtpLog = Platform.environment['WIRE_OTP_LOG'] ?? '';
final String? wireSkip = (wireBaseUrl.isEmpty || wireOtpLog.isEmpty)
    ? 'set WIRE_BASE_URL and WIRE_OTP_LOG to run the wire check'
    : null;

/// True only for an http(s) URL whose host is exactly localhost, 127.0.0.1 or
/// ::1 (and carries no user info). The wire check signs people up and asks for
/// SMS codes, so it must never be pointed at a shared or remote server.
bool isLoopbackBaseUrl(String url) {
  final uri = Uri.tryParse(url);
  if (uri == null) return false;
  if (uri.scheme != 'http' && uri.scheme != 'https') return false;
  if (uri.userInfo.isNotEmpty) return false;
  return const {'localhost', '127.0.0.1', '::1'}.contains(uri.host);
}

final _codeLine = RegExp(r'OTP code:\s*(\d{6})');

List<String> _codeLines() {
  final file = File(wireOtpLog);
  if (!file.existsSync()) return const [];
  return [
    for (final line in file.readAsLinesSync())
      if (_codeLine.hasMatch(line)) line,
  ];
}

/// Runs [trigger] (which makes the server send a code) and answers the code
/// from the NEW line that appeared in the server log afterwards.
Future<String> codeAfter(Future<void> Function() trigger) async {
  final before = _codeLines().length;
  await trigger();
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    final lines = _codeLines();
    if (lines.length > before) {
      return _codeLine.firstMatch(lines.last)!.group(1)!;
    }
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  fail('no new "OTP code" line in the server log within 10 seconds');
}

final _random = Random.secure();
// Distinct per run, so the check can be re-run against the same scratch
// database. Valid Nigerian mobile numbers: 0803 + 7 digits.
final int _phoneBase =
    (DateTime.now().millisecondsSinceEpoch ~/ 1000 % 1000000) * 10;
int _phoneCount = 0;
String nextPhone() =>
    '0803${(_phoneBase + _phoneCount++).toString().padLeft(7, '0')}';

String freshPassword() =>
    'Wc-${List.generate(12, (_) => _random.nextInt(36).toRadixString(36)).join()}';
