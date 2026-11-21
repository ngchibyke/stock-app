import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/theme/tokens.dart';
import 'package:market/core/ui/code_boxes.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/pump_app.dart';

BoxDecoration _decorationOf(WidgetTester tester, int index) {
  final box = tester.widget<Container>(find.byKey(ValueKey('code-box-$index')));
  return box.decoration! as BoxDecoration;
}

void main() {
  testWidgets(
    'code_boxes_accept_only_digits_strip_pasted_spaces_and_ignore_overflow',
    (tester) async {
      final changes = <String>[];
      await pumpApp(tester, CodeBoxes(onChanged: changes.add));

      await tester.enterText(find.byType(TextField), '12a 3456 78');
      await tester.pump();

      expect(changes.last, '123456');
      expect(find.text('1'), findsOneWidget);
      expect(find.text('6'), findsOneWidget);
      expect(find.text('7'), findsNothing);
    },
  );

  testWidgets('code_boxes_report_completion_once_at_full_length', (
    tester,
  ) async {
    final completed = <String>[];
    await pumpApp(
      tester,
      CodeBoxes(onChanged: (_) {}, onCompleted: completed.add),
    );

    await tester.enterText(find.byType(TextField), '12345');
    await tester.pump();
    expect(completed, isEmpty);

    await tester.enterText(find.byType(TextField), '123456');
    await tester.pump();
    expect(completed, ['123456']);

    // More input is ignored, so completion is not reported a second time.
    await tester.enterText(find.byType(TextField), '1234567');
    await tester.pump();
    expect(completed, ['123456']);
  });

  testWidgets('code_boxes_show_the_error_text_and_not_colour_alone', (
    tester,
  ) async {
    await pumpApp(
      tester,
      CodeBoxes(onChanged: (_) {}, errorText: 'That code is not right'),
    );

    expect(find.text('That code is not right'), findsOneWidget);
    expect(_decorationOf(tester, 0).border!.top.color, Tokens.light.bad);
  });

  testWidgets(
    'code_boxes_boxes_meet_the_tap_target_and_do_not_overflow_at_200_percent_text',
    (tester) async {
      await pumpApp(
        tester,
        CodeBoxes(onChanged: (_) {}, errorText: 'That code is not right'),
        textScale: 2.0,
      );

      expect(tester.takeException(), isNull);

      // The row is one tap target (any box focuses the hidden field).
      final row = tester.getSize(find.byType(CodeBoxes));
      expect(row.width, greaterThanOrEqualTo(48));
      expect(row.height, greaterThanOrEqualTo(48));
      for (var i = 0; i < 6; i++) {
        final box = tester.getSize(find.byKey(ValueKey('code-box-$i')));
        expect(box.height, greaterThanOrEqualTo(48));
      }
    },
  );

  testWidgets('code_boxes_highlight_the_active_box_with_the_accent_border', (
    tester,
  ) async {
    await pumpApp(tester, CodeBoxes(onChanged: (_) {}));

    await tester.tap(find.byType(TextField));
    await tester.pump();
    await tester.enterText(find.byType(TextField), '12');
    await tester.pump();

    expect(_decorationOf(tester, 2).border!.top.color, Tokens.light.accent);
    expect(_decorationOf(tester, 4).border!.top.color, Tokens.light.line);
  });

  testWidgets('code_boxes_offer_a_paste_action_and_clean_what_is_pasted', (
    tester,
  ) async {
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.getData') {
          return <String, dynamic>{'text': '12 3456 78'};
        }
        if (call.method == 'Clipboard.hasStrings') {
          return <String, dynamic>{'value': true};
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );

    final changes = <String>[];
    await pumpApp(tester, CodeBoxes(onChanged: changes.add));

    await tester.longPress(find.byType(TextField));
    await tester.pumpAndSettle();

    expect(find.text('Paste'), findsOneWidget);

    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(changes.last, '123456');
  });

  testWidgets('code_boxes_are_labelled_for_screen_readers', (tester) async {
    final semantics = tester.ensureSemantics();
    await pumpApp(tester, CodeBoxes(onChanged: (_) {}));

    expect(find.bySemanticsLabel(RegExp('6 digit')), findsOneWidget);
    semantics.dispose();
  });
}
