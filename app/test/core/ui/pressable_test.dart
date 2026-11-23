import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/core/ui/pressable.dart';
import 'package:material_ui/material_ui.dart';

Future<void> _pump(
  WidgetTester tester, {
  VoidCallback? onTap,
  bool reduceMotion = false,
}) => tester.pumpWidget(
  MaterialApp(
    theme: lightTheme(),
    home: MediaQuery(
      data: MediaQueryData(disableAnimations: reduceMotion),
      child: Center(
        child: Pressable(
          onTap: onTap,
          semanticLabel: 'Go',
          child: const SizedBox.square(dimension: 48),
        ),
      ),
    ),
  ),
);

double _scale(WidgetTester tester) =>
    tester.widget<AnimatedScale>(find.byType(AnimatedScale)).scale;

void main() {
  testWidgets('pressing_sinks_it_and_letting_go_fires_once', (tester) async {
    var taps = 0;
    await _pump(tester, onTap: () => taps++);
    final g = await tester.startGesture(
      tester.getCenter(find.byType(Pressable)),
    );
    await tester.pump();
    expect(_scale(tester), 0.97);
    await g.up();
    await tester.pump();
    expect(_scale(tester), 1);
    expect(taps, 1);
  });

  testWidgets('reduced_motion_does_not_sink', (tester) async {
    await _pump(tester, onTap: () {}, reduceMotion: true);
    final g = await tester.startGesture(
      tester.getCenter(find.byType(Pressable)),
    );
    await tester.pump();
    expect(_scale(tester), 1);
    await g.up();
  });

  testWidgets('disabled_does_nothing_and_reads_as_a_button', (tester) async {
    await _pump(tester);
    final g = await tester.startGesture(
      tester.getCenter(find.byType(Pressable)),
    );
    await tester.pump();
    expect(_scale(tester), 1);
    await g.up();
    expect(find.bySemanticsLabel('Go'), findsOneWidget);
  });
}
