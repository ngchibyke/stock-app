import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/strings.dart';
import 'package:market/features/privacy/privacy_models.dart';
import 'package:market/features/privacy/privacy_pages.dart';
import 'package:market/features/privacy/privacy_strings.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/app_harness.dart';
import '../../support/fake_privacy.dart';
import '../../support/fakes.dart';

Future<AppHarness> _account(WidgetTester tester) async {
  tester.view.physicalSize = const Size(400, 1800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final h = AppHarness(store: MemorySessionStore('tok'));
  h.adapter.reply(200, testMe);
  await h.pump(tester);
  await tester.tap(find.text(Strings.tabAccount));
  await settle(tester);
  return h;
}

Future<void> _open(WidgetTester tester) async {
  final row = find.text(PrivacyStrings.rowLabel);
  await tester.ensureVisible(row);
  await tester.pump();
  await tester.tap(row);
  await settle(tester);
}

Future<void> _type(WidgetTester tester, String text) async {
  await tester.enterText(find.byType(TextField), text);
  await tester.pump();
}

Future<void> _press(WidgetTester tester, String label) async {
  final f = find.text(label);
  await tester.ensureVisible(f.first);
  await tester.pump();
  await tester.tap(f.first);
  await settle(tester);
}

void main() {
  group('download my data', () {
    testWidgets('asking_needs_the_password_and_then_says_it_is_being_made', (
      tester,
    ) async {
      final h = await _account(tester);
      await _open(tester);
      expect(find.text(PrivacyStrings.downloadBody), findsOneWidget);
      // No password, nothing is asked.
      await _press(tester, PrivacyStrings.downloadAsk);
      expect(h.privacy.passwords, isEmpty);
      await _type(tester, 'my-password-1');
      await _press(tester, PrivacyStrings.downloadAsk);
      expect(h.privacy.passwords, ['my-password-1']);
      expect(find.text(PrivacyStrings.downloadPending), findsOneWidget);
      // The password is not left in the field or on screen.
      expect(find.text('my-password-1'), findsNothing);
    });

    testWidgets('a_ready_file_is_handed_to_the_share_sheet_and_not_kept', (
      tester,
    ) async {
      final h = await _account(tester);
      h.privacy.export = ExportStatus(
        state: ExportState.ready,
        expiresAt: DateTime(2026, 12, 3),
      );
      await _open(tester);
      expect(find.text(PrivacyStrings.downloadReady), findsOneWidget);
      expect(
        find.text(PrivacyStrings.downloadUntil('3 Dec 2026')),
        findsOneWidget,
      );
      await _press(tester, PrivacyStrings.downloadSave);
      expect(h.sharer.shared.single.$1, 'my-data.json');
      expect(h.sharer.shared.single.$2, h.privacy.file);
      expect(find.textContaining('Ada'), findsNothing);
    });

    testWidgets('failed_and_expired_files_can_be_asked_for_again', (
      tester,
    ) async {
      final h = await _account(tester);
      h.privacy.export = const ExportStatus(state: ExportState.expired);
      await _open(tester);
      expect(find.text(PrivacyStrings.downloadExpired), findsOneWidget);
      expect(find.text(PrivacyStrings.downloadAsk), findsWidgets);
    });

    testWidgets('a_wrong_password_and_the_daily_limit_say_so_in_plain_words', (
      tester,
    ) async {
      final h = await _account(tester);
      await _open(tester);
      h.privacy.exportError = ApiError(
        status: 401,
        code: 'invalid_credentials',
        message: 'raw server text',
      );
      await _type(tester, 'wrong-wrong-1');
      await _press(tester, PrivacyStrings.downloadAsk);
      expect(find.text(PrivacyStrings.wrongPassword), findsOneWidget);
      expect(find.textContaining('raw server'), findsNothing);
      h.privacy.exportError = ApiError(
        status: 429,
        code: 'export_rate_limited',
        message: 'x',
      );
      await _press(tester, PrivacyStrings.downloadAsk);
      expect(find.text(PrivacyStrings.downloadLimited), findsOneWidget);
    });
  });

  group('delete my account', () {
    Future<AppHarness> openDelete(WidgetTester tester) async {
      final h = await _account(tester);
      await _open(tester);
      await tester.tap(
        find.widgetWithText(OutlinedButton, PrivacyStrings.deleteTitle),
      );
      await settle(tester);
      return h;
    }

    testWidgets('it_explains_what_goes_and_what_stays_before_asking', (
      tester,
    ) async {
      await openDelete(tester);
      expect(find.text(PrivacyStrings.deleteIntro), findsOneWidget);
      expect(find.text(PrivacyStrings.deleteKept), findsOneWidget);
      expect(find.byType(TextField), findsOneWidget);
    });

    testWidgets('something_open_is_listed_in_plain_words_and_nothing_changes', (
      tester,
    ) async {
      final h = await openDelete(tester);
      h.privacy.deleteError = blocked(['open_order', 'owns_store']);
      await _type(tester, 'my-password-1');
      await _press(tester, PrivacyStrings.deleteButton);
      expect(find.text(PrivacyStrings.deleteBlockedTitle), findsOneWidget);
      expect(find.text(PrivacyStrings.blocker('open_order')), findsOneWidget);
      expect(find.text(PrivacyStrings.blocker('owns_store')), findsOneWidget);
      expect(find.text('open_order'), findsNothing);
      // Still in the app, not waiting for deletion.
      expect(find.byType(DeletionPendingPage), findsNothing);
    });

    testWidgets('a_wrong_password_is_refused_without_a_leak', (tester) async {
      final h = await openDelete(tester);
      h.privacy.deleteError = ApiError(
        status: 401,
        code: 'invalid_credentials',
        message: 'raw',
      );
      await _type(tester, 'wrong-wrong-1');
      await _press(tester, PrivacyStrings.deleteButton);
      expect(find.text(PrivacyStrings.wrongPassword), findsOneWidget);
      expect(find.byType(DeletionPendingPage), findsNothing);
    });

    testWidgets('asking_moves_to_the_waiting_screen_and_keeping_returns', (
      tester,
    ) async {
      final h = await openDelete(tester);
      await _type(tester, 'my-password-1');
      await _press(tester, PrivacyStrings.deleteButton);
      expect(find.byType(DeletionPendingPage), findsOneWidget);
      expect(find.text(PrivacyStrings.pendingTitle), findsOneWidget);
      expect(
        find.text(PrivacyStrings.pendingBody('26 Dec 2026')),
        findsOneWidget,
      );
      // Nothing else of the app is reachable behind it.
      expect(find.text(Strings.tabAccount), findsNothing);

      h.adapter.reply(200, testMe);
      await _press(tester, PrivacyStrings.keepAccount);
      expect(h.privacy.cancelled, 1);
      expect(find.byType(DeletionPendingPage), findsNothing);
      expect(find.text(Strings.tabAccount), findsWidgets);
    });

    testWidgets('keeping_fails_in_plain_words_and_stays_on_the_screen', (
      tester,
    ) async {
      final h = await openDelete(tester);
      await _type(tester, 'my-password-1');
      await _press(tester, PrivacyStrings.deleteButton);
      h.privacy.deleteError = ApiError.offline();
      await _press(tester, PrivacyStrings.keepAccount);
      expect(find.text(PrivacyStrings.keepFailed), findsOneWidget);
      expect(find.byType(DeletionPendingPage), findsOneWidget);
    });
  });

  testWidgets('text_at_200_percent_does_not_overflow', (tester) async {
    tester.platformDispatcher.textScaleFactorTestValue = 2.0;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    await _account(tester);
    await _open(tester);
    expect(tester.takeException(), isNull);
  });
}
