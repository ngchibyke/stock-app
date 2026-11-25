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

  group('PhoneInputFormatter', () {
    String type(String text) => const PhoneInputFormatter()
        .formatEditUpdate(TextEditingValue.empty, TextEditingValue(text: text))
        .text;

    test('never lets more than 11 digits in', () {
      expect(type('080312345678999'), '08031234567');
    });

    test('keeps digits only', () {
      expect(type('0803-123 45a67'), '08031234567');
    });

    test('turns a full +234 number into the 11-digit form', () {
      expect(type('+2348031234567'), '08031234567');
      expect(type('2348031234567'), '08031234567');
    });

    test('leaves a short number as typed', () {
      expect(type('0803'), '0803');
    });
  });
}
