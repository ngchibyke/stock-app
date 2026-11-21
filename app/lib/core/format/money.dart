import 'package:intl/intl.dart';

/// Formats an amount in kobo as naira, e.g. `1250` -> `₦12.50`.
///
/// All arithmetic is integer: the whole part is `kobo ~/ 100` and the
/// fraction is `kobo % 100`, so values above 2^53 stay exact.
String formatNaira(int kobo) {
  final negative = kobo < 0;
  final abs = kobo.abs();
  final whole = abs ~/ 100;
  final fraction = abs % 100;
  final grouped = NumberFormat.decimalPattern('en_NG').format(whole);
  final decimals = fraction == 0
      ? ''
      : '.${fraction.toString().padLeft(2, '0')}';
  return '${negative ? '-' : ''}₦$grouped$decimals';
}
