import 'dart:math' as math;

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/theme/tokens.dart';
import 'package:material_ui/material_ui.dart';

double _lin(double c) =>
    c <= 0.03928 ? c / 12.92 : math.pow((c + 0.055) / 1.055, 2.4).toDouble();

double _luminance(Color c) =>
    0.2126 * _lin(c.r) + 0.7152 * _lin(c.g) + 0.0722 * _lin(c.b);

/// WCAG 2.x contrast ratio.
double contrast(Color a, Color b) {
  final la = _luminance(a);
  final lb = _luminance(b);
  final hi = la > lb ? la : lb;
  final lo = la > lb ? lb : la;
  return (hi + 0.05) / (lo + 0.05);
}

void main() {
  final palettes = {'light': Tokens.light, 'dark': Tokens.dark};

  for (final entry in palettes.entries) {
    final c = entry.value;

    test('error_text_meets_aa_on_bg_and_card_in_${entry.key}', () {
      expect(contrast(c.errorText, c.bg), greaterThanOrEqualTo(4.5));
      expect(contrast(c.errorText, c.card), greaterThanOrEqualTo(4.5));
    });

    final chips = {
      'paid': c.paid,
      'on_the_way': c.onTheWay,
      'delivered': c.delivered,
      'report_open': c.reportOpen,
    };
    for (final chip in chips.entries) {
      test('status_chip_${chip.key}_meets_aa_in_${entry.key}', () {
        expect(
          contrast(chip.value.foreground, chip.value.background),
          greaterThanOrEqualTo(4.5),
        );
      });
    }
  }
}
