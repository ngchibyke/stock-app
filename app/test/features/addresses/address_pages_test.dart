// ignore_for_file: deprecated_member_use
import 'dart:async';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/paths.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/session/session_store.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/ui/states.dart';
import 'package:market/features/addresses/address.dart';
import 'package:market/features/addresses/address_repository.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/app_harness.dart';
import '../../support/fake_address_repository.dart';
import '../../support/fakes.dart';

/// Opens the app signed in (as `testMe`, "Ada") on the address list.
Future<AppHarness> _open(
  WidgetTester tester,
  FakeAddressRepository repo, {
  SessionStore? store,
  String path = addressesPath,
}) async {
  final h = AppHarness(store: store ?? MemorySessionStore('tok'));
  h.adapter.reply(200, testMe);
  await h.pump(
    tester,
    extra: [addressRepositoryProvider.overrideWithValue(repo)],
  );
  h.container(tester).read(routerProvider).push(path);
  await settle(tester);
  return h;
}

Finder _field(String label) => find.widgetWithText(TextField, label);

String _text(WidgetTester tester, String label) =>
    tester.widget<TextField>(_field(label)).controller!.text;

Future<void> _tapSave(WidgetTester tester) async {
  await tester.ensureVisible(find.text(Strings.saveAddress));
  await tester.tap(find.text(Strings.saveAddress));
  await settle(tester);
}

Future<void> _pickCouncil(WidgetTester tester, AreaCouncil council) async {
  final picker = find.byType(DropdownButtonFormField<AreaCouncil>);
  await tester.ensureVisible(picker);
  await tester.tap(picker);
  await settle(tester);
  await tester.tap(find.text(council.label).last);
  await settle(tester);
}

Future<void> _fillValid(
  WidgetTester tester, {
  String line = '12 Example Street, Wuse 2',
  AreaCouncil council = AreaCouncil.amac,
}) async {
  await tester.enterText(_field(Strings.addressLineLabel), line);
  await _pickCouncil(tester, council);
}

class _RecordingStorage extends FlutterSecureStorage {
  final List<String> writes = [];
  final List<String> deletes = [];

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => 'tok';

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => writes.add(key);

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => deletes.add(key);
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  group('the list', () {
    testWidgets('the_list_has_loading_empty_error_and_data_states', (
      tester,
    ) async {
      final pending = Completer<List<Address>>();
      final repo = FakeAddressRepository()..onList = () => pending.future;
      final h = await _open(tester, repo);

      // Loading: skeleton rows, no empty or error text yet.
      expect(find.byType(Skeleton), findsWidgets);
      expect(find.text(Strings.addAddress), findsNothing);

      // Error, with a retry.
      pending.completeError(ApiError.offline());
      await settle(tester);
      expect(find.text(Strings.offlineMessage), findsOneWidget);
      expect(find.text(Strings.retry), findsOneWidget);

      // Retry: now empty, which offers to add one.
      repo.onList = null;
      await tester.tap(find.text(Strings.retry));
      await settle(tester);
      expect(find.text(Strings.addressesEmptyTitle), findsOneWidget);
      expect(find.text(Strings.addAddress), findsOneWidget);

      // Data: a row with every part, and an add button that stays.
      repo.rows.add(
        testAddress(
          landmark: 'Opposite the mosque',
          postcode: 'FC123456789',
          council: AreaCouncil.bwari,
        ),
      );
      h.container(tester).invalidate(addressesProvider);
      await settle(tester);
      expect(find.text('Ada Obi'), findsOneWidget);
      expect(find.text('Bwari'), findsOneWidget);
      expect(find.text('12 Example Street, Wuse 2'), findsOneWidget);
      expect(find.textContaining('Opposite the mosque'), findsOneWidget);
      expect(find.text('0803 123 4567'), findsOneWidget);
      expect(find.text(Strings.addAddress), findsOneWidget);
      expect(find.text(Strings.addressesEmptyTitle), findsNothing);
    });

    testWidgets('the_list_shows_the_council_label_and_never_a_raw_wire_value', (
      tester,
    ) async {
      final repo = FakeAddressRepository([
        for (final c in AreaCouncil.values)
          testAddress(id: c.wire, council: c, name: 'Person ${c.wire}'),
        testAddress(id: 'odd', council: null, name: 'Odd council'),
      ]);
      await _open(tester, repo);
      // Tall enough that every row is built.
      tester.view.physicalSize = const Size(400, 3000);
      await settle(tester);

      for (final c in AreaCouncil.values) {
        expect(find.text(c.label), findsOneWidget, reason: c.wire);
        // Case-sensitive: a raw wire value is lower case.
        expect(find.text(c.wire), findsNothing, reason: c.wire);
      }
      expect(find.text(Strings.unknownCouncil), findsOneWidget);
    });

    testWidgets('delete_asks_to_confirm_and_cancel_does_nothing', (
      tester,
    ) async {
      final repo = FakeAddressRepository([testAddress()]);
      await _open(tester, repo);
      final delete = find.byTooltip(Strings.deleteAddressFor('Ada Obi'));

      await tester.tap(delete);
      await settle(tester);
      expect(find.text(Strings.deleteConfirmTitle), findsOneWidget);

      await tester.tap(find.text(Strings.cancel));
      await settle(tester);
      expect(find.text(Strings.deleteConfirmTitle), findsNothing);
      expect(repo.removed, isEmpty);
      expect(find.text('Ada Obi'), findsOneWidget);

      await tester.tap(delete);
      await settle(tester);
      await tester.tap(find.text(Strings.delete));
      await settle(tester);

      expect(repo.removed, ['a1']);
      expect(find.text('Ada Obi'), findsNothing);
      expect(find.text(Strings.addressesEmptyTitle), findsOneWidget);
    });

    testWidgets('a_failed_delete_keeps_the_row_and_shows_a_message', (
      tester,
    ) async {
      final repo = FakeAddressRepository([testAddress()])
        ..onRemove = (_) async => throw ApiError(
          status: 500,
          code: 'internal',
          message: 'database exploded',
        );
      await _open(tester, repo);

      await tester.tap(find.byTooltip(Strings.deleteAddressFor('Ada Obi')));
      await settle(tester);
      await tester.tap(find.text(Strings.delete));
      await settle(tester);

      expect(find.text('Ada Obi'), findsOneWidget);
      expect(find.text(Strings.serverDownMessage), findsOneWidget);
      expect(find.textContaining('database exploded'), findsNothing);
    });

    testWidgets('an_offline_delete_says_so_and_keeps_the_row', (tester) async {
      final repo = FakeAddressRepository([testAddress()])
        ..onRemove = (_) async => throw ApiError.offline();
      await _open(tester, repo);

      await tester.tap(find.byTooltip(Strings.deleteAddressFor('Ada Obi')));
      await settle(tester);
      await tester.tap(find.text(Strings.delete));
      await settle(tester);

      expect(find.text('Ada Obi'), findsOneWidget);
      expect(find.text(Strings.offlineMessage), findsOneWidget);
    });

    testWidgets('very_long_names_and_lines_wrap_instead_of_clipping', (
      tester,
    ) async {
      tester.platformDispatcher.textScaleFactorTestValue = 2.0;
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      final name = List.filled(10, 'Chukwuemeka').join(' ');
      final line = List.filled(25, 'Plot 14 Cadastral').join(', ');
      final landmark = List.filled(8, 'Behind the old market').join(' ');
      final repo = FakeAddressRepository([
        testAddress(name: name, line: line, landmark: landmark),
      ]);
      await _open(tester, repo);

      expect(tester.takeException(), isNull);
      for (final long in [name, line]) {
        final widget = tester.widget<Text>(find.text(long));
        expect(widget.maxLines, isNull, reason: long);
        expect(widget.overflow, isNot(TextOverflow.ellipsis));
        // Wrapped onto several lines rather than cut to one.
        expect(
          tester.getSize(find.text(long)).height,
          greaterThan(3 * 16 * 2.0),
          reason: long,
        );
      }
      // The whole address can be scrolled to.
      await tester.scrollUntilVisible(
        find.textContaining(landmark),
        200,
        scrollable: find.byType(Scrollable).first,
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('the_controls_have_labels_and_48dp_targets', (tester) async {
      final semantics = tester.ensureSemantics();
      final repo = FakeAddressRepository([testAddress()]);
      await _open(tester, repo);

      final delete = find.byTooltip(Strings.deleteAddressFor('Ada Obi'));
      expect(delete, findsOneWidget);
      expect(tester.getSize(delete).width, greaterThanOrEqualTo(48));
      expect(tester.getSize(delete).height, greaterThanOrEqualTo(48));
      // Announced with the person it belongs to.
      final node = tester.getSemantics(delete);
      expect(
        '${node.label} ${node.tooltip}',
        contains(Strings.deleteAddressFor('Ada Obi')),
      );
      final add = find.ancestor(
        of: find.text(Strings.addAddress),
        matching: find.byType(FilledButton),
      );
      expect(tester.getSize(add).height, greaterThanOrEqualTo(48));
      semantics.dispose();
    });
  });

  group('the form', () {
    Future<AppHarness> openForm(
      WidgetTester tester,
      FakeAddressRepository repo,
    ) async {
      final h = await _open(tester, repo);
      await tester.tap(find.text(Strings.addAddress));
      await settle(tester);
      expect(find.text(Strings.newAddressTitle), findsOneWidget);
      return h;
    }

    testWidgets('the_form_starts_with_the_persons_name_and_phone', (
      tester,
    ) async {
      await openForm(tester, FakeAddressRepository());

      expect(_text(tester, Strings.recipientNameLabel), 'Ada');
      expect(_text(tester, Strings.phoneLabel), '0803 123 4567');
      expect(_text(tester, Strings.addressLineLabel), '');
      expect(_text(tester, Strings.landmarkLabel), '');
      expect(_text(tester, Strings.postcodeLabel), '');
      expect(find.text(Strings.postcodeHint), findsOneWidget);
    });

    testWidgets('the_form_validates_lengths_like_the_server', (tester) async {
      final repo = FakeAddressRepository();
      await openForm(tester, repo);

      // A longer value is stopped at the field.
      await tester.enterText(_field(Strings.recipientNameLabel), 'n' * 101);
      await tester.enterText(_field(Strings.addressLineLabel), 'a' * 301);
      await tester.enterText(_field(Strings.landmarkLabel), 'l' * 201);
      expect(_text(tester, Strings.recipientNameLabel).length, 100);
      expect(_text(tester, Strings.addressLineLabel).length, 300);
      expect(_text(tester, Strings.landmarkLabel).length, 200);

      // A blank required field shows an error before any call.
      await tester.enterText(_field(Strings.recipientNameLabel), '   ');
      await tester.enterText(_field(Strings.addressLineLabel), '');
      await tester.enterText(_field(Strings.phoneLabel), '123');
      await _tapSave(tester);

      expect(find.text(Strings.recipientRequired), findsOneWidget);
      expect(find.text(Strings.addressLineRequired), findsOneWidget);
      expect(find.text(Strings.phoneInvalid), findsOneWidget);
      expect(find.text(Strings.councilRequired), findsOneWidget);
      expect(repo.created, isEmpty);
    });

    testWidgets('the_form_requires_a_council_and_offers_only_the_six', (
      tester,
    ) async {
      final repo = FakeAddressRepository();
      await openForm(tester, repo);
      await tester.enterText(_field(Strings.addressLineLabel), '1 Road');

      await _tapSave(tester);
      expect(find.text(Strings.councilRequired), findsOneWidget);
      expect(repo.created, isEmpty);

      await tester.tap(find.byType(DropdownButtonFormField<AreaCouncil>));
      await settle(tester);
      final offered = tester
          .widgetList<DropdownMenuItem<AreaCouncil>>(
            find.byType(DropdownMenuItem<AreaCouncil>, skipOffstage: false),
          )
          .map((i) => i.value)
          .toSet();
      expect(offered, AreaCouncil.values.toSet());
      for (final c in AreaCouncil.values) {
        expect(find.text(c.label), findsWidgets, reason: c.wire);
      }

      await tester.tap(find.text(AreaCouncil.kwali.label).last);
      await settle(tester);
      expect(find.text(Strings.councilRequired), findsNothing);
      await _tapSave(tester);
      expect(repo.created.single.areaCouncil, AreaCouncil.kwali);
    });

    testWidgets(
      'a_postcode_is_optional_and_a_malformed_one_shows_the_server_message',
      (tester) async {
        final repo = FakeAddressRepository()
          ..onCreate = (_) async => throw ApiError(
            status: 422,
            code: 'invalid_postcode',
            message: 'postcode must be 11 characters',
          );
        await openForm(tester, repo);
        await _fillValid(tester);
        await tester.enterText(_field(Strings.postcodeLabel), 'XX123');
        await _tapSave(tester);

        expect(find.text(Strings.postcodeInvalid), findsOneWidget);
        expect(find.textContaining('must be 11'), findsNothing);
        expect(repo.created.single.postcode, 'XX123');

        // Blank is fine: no postcode is sent.
        repo.onCreate = null;
        await tester.enterText(_field(Strings.postcodeLabel), '  ');
        await _tapSave(tester);
        expect(repo.created.length, 2);
        expect(repo.created.last.toJson().containsKey('postcode'), isFalse);
        expect(find.text(Strings.newAddressTitle), findsNothing);
      },
    );

    testWidgets('the_server_error_messages_are_shown_in_plain_words', (
      tester,
    ) async {
      final repo = FakeAddressRepository();
      await openForm(tester, repo);
      await _fillValid(tester);

      final cases = {
        'outside_service_area': Strings.outsideServiceArea,
        'invalid_postcode': Strings.postcodeInvalid,
        'too_many_addresses': Strings.tooManyAddresses,
        'invalid_phone': Strings.phoneInvalid,
        'validation_failed': Strings.checkDetails,
      };
      for (final entry in cases.entries) {
        repo.onCreate = (_) async => throw ApiError(
          status: 422,
          code: entry.key,
          message: 'raw server words for ${entry.key}',
        );
        await _tapSave(tester);
        expect(find.text(entry.value), findsOneWidget, reason: entry.key);
        expect(find.textContaining('raw server words'), findsNothing);
        expect(find.text(Strings.newAddressTitle), findsOneWidget);
      }
    });

    testWidgets('a_validation_error_naming_a_field_is_shown_at_that_field', (
      tester,
    ) async {
      final repo = FakeAddressRepository()
        ..onCreate = (_) async => throw ApiError(
          status: 422,
          code: 'validation_failed',
          message: 'landmark must be at most 200 characters',
        );
      await openForm(tester, repo);
      await _fillValid(tester);
      await _tapSave(tester);

      expect(find.text(Strings.landmarkInvalid), findsOneWidget);
      expect(find.textContaining('at most 200'), findsNothing);
    });

    testWidgets('an_offline_save_says_so_and_can_be_retried', (tester) async {
      final repo = FakeAddressRepository()
        ..onCreate = (_) async => throw ApiError.offline();
      await openForm(tester, repo);
      await _fillValid(tester);
      await _tapSave(tester);

      expect(find.text(Strings.offlineMessage), findsOneWidget);
      repo.onCreate = null;
      await tester.tap(find.text(Strings.retry));
      await settle(tester);
      expect(repo.created.length, 2);
      expect(find.text(Strings.newAddressTitle), findsNothing);
    });

    testWidgets('saving_twice_sends_one_request', (tester) async {
      final done = Completer<Address>();
      final repo = FakeAddressRepository()..onCreate = (_) => done.future;
      await openForm(tester, repo);
      await _fillValid(tester);

      await tester.ensureVisible(find.text(Strings.saveAddress));
      await tester.tap(find.text(Strings.saveAddress));
      await tester.pump();
      // The button now shows progress; tap it again, twice.
      final button = find.byType(FilledButton);
      await tester.tap(button, warnIfMissed: false);
      await tester.pump();
      await tester.tap(button, warnIfMissed: false);
      await settle(tester);

      expect(repo.created.length, 1);

      done.complete(testAddress(id: 'made'));
      await settle(tester);
      expect(repo.created.length, 1);
    });

    testWidgets('success_pops_back_and_the_new_address_is_at_the_top', (
      tester,
    ) async {
      final repo = FakeAddressRepository([
        testAddress(id: 'old', name: 'Old Person', line: '1 Old Road'),
      ]);
      await openForm(tester, repo);
      await tester.enterText(_field(Strings.recipientNameLabel), 'Newcomer');
      await _fillValid(tester, line: '9 New Road');
      await _tapSave(tester);

      expect(find.text(Strings.newAddressTitle), findsNothing);
      expect(find.text('Newcomer'), findsOneWidget);
      expect(find.text('Old Person'), findsOneWidget);
      expect(
        tester.getTopLeft(find.text('Newcomer')).dy,
        lessThan(tester.getTopLeft(find.text('Old Person')).dy),
      );
      // No full reload: the list was asked for once.
      expect(repo.listCalls, 1);
      expect(repo.created.single.recipientName, 'Newcomer');
      expect(repo.created.single.addressLine, '9 New Road');
    });

    testWidgets(
      'the_form_stays_usable_with_the_keyboard_up_and_at_200_percent_text',
      (tester) async {
        tester.platformDispatcher.textScaleFactorTestValue = 2.0;
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        final repo = FakeAddressRepository();
        await openForm(tester, repo);

        // Focus the last field, then 300 of the 1000 logical pixels are taken
        // by the keyboard, as when it slides up.
        await tester.tap(_field(Strings.postcodeLabel));
        await settle(tester);
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        addTearDown(tester.view.resetViewInsets);
        await settle(tester);
        await tester.pump(const Duration(seconds: 1));

        // Where the person types (not the helper text under it) is above the
        // keyboard.
        final rect = tester.getRect(
          find.descendant(
            of: _field(Strings.postcodeLabel),
            matching: find.byType(EditableText),
          ),
        );
        expect(rect.bottom, lessThanOrEqualTo(700));
        expect(rect.top, greaterThanOrEqualTo(0));
        expect(tester.takeException(), isNull);

        // Every field and the council picker can still be reached.
        for (final label in [
          Strings.recipientNameLabel,
          Strings.phoneLabel,
          Strings.addressLineLabel,
          Strings.landmarkLabel,
          Strings.councilLabel,
        ]) {
          await tester.ensureVisible(find.text(label).first);
          expect(find.text(label), findsWidgets, reason: label);
        }
        expect(tester.takeException(), isNull);
      },
    );

    for (final width in [400.0, 320.0]) {
      testWidgets(
        'the_council_menu_and_closed_field_do_not_clip_at_200_percent_on_${width.toInt()}_wide',
        (tester) async {
          tester.platformDispatcher.textScaleFactorTestValue = 2.0;
          addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
          await openForm(tester, FakeAddressRepository());
          tester.view.physicalSize = Size(width, 1000);
          await settle(tester);

          // Open menu: every item shows its whole label inside its own box.
          final picker = find.byType(DropdownButtonFormField<AreaCouncil>);
          await tester.ensureVisible(picker);
          await tester.tap(picker);
          await settle(tester);
          expect(tester.takeException(), isNull);
          for (final c in AreaCouncil.values) {
            final item = find
                .ancestor(
                  of: find.text(c.label).last,
                  matching: find.byType(DropdownMenuItem<AreaCouncil>),
                )
                .last;
            final textBox = tester.getRect(find.text(c.label).last);
            final itemBox = tester.getRect(item);
            expect(
              itemBox.height,
              greaterThanOrEqualTo(textBox.height),
              reason: '${c.wire} item is shorter than its text',
            );
            expect(
              textBox.right,
              lessThanOrEqualTo(itemBox.right + 0.5),
              reason: '${c.wire} text runs past its item',
            );
            expect(textBox.left, greaterThanOrEqualTo(itemBox.left - 0.5));
            expect(itemBox.top, lessThanOrEqualTo(textBox.top + 0.5));
            expect(itemBox.bottom, greaterThanOrEqualTo(textBox.bottom - 0.5));
          }

          // Select the longest label; the closed field shows all of it.
          await tester.tap(find.text(AreaCouncil.amac.label).last);
          await settle(tester);
          expect(tester.takeException(), isNull);
          final closed = tester.getRect(
            find
                .descendant(
                  of: picker,
                  matching: find.text(AreaCouncil.amac.label),
                )
                .first,
          );
          final field = tester.getRect(picker);
          expect(field.contains(closed.topLeft), isTrue);
          expect(field.contains(closed.bottomRight), isTrue);

          // And it opens again with the selection in place.
          await tester.tap(picker);
          await settle(tester);
          expect(tester.takeException(), isNull);
        },
      );
    }

    testWidgets('the_form_survives_a_very_long_name_and_line_at_200_percent', (
      tester,
    ) async {
      tester.platformDispatcher.textScaleFactorTestValue = 2.0;
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      await openForm(tester, FakeAddressRepository());

      await tester.enterText(
        _field(Strings.recipientNameLabel),
        List.filled(12, 'Chukwuemeka').join(' '),
      );
      await tester.enterText(
        _field(Strings.addressLineLabel),
        List.filled(30, 'Plot 14 Cadastral').join(', '),
      );
      await settle(tester);

      final line = tester.widget<TextField>(_field(Strings.addressLineLabel));
      expect(line.maxLines, greaterThan(1));
      expect(tester.takeException(), isNull);
    });

    testWidgets('the_fields_have_labels_and_48dp_targets', (tester) async {
      final semantics = tester.ensureSemantics();
      await openForm(tester, FakeAddressRepository());

      for (final label in [
        Strings.recipientNameLabel,
        Strings.phoneLabel,
        Strings.addressLineLabel,
        Strings.landmarkLabel,
        Strings.postcodeLabel,
      ]) {
        expect(
          find.bySemanticsLabel(RegExp(RegExp.escape(label))),
          findsWidgets,
        );
        expect(
          tester.getSize(_field(label)).height,
          greaterThanOrEqualTo(48),
          reason: label,
        );
      }
      expect(
        tester
            .getSize(find.byType(DropdownButtonFormField<AreaCouncil>))
            .height,
        greaterThanOrEqualTo(48),
      );
      expect(
        tester.getSize(find.byType(FilledButton)).height,
        greaterThanOrEqualTo(48),
      );
      semantics.dispose();
    });

    testWidgets(
      'a_deep_link_to_the_form_opens_it_and_saving_returns_to_the_list',
      (tester) async {
        final repo = FakeAddressRepository();
        final h = AppHarness(store: MemorySessionStore('tok'));
        h.adapter.reply(200, testMe);
        await h.pump(
          tester,
          extra: [addressRepositoryProvider.overrideWithValue(repo)],
        );
        h.container(tester).read(routerProvider).go(newAddressPath);
        await settle(tester);
        expect(find.text(Strings.newAddressTitle), findsOneWidget);

        await _fillValid(tester);
        await _tapSave(tester);
        expect(find.text(Strings.newAddressTitle), findsNothing);
        expect(find.text('Ada'), findsOneWidget);
        expect(find.text(Strings.addAddress), findsOneWidget);
      },
    );
  });

  testWidgets('addresses_are_never_written_to_disk_or_logs', (tester) async {
    final printed = <String>[];
    final originalDebugPrint = debugPrint;
    debugPrint = (String? message, {int? wrapWidth}) =>
        printed.add(message ?? '');
    addTearDown(() => debugPrint = originalDebugPrint);

    await runZoned(
      () async {
        final secure = _RecordingStorage();
        final store = SecureSessionStore(secure);
        final repo = FakeAddressRepository([testAddress(id: 'first')]);
        final h = await _open(tester, repo, store: store);

        // Add one, delete one.
        await tester.tap(find.text(Strings.addAddress));
        await settle(tester);
        await tester.enterText(_field(Strings.landmarkLabel), 'Secret place');
        await _fillValid(tester, line: '77 Private Close');
        await _tapSave(tester);
        await tester.tap(find.byTooltip(Strings.deleteAddressFor('Ada Obi')));
        await settle(tester);
        await tester.tap(find.text(Strings.delete));
        await settle(tester);

        // The held list goes with the page, and with the session.
        expect(h.container(tester).exists(addressesProvider), isTrue);

        expect(secure.writes, isEmpty);
        expect(secure.deletes, isEmpty);
        final prefs = await SharedPreferences.getInstance();
        expect(prefs.getKeys(), isEmpty);
      },
      zoneSpecification: ZoneSpecification(
        print: (self, parent, zone, line) => printed.add(line),
      ),
    );

    expect(printed, isEmpty);
  });

  testWidgets('leaving_the_list_drops_what_it_held', (tester) async {
    final repo = FakeAddressRepository([testAddress()]);
    final h = await _open(tester, repo);
    final container = h.container(tester);
    expect(container.exists(addressesProvider), isTrue);

    container.read(routerProvider).pop();
    await settle(tester);

    expect(container.exists(addressesProvider), isFalse);
  });

  testWidgets('signing_out_drops_the_held_addresses', (tester) async {
    final repo = FakeAddressRepository([testAddress()]);
    final h = await _open(tester, repo);
    final container = h.container(tester);
    expect(find.text('Ada Obi'), findsOneWidget);

    h.adapter.reply(200, <String, dynamic>{});
    unawaited(container.read(sessionProvider.notifier).signOut());
    await settle(tester);

    expect(find.text('Ada Obi'), findsNothing);
    expect(find.text(Strings.signInTitle), findsOneWidget);
    expect(container.exists(addressesProvider), isFalse);
  });
}
