import 'package:flutter/services.dart';

/// A NIN is exactly 11 digits.
const int ninLength = 11;

/// Keeps only digits and stops at 11, like the phone fields.
final TextInputFormatter ninFormatter = TextInputFormatter.withFunction((
  old,
  next,
) {
  final digits = next.text.replaceAll(RegExp(r'[^0-9]'), '');
  final cut = digits.length > ninLength
      ? digits.substring(0, ninLength)
      : digits;
  if (cut == next.text) return next;
  return TextEditingValue(
    text: cut,
    selection: TextSelection.collapsed(offset: cut.length),
  );
});

bool isCompleteNin(String text) => text.length == ninLength;
