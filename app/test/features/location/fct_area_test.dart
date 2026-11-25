import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/location/fct_area.dart';

void main() {
  test('places in the FCT and the box corners are inside', () {
    for (final (lat, lng) in [
      (9.0765, 7.3986), // Wuse
      (8.9400, 7.0800), // Gwagwalada
      (8.40, 6.75),
      (9.35, 7.62),
    ]) {
      expect(inFct(lat, lng), isTrue, reason: '$lat,$lng');
    }
  });

  test('places outside the FCT are outside', () {
    for (final (lat, lng) in [
      (6.5244, 3.3792), // Lagos
      (12.0022, 8.5920), // Kano
      (8.39, 7.0),
      (9.0, 7.63),
    ]) {
      expect(inFct(lat, lng), isFalse, reason: '$lat,$lng');
    }
  });
}
