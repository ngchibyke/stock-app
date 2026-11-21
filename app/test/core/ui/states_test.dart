import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/ui/states.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/pump_app.dart';

void main() {
  testWidgets('empty_state_shows_title_and_action', (tester) async {
    var taps = 0;
    await pumpApp(
      tester,
      EmptyState(
        title: 'No orders yet',
        body: 'Your orders will show up here.',
        actionLabel: 'Browse stores',
        onAction: () => taps++,
      ),
    );

    expect(find.text('No orders yet'), findsOneWidget);
    expect(find.text('Your orders will show up here.'), findsOneWidget);

    await tester.tap(find.text('Browse stores'));
    await tester.pump();

    expect(taps, 1);
  });

  testWidgets('empty_state_without_an_action_shows_no_button', (tester) async {
    await pumpApp(tester, const EmptyState(title: 'No orders yet'));

    expect(find.byType(FilledButton), findsNothing);
  });

  testWidgets('error_state_retry_is_tappable_and_labelled', (tester) async {
    final semantics = tester.ensureSemantics();
    var retries = 0;
    await pumpApp(
      tester,
      ErrorState(message: Strings.genericError, onRetry: () => retries++),
    );

    expect(find.text(Strings.genericError), findsOneWidget);
    expect(find.bySemanticsLabel(Strings.retry), findsOneWidget);

    await tester.tap(find.text(Strings.retry));
    await tester.pump();

    expect(retries, 1);
    semantics.dispose();
  });

  testWidgets('error_state_without_a_retry_callback_shows_no_button', (
    tester,
  ) async {
    await pumpApp(tester, ErrorState(message: Strings.genericError));

    expect(find.text(Strings.retry), findsNothing);
  });

  testWidgets('loading_button_ignores_taps_while_loading', (tester) async {
    var calls = 0;
    await pumpApp(
      tester,
      LoadingButton(label: 'Sign in', onPressed: () => calls++, loading: true),
    );

    await tester.tap(find.byType(LoadingButton));
    await tester.pump();
    await tester.tap(find.byType(LoadingButton));
    await tester.pump();

    expect(calls, 0);
  });

  testWidgets('loading_button_calls_back_when_idle', (tester) async {
    var calls = 0;
    await pumpApp(
      tester,
      LoadingButton(label: 'Sign in', onPressed: () => calls++),
    );

    await tester.tap(find.text('Sign in'));
    await tester.pump();

    expect(calls, 1);
  });

  testWidgets('controls_meet_the_48dp_tap_target', (tester) async {
    await pumpApp(
      tester,
      Column(
        children: [
          LoadingButton(label: 'Sign in', onPressed: () {}),
          ErrorState(message: Strings.genericError, onRetry: () {}),
        ],
      ),
    );

    final button = tester.getSize(
      find.descendant(
        of: find.byType(LoadingButton),
        matching: find.byType(FilledButton),
      ),
    );
    final retry = tester.getSize(find.byType(OutlinedButton));

    expect(button.width, greaterThanOrEqualTo(48));
    expect(button.height, greaterThanOrEqualTo(48));
    expect(retry.width, greaterThanOrEqualTo(48));
    expect(retry.height, greaterThanOrEqualTo(48));
  });

  testWidgets('states_do_not_overflow_at_200_percent_text', (tester) async {
    final widgets = <Widget>[
      const Skeleton(),
      const Skeleton(height: 80, width: 200),
      EmptyState(
        title: 'No orders yet, but there will be soon',
        body: 'Your orders will show up here once you place one.',
        actionLabel: 'Browse the stores near you',
        onAction: () {},
      ),
      ErrorState(message: Strings.genericError, onRetry: () {}),
      ErrorState(message: Strings.offlineTitle, onRetry: () {}),
      LoadingButton(label: 'Create my account', onPressed: () {}),
      LoadingButton(
        label: 'Create my account',
        onPressed: () {},
        loading: true,
      ),
    ];

    for (final widget in widgets) {
      await pumpApp(tester, widget, textScale: 2.0);
      expect(
        tester.takeException(),
        isNull,
        reason: 'overflow in ${widget.runtimeType}',
      );
    }
  });

  testWidgets('text_field_shows_label_and_error_text_and_reports_changes', (
    tester,
  ) async {
    final changes = <String>[];
    await pumpApp(
      tester,
      AppTextField(
        label: 'Phone number',
        errorText: 'Enter a valid phone number',
        onChanged: changes.add,
      ),
      textScale: 2.0,
    );

    expect(find.text('Phone number'), findsOneWidget);
    expect(find.text('Enter a valid phone number'), findsOneWidget);
    expect(tester.takeException(), isNull);

    await tester.enterText(find.byType(TextField), '0803');
    expect(changes, ['0803']);
  });

  testWidgets('skeleton_does_not_animate_when_reduced_motion_is_on', (
    tester,
  ) async {
    await pumpApp(tester, const Skeleton(), disableAnimations: true);

    expect(tester.binding.transientCallbackCount, 0);
  });

  testWidgets('skeleton_pulses_when_reduced_motion_is_off', (tester) async {
    await pumpApp(tester, const Skeleton());

    expect(tester.binding.transientCallbackCount, greaterThan(0));
  });
}
