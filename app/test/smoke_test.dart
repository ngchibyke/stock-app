import 'package:flutter_test/flutter_test.dart';
import 'package:market/app.dart';
import 'package:market/core/strings.dart';
import 'package:material_ui/material_ui.dart';

import 'support/app_harness.dart';

void main() {
  testWidgets('app_builds_a_material_app', (tester) async {
    final h = AppHarness();
    await h.pump(tester);

    expect(find.byType(MarketApp), findsOneWidget);
    expect(find.byType(MaterialApp), findsOneWidget);
    // A signed-out start reaches the sign-in page.
    expect(find.text(Strings.signInTitle), findsOneWidget);
  });
}
