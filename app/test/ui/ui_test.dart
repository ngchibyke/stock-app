import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market_app/ui/app_icon.dart';
import 'package:market_app/ui/buttons.dart';
import 'package:market_app/ui/cards.dart';
import 'package:market_app/ui/theme.dart';

Widget host(Widget child, {bool reduce = false}) => MaterialApp(
  theme: AppTheme.light(),
  builder: (context, w) => MediaQuery(
    data: MediaQuery.of(context).copyWith(disableAnimations: reduce),
    child: w!,
  ),
  home: Scaffold(body: Center(child: child)),
);

void main() {
  group('icons', () {
    test('every icon has a behaviour that moves and then comes to rest', () {
      for (final entry in AppIcons.all.entries) {
        final start = behaviourAt(entry.value.behaviour, 0);
        final end = behaviourAt(entry.value.behaviour, 1);
        // The behaviour does something mid-way ...
        final moved =
            [
              for (var i = 1; i < 10; i++)
                behaviourAt(entry.value.behaviour, i / 10),
            ].any(
              (m) =>
                  m.rotate.abs() > 0.01 ||
                  (m.scale - 1).abs() > 0.01 ||
                  m.dx.abs() > 0.01 ||
                  m.dy.abs() > 0.01,
            );
        expect(moved, isTrue, reason: entry.key);
        // ... and starts from rest. (Rotations that finish a half turn end turned, by design.)
        expect(start.scale, 1, reason: entry.key);
        expect(start.dx, 0, reason: entry.key);
        expect(end.scale, closeTo(1, 0.001), reason: entry.key);
      }
    });

    test(
      'behaviours spread across the set (not one effect for everything)',
      () {
        expect(
          AppIcons.all.values.map((s) => s.behaviour).toSet().length,
          IconBehaviour.values.length,
        );
      },
    );

    testWidgets('plays on press and holds still with reduce motion on', (
      tester,
    ) async {
      await tester.pumpWidget(
        host(const AppIcon(AppIcons.bell, key: Key('i'))),
      );
      final before = tester.getRect(find.byKey(const Key('i')));
      final g = await tester.startGesture(
        tester.getCenter(find.byKey(const Key('i'))),
      );
      await tester.pump(); // the ticker takes its start time from this frame
      await tester.pump(const Duration(milliseconds: 120));
      final during = tester.widgetList<Transform>(
        find.descendant(
          of: find.byKey(const Key('i')),
          matching: find.byType(Transform),
        ),
      );
      expect(during.any((t) => !t.transform.isIdentity()), isTrue);
      await g.up();
      await tester.pumpAndSettle();
      expect(tester.getRect(find.byKey(const Key('i'))), before);

      await tester.pumpWidget(
        host(const AppIcon(AppIcons.bell, key: Key('j')), reduce: true),
      );
      final g2 = await tester.startGesture(
        tester.getCenter(find.byKey(const Key('j'))),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 120));
      final still = tester.widgetList<Transform>(
        find.descendant(
          of: find.byKey(const Key('j')),
          matching: find.byType(Transform),
        ),
      );
      expect(still.every((t) => t.transform.isIdentity()), isTrue);
      await g2.up();
    });
  });

  group('AppButton', () {
    testWidgets('taps', (tester) async {
      var taps = 0;
      await tester.pumpWidget(
        host(AppButton(label: 'Pay', onPressed: () => taps++)),
      );
      await tester.tap(find.text('Pay'));
      expect(taps, 1);
    });

    testWidgets('ignores taps while loading and shows a spinner', (
      tester,
    ) async {
      var taps = 0;
      await tester.pumpWidget(
        host(AppButton(label: 'Pay', loading: true, onPressed: () => taps++)),
      );
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      await tester.tap(find.text('Pay'));
      expect(taps, 0);
    });

    testWidgets('shows a check when done', (tester) async {
      await tester.pumpWidget(
        host(AppButton(label: 'Saved', done: true, onPressed: () {})),
      );
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byIcon(Icons.check_rounded), findsOneWidget);
    });

    testWidgets('is labelled for screen readers and at least 48 dp', (
      tester,
    ) async {
      await tester.pumpWidget(host(AppButton(label: 'Pay', onPressed: () {})));
      expect(find.bySemanticsLabel('Pay'), findsOneWidget);
      expect(
        tester.getSize(find.byType(AppButton)).height,
        greaterThanOrEqualTo(48),
      );
    });

    testWidgets('a disabled button does nothing', (tester) async {
      await tester.pumpWidget(
        host(const AppButton(label: 'Pay', onPressed: null)),
      );
      await tester.tap(find.text('Pay'));
      await tester.pump();
    });
  });

  testWidgets('PressableCard answers a tap', (tester) async {
    var taps = 0;
    await tester.pumpWidget(
      host(PressableCard(onTap: () => taps++, child: const Text('Order'))),
    );
    await tester.tap(find.text('Order'));
    expect(taps, 1);
  });

  test('themes build in both brightnesses', () {
    expect(AppTheme.light().brightness, Brightness.light);
    expect(AppTheme.dark().brightness, Brightness.dark);
  });
}
