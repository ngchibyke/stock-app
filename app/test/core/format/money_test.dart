import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/format/money.dart';

void main() {
  group('formatNaira', () {
    test('whole naira has no decimals', () {
      expect(formatNaira(1200000), '₦12,000');
      expect(formatNaira(0), '₦0');
      expect(formatNaira(100), '₦1');
      expect(formatNaira(1000000000), '₦10,000,000');
    });

    test('kobo shows two decimals', () {
      expect(formatNaira(1250), '₦12.50');
      expect(formatNaira(99), '₦0.99');
    });

    test('stays exact above 2^53 kobo', () {
      expect(formatNaira(9007199254740993), '₦90,071,992,547,409.93');
    });
  });
}
