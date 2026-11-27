import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/strings.dart';
import 'package:market/features/addresses/address_list_page.dart';
import 'package:market/features/buy/buy_models.dart';
import 'package:market/features/buy/inbox.dart';
import 'package:market/features/buy/orders_pages.dart';
import 'package:market/features/chat/chat_pages.dart';
import 'package:market/features/privacy/privacy_pages.dart';
import 'package:material_ui/material_ui.dart';

import 'support/app_harness.dart';
import 'support/fake_chat.dart';
import 'support/fakes.dart';
import 'support/pump_buy.dart';

/// Walks the main screens and fails on a tap target that is too small or has no label.
/// This is the part of the accessibility pass a test can do; TalkBack and VoiceOver on a
/// phone are in the device checklist.
///
/// Semantics must be on **before** the first frame, or there is no tree and every guideline
/// passes for nothing, so each test calls this first and disposes the handle at the end.
Future<void> _meetsGuidelines(WidgetTester tester) async {
  expect(
    SemanticsBinding.instance.semanticsEnabled,
    isTrue,
    reason: 'turn semantics on before pumping the screen',
  );
  // A screen with nothing to tap would pass for nothing.
  var tappable = 0;
  void count(SemanticsNode n) {
    if (n.getSemanticsData().hasAction(SemanticsAction.tap)) tappable++;
    n.visitChildren((c) {
      count(c);
      return true;
    });
  }

  count(
    tester.binding.renderViews.first.owner!.semanticsOwner!.rootSemanticsNode!,
  );
  expect(
    tappable,
    greaterThan(0),
    reason: 'the screen has no tappable control',
  );
  await expectLater(tester, meetsGuideline(androidTapTargetGuideline));
  await expectLater(tester, meetsGuideline(iOSTapTargetGuideline));
  await expectLater(tester, meetsGuideline(labeledTapTargetGuideline));
}

void main() {
  testWidgets('the_check_itself_catches_a_tiny_unlabelled_button', (
    tester,
  ) async {
    // A control: if this stopped failing, the walk below would prove nothing.
    // One logical pixel per physical pixel, as the screens below are pumped.
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    final handle = tester.ensureSemantics();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                // Too small but labelled: the size guideline must fail.
                Semantics(
                  button: true,
                  label: 'Tiny',
                  onTap: () {},
                  child: const SizedBox(width: 16, height: 16),
                ),
                const SizedBox(height: 40),
                // Big enough but unlabelled: the label guideline must fail.
                Semantics(
                  button: true,
                  onTap: () {},
                  child: const SizedBox(width: 80, height: 80),
                ),
              ],
            ),
          ),
        ),
      ),
    );
    await expectLater(
      () => expectLater(tester, meetsGuideline(androidTapTargetGuideline)),
      throwsA(isA<TestFailure>()),
    );
    await expectLater(
      () => expectLater(tester, meetsGuideline(labeledTapTargetGuideline)),
      throwsA(isA<TestFailure>()),
    );
    handle.dispose();
  });

  testWidgets('sign_in', (tester) async {
    final handle = tester.ensureSemantics();
    final h = AppHarness();
    await h.pump(tester);
    expect(find.text(Strings.signInAction), findsWidgets);
    await _meetsGuidelines(tester);
    handle.dispose();
  });

  testWidgets('account_page', (tester) async {
    final handle = tester.ensureSemantics();
    tester.view.physicalSize = const Size(400, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final h = AppHarness(store: MemorySessionStore('tok'));
    h.adapter.reply(200, testMe);
    await h.pump(tester);
    await tester.tap(find.text(Strings.tabAccount));
    await settle(tester);
    await _meetsGuidelines(tester);
    await tester.pumpWidget(const SizedBox());
    handle.dispose();
  });

  // Each screen is shown with data, so its rows and buttons are what is checked.
  final screens = <(String, Widget, void Function(BuyEnv))>[
    ('privacy_page', const PrivacyPage(), (_) {}),
    ('delete_account_page', const DeleteAccountPage(), (_) {}),
    (
      'messages_page',
      const MessagesPage(),
      (e) => e.chat.conversationRows = [fakeConversation(unread: 2)],
    ),
    (
      'orders_page',
      const OrdersPage(),
      (e) => e.orders.orderRows = [
        BuyOrder(
          id: 'o1',
          status: 'paid',
          totalKobo: 1650000,
          createdAt: DateTime(2026, 11, 20),
          lines: const [
            BuyLine(name: 'Ankara dress', quantity: 1, unitPriceKobo: 1500000),
          ],
        ),
      ],
    ),
    (
      'notices_page',
      const InboxPage(),
      (e) => e.inbox.box = Inbox(
        unread: 1,
        items: [
          Notice(
            id: 'n1',
            kind: 'order_paid',
            title: 'Order paid',
            body: 'Your order is paid.',
            createdAt: DateTime(2026, 11, 26),
            read: false,
            orderId: 'o1',
          ),
        ],
      ),
    ),
    ('addresses_page', const AddressListPage(), (_) {}),
  ];
  for (final (name, page, seed) in screens) {
    testWidgets(name, (tester) async {
      final handle = tester.ensureSemantics();
      final env = BuyEnv();
      seed(env);
      await pumpBuy(tester, page, env);
      await tester.pump(const Duration(milliseconds: 300));
      await _meetsGuidelines(tester);
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(minutes: 1));
      handle.dispose();
    });
  }
}
