import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/core/theme/tokens.dart';
import 'package:market/core/ui/brand_header.dart';
import 'package:market/core/ui/code_boxes.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/pump_app.dart';

void main() {
  testWidgets(
    'the_header_shows_title_and_subtitle_and_the_sheet_holds_the_child',
    (tester) async {
      await pumpApp(
        tester,
        const BrandHeaderScaffold(
          title: 'Welcome back',
          subtitle: 'Sign in to keep shopping',
          child: Text('the form'),
        ),
        scaffold: false,
      );

      expect(find.text(Strings.wordmark), findsOneWidget);
      expect(find.text('Welcome back'), findsOneWidget);
      expect(find.text('Sign in to keep shopping'), findsOneWidget);
      expect(find.text('the form'), findsOneWidget);
      expect(find.byTooltip(Strings.back), findsNothing);
    },
  );

  testWidgets('the_sheet_scrolls_so_a_focused_field_stays_above_the_keyboard', (
    tester,
  ) async {
    await pumpApp(
      tester,
      const BrandHeaderScaffold(
        title: 'Create your account',
        subtitle: 'It takes a minute',
        child: Column(
          children: [
            SizedBox(height: 400),
            TextField(key: ValueKey('field')),
          ],
        ),
      ),
      scaffold: false,
      viewInsets: const EdgeInsets.only(bottom: 300),
    );

    expect(tester.takeException(), isNull);

    await tester.ensureVisible(find.byKey(const ValueKey('field')));
    await tester.pump();

    final bottom = tester.getBottomLeft(find.byKey(const ValueKey('field'))).dy;
    expect(bottom, lessThanOrEqualTo(640 - 300));
  });

  for (final scale in [1.0, 2.0]) {
    testWidgets(
      'code_boxes_work_inside_the_header_at_${scale}x_with_keyboard',
      (tester) async {
        await pumpApp(
          tester,
          BrandHeaderScaffold(
            title: 'Enter your code',
            subtitle: 'We sent it by SMS',
            showBack: true,
            child: CodeBoxes(
              onChanged: (_) {},
              errorText: 'That code is wrong',
            ),
          ),
          scaffold: false,
          textScale: scale,
          viewInsets: const EdgeInsets.only(bottom: 300),
        );

        expect(tester.takeException(), isNull);
        expect(find.byType(CodeBoxes), findsOneWidget);
        expect(find.text('That code is wrong'), findsOneWidget);
      },
    );
  }

  testWidgets('the_header_does_not_overflow_at_200_percent_text', (
    tester,
  ) async {
    await pumpApp(
      tester,
      const BrandHeaderScaffold(
        title: 'Create your account',
        subtitle: 'It takes a minute and you can change it later',
        showBack: true,
        child: Text('the form'),
      ),
      scaffold: false,
      textScale: 2.0,
    );

    expect(tester.takeException(), isNull);
  });

  testWidgets('the_back_button_is_labelled_and_48dp_when_shown', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    await pumpApp(
      tester,
      const BrandHeaderScaffold(
        title: 'Reset password',
        showBack: true,
        child: SizedBox(),
      ),
      scaffold: false,
    );

    expect(find.bySemanticsLabel(Strings.back), findsOneWidget);
    final size = tester.getSize(find.byTooltip(Strings.back));
    expect(size.width, greaterThanOrEqualTo(48));
    expect(size.height, greaterThanOrEqualTo(48));
    semantics.dispose();
  });

  testWidgets(
    'the_theme_gives_primary_buttons_the_accent_pill_and_fields_the_line_border',
    (tester) async {
      await pumpApp(
        tester,
        Column(
          children: [
            FilledButton(onPressed: () {}, child: const Text('Primary')),
            OutlinedButton(onPressed: () {}, child: const Text('Secondary')),
          ],
        ),
      );

      final primary = tester.widget<Material>(
        find.descendant(
          of: find.byType(FilledButton),
          matching: find.byType(Material),
        ),
      );
      expect(primary.color, Tokens.light.accent);
      expect(primary.shape, isA<StadiumBorder>());
      expect(
        tester.getSize(find.byType(FilledButton)).width,
        320,
        reason: 'primary buttons are full width',
      );

      final theme = lightTheme();
      final secondary = theme.outlinedButtonTheme.style!;
      expect(secondary.side!.resolve({})!.color, Tokens.light.line);
      expect(secondary.shape!.resolve({}), isA<StadiumBorder>());

      final decoration = theme.inputDecorationTheme;
      expect(
        (decoration.enabledBorder! as OutlineInputBorder).borderSide.color,
        Tokens.light.line,
      );
      expect(
        (decoration.focusedBorder! as OutlineInputBorder).borderSide.color,
        Tokens.light.accent,
      );
      expect(
        (decoration.errorBorder! as OutlineInputBorder).borderSide.color,
        Tokens.light.bad,
      );
    },
  );

  testWidgets('the_dark_theme_uses_the_derived_dark_palette', (tester) async {
    final theme = darkTheme();

    expect(theme.brightness, Brightness.dark);
    expect(theme.scaffoldBackgroundColor, Tokens.dark.bg);
    expect(theme.extension<TokenColors>(), Tokens.dark);
    expect(Tokens.dark.accent, Tokens.light.accent);
  });
}
