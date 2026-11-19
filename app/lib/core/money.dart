/// Kobo as naira with grouping, e.g. 123450 -> "₦1,234.50". Money is always
/// whole kobo in the app; this is the only place it is turned into text.
String formatKobo(int? kobo) {
  if (kobo == null) return '–';
  final sign = kobo < 0 ? '-' : '';
  final abs = kobo.abs();
  final naira = abs ~/ 100;
  final rest = (abs % 100).toString().padLeft(2, '0');
  final grouped = naira.toString().replaceAllMapped(
    RegExp(r'\B(?=(\d{3})+(?!\d))'),
    (_) => ',',
  );
  return '$sign₦$grouped.$rest';
}
