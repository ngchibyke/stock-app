/// Shapes a Nigerian mobile number as it is typed: "0803 123 4567". The server
/// decides what is valid; this only keeps the field readable.
String formatPhoneInput(String raw) {
  var digits = raw.replaceAll(RegExp(r'[^0-9+]'), '');
  final plus = digits.startsWith('+');
  digits = digits.replaceAll('+', '');
  if (plus || digits.startsWith('234')) {
    // International form: +234 803 123 4567
    final national = digits.startsWith('234') ? digits.substring(3) : digits;
    final n = national.length > 10 ? national.substring(0, 10) : national;
    return '+234${n.isEmpty ? '' : ' '}${_group(n, const [3, 3, 4])}';
  }
  final d = digits.length > 11 ? digits.substring(0, 11) : digits;
  return _group(d, const [4, 3, 4]);
}

String _group(String digits, List<int> sizes) {
  final parts = <String>[];
  var i = 0;
  for (final size in sizes) {
    if (i >= digits.length) break;
    final end = i + size > digits.length ? digits.length : i + size;
    parts.add(digits.substring(i, end));
    i = end;
  }
  return parts.join(' ');
}

/// What to send to the server: the digits (and a leading +), spaces removed.
String phoneForServer(String shown) => shown.replaceAll(RegExp(r'\s'), '');
