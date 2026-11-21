import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/format/phone.dart';

void main() {
  group('displayPhone', () {
    test('formats an E.164 Nigerian number', () {
      expect(displayPhone('+2348031234567'), '0803 123 4567');
    });

    test('returns unparseable input unchanged', () {
      expect(displayPhone('12345'), '12345');
      expect(displayPhone(''), '');
      expect(displayPhone('+447911123456'), '+447911123456');
    });
  });

  group('looksLikeNigerianPhone', () {
    test('accepts common forms', () {
      for (final s in [
        '08031234567',
        '+2348031234567',
        '+234 803 123 4567',
        '0803-123-4567',
        '2348031234567',
      ]) {
        expect(looksLikeNigerianPhone(s), isTrue, reason: s);
      }
    });

    test('rejects bad input', () {
      for (final s in ['12345', '', '0803123456', 'abcdefghijk']) {
        expect(looksLikeNigerianPhone(s), isFalse, reason: s);
      }
    });
  });
}
