final RegExp _separators = RegExp(r'[\s-]');
final RegExp _digitsOnly = RegExp(r'^\d+$');

/// Strips spaces and hyphens, and returns the national 11-digit form
/// (`0XXXXXXXXXX`) if [input] looks like a Nigerian number, else null.
String? _nationalForm(String input) {
  var s = input.replaceAll(_separators, '');
  if (s.startsWith('+')) s = s.substring(1);
  if (!_digitsOnly.hasMatch(s)) return null;
  if (s.length == 11 && s.startsWith('0')) return s;
  if (s.length == 13 && s.startsWith('234')) return '0${s.substring(3)}';
  return null;
}

/// `+2348031234567` -> `0803 123 4567`. Anything unparseable is returned
/// unchanged.
String displayPhone(String e164) {
  final national = _nationalForm(e164);
  if (national == null) return e164;
  return '${national.substring(0, 4)} ${national.substring(4, 7)} '
      '${national.substring(7)}';
}

/// A hint for the form only; the server normalises and decides.
bool looksLikeNigerianPhone(String input) => _nationalForm(input) != null;
