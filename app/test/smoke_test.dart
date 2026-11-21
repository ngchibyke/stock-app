import 'package:material_ui/material_ui.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('app_builds_a_material_app', (tester) async {
    await tester.pumpWidget(
      const ProviderScope(child: MaterialApp(home: SizedBox())),
    );

    expect(find.byType(MaterialApp), findsOneWidget);
  });
}
