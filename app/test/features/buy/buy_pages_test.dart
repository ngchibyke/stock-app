import 'dart:typed_data';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:market/core/links/external_link.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/core/ui/states.dart';
import 'package:market/features/buy/buy_models.dart';
import 'package:market/features/chat/chat_strings.dart';
import 'package:market/features/buy/buy_strings.dart';
import 'package:market/features/buy/checkout_page.dart';
import 'package:market/features/buy/dispute_pages.dart';
import 'package:market/features/buy/inbox.dart';
import 'package:market/features/buy/orders_pages.dart';
import 'package:market/features/buy/payment_page.dart';
import 'package:market/features/sell/photos/photo_source.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/fake_buy.dart';
import '../../support/pump_buy.dart';
import '../../support/pump_sell.dart' show settleFrames;

Future<void> _tap(WidgetTester tester, Finder f) async {
  await tester.ensureVisible(f);
  await tester.pump();
  await tester.tap(f);
}

DeliveryOption _opt(String id, BuyMethod m, int fee) =>
    DeliveryOption(id: id, method: m, feeKobo: fee, promisedDate: '2026-12-02');

StoreDelivery _oneBranch({List<ShortLine> short = const []}) => StoreDelivery(
  storeId: 's1',
  storeName: 'Ada Fashion',
  short: short,
  groups: [
    DeliveryGroup(
      groupId: 'g1',
      branchName: 'Garki',
      options: [
        _opt('c1', BuyMethod.courier, 150000),
        _opt('p1', BuyMethod.collect, 0),
      ],
    ),
  ],
);

BuyOrder _order(String id, String status, {String? dispute}) => BuyOrder(
  id: id,
  status: status,
  totalKobo: 1650000,
  createdAt: DateTime(2026, 11, 20),
  disputeId: dispute,
  lines: const [
    BuyLine(name: 'Ankara dress', quantity: 1, unitPriceKobo: 1500000),
  ],
);

Uint8List _jpeg() =>
    Uint8List.fromList(img.encodeJpg(img.Image(width: 300, height: 200)));

class _Photos implements PhotoSource {
  @override
  Future<Uint8List?> fromCamera(
    BuildContext context, {
    required int shotIndex,
  }) async => _jpeg();
  @override
  Future<Uint8List?> fromGallery() async => _jpeg();
}

void main() {
  group('checkout', () {
    testWidgets(
      'address_then_options_then_review_then_pay_sends_the_chosen_options',
      (tester) async {
        final env = BuyEnv();
        env.checkout.stores = [_oneBranch()];
        await pumpBuy(tester, const CheckoutPage(), env);
        await settleFrames(tester, 2);
        await _tap(tester, find.text('Ada Obi'));
        await settleFrames(tester, 3);
        expect(env.checkout.optionCalls, ['a1']);
        expect(find.text('Ada Fashion'), findsOneWidget);
        expect(find.text(BuyStrings.sentFrom('Garki')), findsOneWidget);
        expect(find.text(BuyStrings.courier), findsOneWidget);
        expect(find.text(BuyStrings.collect), findsOneWidget);
        await _tap(tester, find.text(BuyStrings.collect));
        await tester.pump();
        expect(find.text(BuyStrings.collectNote), findsOneWidget);
        await _tap(tester, find.text(BuyStrings.courier));
        await tester.pump();
        await _tap(
          tester,
          find.widgetWithText(LoadingButton, BuyStrings.reviewTitle),
        );
        await settleFrames(tester, 3);
        // Goods from the cart plus the delivery fee.
        expect(find.text('₦15,000'), findsWidgets);
        expect(find.text('₦1,500'), findsOneWidget);
        expect(find.text('₦16,500'), findsOneWidget);
        await _tap(tester, find.widgetWithText(LoadingButton, BuyStrings.pay));
        await settleFrames(tester, 3);
        final sent = env.checkout.created.single;
        expect(sent['method'], 'courier');
        expect(sent['address'], 'a1');
        expect(sent['groups'], {'g1': 'c1'});
        expect(sent['key'], matches(RegExp(r'^[a-f0-9]{32}$')));
        expect(find.byType(PaymentPage), findsOneWidget);
      },
    );

    testWidgets(
      'a_stale_option_refreshes_the_choices_and_a_retry_keeps_the_same_key',
      (tester) async {
        final env = BuyEnv();
        env.checkout.stores = [_oneBranch()];
        env.checkout.failCreate = apiError('allocation_changed');
        await pumpBuy(tester, const CheckoutPage(), env);
        await settleFrames(tester, 2);
        await _tap(tester, find.text('Ada Obi'));
        await settleFrames(tester, 3);
        await _tap(
          tester,
          find.widgetWithText(LoadingButton, BuyStrings.reviewTitle),
        );
        await settleFrames(tester, 3);
        await _tap(tester, find.widgetWithText(LoadingButton, BuyStrings.pay));
        await settleFrames(tester, 4);
        // Back on the options with a fresh quote and the plain explanation.
        expect(env.checkout.optionCalls, ['a1', 'a1']);
        expect(find.text(BuyStrings.optionsChanged), findsOneWidget);
        expect(find.byType(PaymentPage), findsNothing);
        await _tap(
          tester,
          find.widgetWithText(LoadingButton, BuyStrings.reviewTitle),
        );
        await settleFrames(tester, 3);
        await _tap(tester, find.widgetWithText(LoadingButton, BuyStrings.pay));
        await settleFrames(tester, 3);
        expect(env.checkout.created, hasLength(2));
        expect(
          env.checkout.created[1]['key'],
          env.checkout.created[0]['key'],
          reason: 'the same choices are the same checkout',
        );
        expect(find.byType(PaymentPage), findsOneWidget);
      },
    );

    testWidgets('a_short_line_blocks_review_and_says_how_many_are_left', (
      tester,
    ) async {
      final env = BuyEnv();
      env.checkout.stores = [
        _oneBranch(
          short: const [ShortLine(variantId: 'v1', wanted: 3, available: 1)],
        ),
      ];
      await pumpBuy(tester, const CheckoutPage(), env);
      await settleFrames(tester, 2);
      await _tap(tester, find.text('Ada Obi'));
      await settleFrames(tester, 3);
      expect(find.text(BuyStrings.onlyAvailable(1)), findsOneWidget);
      expect(find.text(BuyStrings.changeCart), findsOneWidget);
      expect(
        tester
            .widget<LoadingButton>(
              find.widgetWithText(LoadingButton, BuyStrings.reviewTitle),
            )
            .onPressed,
        isNull,
      );
    });

    testWidgets('goods_from_two_branches_are_shown_plainly', (tester) async {
      final env = BuyEnv();
      env.checkout.stores = [
        StoreDelivery(
          storeId: 's1',
          storeName: 'Ada Fashion',
          short: const [],
          groups: [
            DeliveryGroup(
              groupId: 'g1',
              branchName: 'Garki',
              options: [_opt('c1', BuyMethod.courier, 100)],
            ),
            DeliveryGroup(
              groupId: 'g2',
              branchName: 'Wuse',
              options: [_opt('c2', BuyMethod.courier, 200)],
            ),
          ],
        ),
      ];
      await pumpBuy(tester, const CheckoutPage(), env);
      await settleFrames(tester, 2);
      await _tap(tester, find.text('Ada Obi'));
      await settleFrames(tester, 3);
      expect(find.text(BuyStrings.splitNote), findsOneWidget);
      expect(find.text(BuyStrings.sentFrom('Garki')), findsOneWidget);
      expect(find.text(BuyStrings.sentFrom('Wuse')), findsOneWidget);
      await _tap(
        tester,
        find.widgetWithText(LoadingButton, BuyStrings.reviewTitle),
      );
      await settleFrames(tester, 3);
      await _tap(tester, find.widgetWithText(LoadingButton, BuyStrings.pay));
      await settleFrames(tester, 3);
      expect(env.checkout.created.single['groups'], {'g1': 'c1', 'g2': 'c2'});
    });

    testWidgets(
      'no_saved_address_offers_to_add_one_and_a_failure_offers_retry',
      (tester) async {
        final env = BuyEnv();
        env.addresses.rows.clear();
        await pumpBuy(tester, const CheckoutPage(), env);
        await settleFrames(tester, 2);
        expect(find.text(BuyStrings.noAddresses), findsOneWidget);
        expect(find.text(BuyStrings.addAddress), findsOneWidget);

        final env2 = BuyEnv();
        env2.checkout.failOptions = apiError('server_down', status: 500);
        await pumpBuy(tester, const CheckoutPage(), env2);
        await settleFrames(tester, 2);
        await _tap(tester, find.text('Ada Obi'));
        await settleFrames(tester, 3);
        expect(find.byType(ErrorState), findsOneWidget);
      },
    );
  });

  group('payment', () {
    CreatedCheckout checkout([String url = 'https://pay.test/x']) =>
        CreatedCheckout(checkoutId: 'c1', paymentUrl: url, totalKobo: 1000);

    testWidgets('the_app_shows_paid_only_when_the_server_says_so', (
      tester,
    ) async {
      final env = BuyEnv();
      env.checkout.statuses = const [
        CheckoutStatus(status: 'pending', orderIds: []),
        CheckoutStatus(status: 'pending', orderIds: []),
        CheckoutStatus(status: 'paid', orderIds: ['o1']),
      ];
      bool? finished;
      await pumpBuy(
        tester,
        PaymentPage(checkout: checkout(), onFinished: (p) => finished = p),
        env,
      );
      await settleFrames(tester, 2);
      expect(find.text('PAY https://pay.test/x'), findsOneWidget);
      await tester.tap(find.text('provider returns'));
      await settleFrames(tester, 1);
      // Returned from the provider, but nothing is claimed yet.
      expect(find.text(BuyStrings.paidTitle), findsNothing);
      expect(find.text(BuyStrings.checking), findsOneWidget);
      await settleFrames(tester, 4);
      expect(find.text(BuyStrings.paidTitle), findsOneWidget);
      await tester.tap(find.text(BuyStrings.viewOrders));
      expect(finished, isTrue);
    });

    testWidgets('an_expired_checkout_says_the_buyer_was_not_charged', (
      tester,
    ) async {
      final env = BuyEnv();
      env.checkout.statuses = const [
        CheckoutStatus(status: 'expired', orderIds: []),
      ];
      bool? finished;
      await pumpBuy(
        tester,
        PaymentPage(checkout: checkout(), onFinished: (p) => finished = p),
        env,
      );
      await tester.pump();
      await tester.tap(find.text('provider returns'));
      await settleFrames(tester, 3);
      expect(find.text(BuyStrings.failedTitle), findsOneWidget);
      expect(find.text(BuyStrings.paidTitle), findsNothing);
      await tester.tap(find.text(BuyStrings.backToCart));
      expect(finished, isFalse);
    });

    testWidgets('still_pending_after_the_wait_never_claims_paid', (
      tester,
    ) async {
      final env = BuyEnv();
      await pumpBuy(
        tester,
        PaymentPage(checkout: checkout(), onFinished: (_) {}),
        env,
      );
      await tester.pump();
      await tester.tap(find.text('provider returns'));
      await settleFrames(tester, 8);
      expect(find.text(BuyStrings.pendingTitle), findsOneWidget);
      expect(find.text(BuyStrings.paidTitle), findsNothing);
      expect(env.checkout.statusCalls, 4);
    });

    testWidgets('closing_the_payment_early_asks_the_server_too', (
      tester,
    ) async {
      final env = BuyEnv();
      env.checkout.statuses = const [
        CheckoutStatus(status: 'paid', orderIds: ['o1']),
      ];
      await pumpBuy(
        tester,
        PaymentPage(checkout: checkout(), onFinished: (_) {}),
        env,
      );
      await tester.pump();
      await tester.tap(find.text(BuyStrings.closePayment));
      await settleFrames(tester, 3);
      expect(env.checkout.statusCalls, greaterThan(0));
      expect(find.text(BuyStrings.paidTitle), findsOneWidget);
    });

    testWidgets('an_address_that_is_not_https_is_never_opened', (tester) async {
      final env = BuyEnv();
      await pumpBuy(
        tester,
        PaymentPage(
          checkout: checkout('http://pay.test/x'),
          onFinished: (_) {},
        ),
        env,
      );
      await tester.pump();
      expect(find.text(BuyStrings.paymentBlocked), findsOneWidget);
      expect(find.textContaining('PAY'), findsNothing);
    });
  });

  group('orders', () {
    testWidgets('the_list_shows_status_and_total_and_the_empty_state', (
      tester,
    ) async {
      final env = BuyEnv();
      await pumpBuy(tester, const OrdersPage(), env);
      await settleFrames(tester, 2);
      expect(find.text(BuyStrings.noOrders), findsOneWidget);
      env.orders.orderRows = [_order('o1', 'paid'), _order('o2', 'completed')];
      await pumpBuy(tester, const OrdersPage(), env);
      await settleFrames(tester, 2);
      expect(find.text('Ankara dress'), findsNWidgets(2));
      expect(
        find.textContaining(BuyStrings.statusLabel('paid')),
        findsOneWidget,
      );
    });

    testWidgets('message_the_seller_shows_once_paid_and_opens_the_chat', (
      tester,
    ) async {
      final pending = BuyEnv();
      pending.orders.orderRows = [_order('o1', 'pending')];
      await pumpBuy(tester, const BuyOrderPage(orderId: 'o1'), pending);
      await settleFrames(tester, 2);
      expect(find.text(ChatStrings.messageSeller), findsNothing);

      final env = BuyEnv();
      env.orders.orderRows = [_order('o1', 'paid')];
      await pumpBuy(tester, const BuyOrderPage(orderId: 'o1'), env);
      await settleFrames(tester, 2);
      await _tap(tester, find.text(ChatStrings.messageSeller));
      await settleFrames(tester, 4);
      expect(env.chat.opened, 1);
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(minutes: 1));
    });

    testWidgets('a_paid_order_can_be_cancelled_after_confirming', (
      tester,
    ) async {
      final env = BuyEnv();
      env.orders.orderRows = [_order('o1', 'paid')];
      await pumpBuy(tester, const BuyOrderPage(orderId: 'o1'), env);
      await settleFrames(tester, 2);
      expect(find.text(BuyStrings.received), findsNothing);
      await _tap(tester, find.text(BuyStrings.cancel));
      await settleFrames(tester, 3);
      expect(find.text(BuyStrings.cancelBody), findsOneWidget);
      await tester.tap(find.text(BuyStrings.keep));
      await settleFrames(tester, 3);
      expect(env.orders.actions, isEmpty);
      await _tap(tester, find.text(BuyStrings.cancel));
      await settleFrames(tester, 3);
      await tester.tap(find.text(BuyStrings.confirm));
      await settleFrames(tester, 4);
      expect(env.orders.actions, ['cancel:o1']);
      expect(find.text(BuyStrings.statusLabel('cancelled')), findsOneWidget);
    });

    testWidgets(
      'a_delivered_order_is_confirmed_received_only_after_a_warning',
      (tester) async {
        final env = BuyEnv();
        env.orders.orderRows = [_order('o1', 'delivered')];
        await pumpBuy(tester, const BuyOrderPage(orderId: 'o1'), env);
        await settleFrames(tester, 2);
        expect(find.text(BuyStrings.cancel), findsNothing);
        await _tap(tester, find.text(BuyStrings.received));
        await settleFrames(tester, 3);
        expect(find.text(BuyStrings.receivedBody), findsOneWidget);
        await tester.tap(find.text(BuyStrings.confirm));
        await settleFrames(tester, 4);
        expect(env.orders.actions, ['received:o1']);
        // Completed: review is offered.
        expect(find.text(BuyStrings.leaveReview), findsOneWidget);
      },
    );

    testWidgets('an_action_that_fails_shows_plain_words', (tester) async {
      final env = BuyEnv();
      env.orders.orderRows = [_order('o1', 'paid')];
      env.orders.failAction = apiError('invalid_order_state');
      await pumpBuy(tester, const BuyOrderPage(orderId: 'o1'), env);
      await settleFrames(tester, 2);
      await _tap(tester, find.text(BuyStrings.cancel));
      await settleFrames(tester, 3);
      await tester.tap(find.text(BuyStrings.confirm));
      await settleFrames(tester, 4);
      expect(find.byType(ErrorLine), findsOneWidget);
      expect(find.textContaining('invalid_order_state'), findsNothing);
    });

    testWidgets(
      'the_handover_code_shows_with_its_warning_and_the_pickup_point_for_collect',
      (tester) async {
        final env = BuyEnv();
        env.orders.orderRows = [_order('o1', 'paid')];
        env.orders.trackingRow = const Tracking(
          status: 'paid',
          steps: [TrackStep(key: 'paid', label: 'Paid', current: true)],
          handoverCode: '482913',
          pickupBranch: 'Garki',
          pickupAddress: '5 Market Road, Garki',
          pickupPhone: '08039876543',
          trackingUrl: 'https://track.test/abc',
          promisedDate: '2026-12-02',
        );
        final opened = <Uri>[];
        await pumpBuy(tester, const BuyOrderPage(orderId: 'o1'), env);
        await tester.pumpWidget(const SizedBox());
        tester.view.physicalSize = const Size(400, 1800);
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              ...env.overrides,
              externalLinkOpenerProvider.overrideWithValue((uri) async {
                opened.add(uri);
                return true;
              }),
            ],
            child: MaterialApp(
              theme: lightTheme(),
              home: const BuyOrderPage(orderId: 'o1'),
            ),
          ),
        );
        await settleFrames(tester, 3);
        expect(find.text('482913'), findsOneWidget);
        expect(find.text(BuyStrings.handoverHint), findsOneWidget);
        expect(find.text(BuyStrings.pickupTitle), findsOneWidget);
        expect(find.text('5 Market Road, Garki'), findsOneWidget);
        expect(find.text(BuyStrings.promised('2026-12-02')), findsOneWidget);
        await _tap(tester, find.text(BuyStrings.openTracking));
        expect(opened.single.toString(), 'https://track.test/abc');
      },
    );

    testWidgets('without_a_code_none_is_shown', (tester) async {
      final env = BuyEnv();
      env.orders.orderRows = [_order('o1', 'paid')];
      await pumpBuy(tester, const BuyOrderPage(orderId: 'o1'), env);
      await settleFrames(tester, 2);
      expect(find.text(BuyStrings.handoverTitle), findsNothing);
      expect(find.text(BuyStrings.pickupTitle), findsNothing);
    });
  });

  group('reports and reviews', () {
    testWidgets(
      'a_report_needs_words_and_sends_reason_text_and_uploaded_photos',
      (tester) async {
        final env = BuyEnv();
        env.photos = _Photos();
        env.orders.orderRows = [_order('o1', 'delivered')];
        await pumpBuy(tester, const ReportPage(orderId: 'o1'), env);
        await settleFrames(tester, 2);
        final send = find.widgetWithText(LoadingButton, BuyStrings.sendReport);
        expect(tester.widget<LoadingButton>(send).onPressed, isNull);
        await tester.tap(find.byType(DropdownButtonFormField<DisputeReason>));
        await settleFrames(tester, 3);
        await tester.tap(find.text(BuyStrings.reason('damaged')).last);
        await settleFrames(tester, 3);
        await tester.enterText(
          find.widgetWithText(TextField, BuyStrings.statementLabel),
          'The strap broke',
        );
        await tester.pump();
        await _tap(tester, find.text('Choose from gallery'));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 600)),
        );
        await settleFrames(tester, 3);
        expect(env.work.uploaded, ['u1']);
        await _tap(tester, send);
        await settleFrames(tester, 4);
        final d = env.orders.disputes.single;
        expect(d['reason'], 'damaged');
        expect(d['body'], 'The strap broke');
        expect(d['photos'], ['key/u1.jpg']);
        expect(find.text(BuyStrings.awaitingSeller), findsOneWidget);
      },
    );

    testWidgets('a_report_can_be_withdrawn_while_the_seller_has_not_answered', (
      tester,
    ) async {
      final env = BuyEnv();
      env.orders.disputeRow = BuyDispute(
        id: 'd1',
        status: 'awaiting_seller',
        respondBy: DateTime(2026, 12, 1),
      );
      await pumpBuy(tester, const DisputeStatusPage(disputeId: 'd1'), env);
      await settleFrames(tester, 2);
      await _tap(tester, find.text(BuyStrings.withdraw));
      await settleFrames(tester, 3);
      expect(find.text(BuyStrings.withdrawBody), findsOneWidget);
      await tester.tap(find.text(BuyStrings.confirm));
      await settleFrames(tester, 4);
      expect(env.orders.actions, ['withdraw:d1']);
      expect(find.text(BuyStrings.withdrawn), findsOneWidget);
    });

    testWidgets('a_decided_report_shows_the_outcome_and_the_sellers_answer', (
      tester,
    ) async {
      final env = BuyEnv();
      env.orders.disputeRow = BuyDispute(
        id: 'd1',
        status: 'decided',
        respondBy: null,
        sellerAnswer: 'It was sent Monday',
        refundKobo: 500000,
        decision: 'partial_refund',
      );
      await pumpBuy(tester, const DisputeStatusPage(disputeId: 'd1'), env);
      await settleFrames(tester, 2);
      expect(find.text(BuyStrings.decided), findsOneWidget);
      expect(find.text('It was sent Monday'), findsOneWidget);
      expect(find.text(BuyStrings.refunded('₦5,000')), findsOneWidget);
      expect(find.text(BuyStrings.withdraw), findsNothing);
    });

    testWidgets('a_review_needs_stars_and_sends_them_with_the_words', (
      tester,
    ) async {
      final env = BuyEnv();
      await pumpBuy(tester, const ReviewPage(orderId: 'o1'), env);
      await settleFrames(tester, 2);
      final send = find.widgetWithText(LoadingButton, BuyStrings.sendReview);
      expect(tester.widget<LoadingButton>(send).onPressed, isNull);
      await tester.tap(find.bySemanticsLabel('4'));
      await tester.pump();
      await tester.enterText(
        find.widgetWithText(TextField, BuyStrings.reviewText),
        'Lovely cloth',
      );
      await tester.pump();
      await _tap(tester, send);
      await settleFrames(tester, 3);
      expect(env.orders.reviews.single, {
        'order': 'o1',
        'stars': 4,
        'body': 'Lovely cloth',
        'photos': <String>[],
      });
    });
  });

  group('inbox and push', () {
    testWidgets('notices_show_unread_marks_and_open_the_order_by_id', (
      tester,
    ) async {
      final env = BuyEnv();
      env.orders.orderRows = [_order('o1', 'paid')];
      env.inbox.box = Inbox(
        unread: 1,
        items: [
          Notice(
            id: 'n1',
            kind: 'order_picked_up',
            title: 'Order picked up',
            body: 'Your order is on its way.',
            createdAt: DateTime(2026, 11, 20),
            read: false,
            orderId: 'o1',
          ),
          Notice(
            id: 'n2',
            kind: 'moderation_warning',
            title: 'Message not sent',
            body: 'A recent message broke the rules.',
            createdAt: DateTime(2026, 11, 19),
            read: true,
          ),
        ],
      );
      await pumpBuy(tester, const InboxPage(), env);
      await settleFrames(tester, 2);
      expect(find.text('Order picked up'), findsOneWidget);
      expect(find.text(BuyStrings.markAllRead), findsOneWidget);
      await tester.tap(find.text('Order picked up'));
      await settleFrames(tester, 4);
      expect(env.inbox.read, ['n1']);
      expect(find.byType(BuyOrderPage), findsOneWidget);
    });

    testWidgets('mark_all_read_and_the_empty_state', (tester) async {
      final env = BuyEnv();
      await pumpBuy(tester, const InboxPage(), env);
      await settleFrames(tester, 2);
      expect(find.text(BuyStrings.noNotices), findsOneWidget);
      env.inbox.box = Inbox(
        unread: 2,
        items: [
          Notice(
            id: 'n1',
            kind: 'k',
            title: 'T',
            body: 'B',
            createdAt: DateTime(2026),
            read: false,
          ),
        ],
      );
      await pumpBuy(tester, const InboxPage(), env);
      await settleFrames(tester, 2);
      await tester.tap(find.text(BuyStrings.markAllRead));
      await settleFrames(tester, 2);
      expect(env.inbox.read, ['all']);
    });

    testWidgets(
      'push_is_asked_once_explains_itself_and_registers_only_after_a_yes',
      (tester) async {
        SharedPreferences.setMockInitialValues({});
        final env = BuyEnv();
        await pumpBuy(
          tester,
          Consumer(
            builder: (context, ref, _) => TextButton(
              onPressed: () => maybeAskForPush(context, ref),
              child: const Text('ask'),
            ),
          ),
          env,
        );
        await tester.runAsync(() async {});
        await tester.tap(find.text('ask'));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
        await settleFrames(tester, 3);
        expect(find.text(BuyStrings.pushTitle), findsOneWidget);
        // The explanation says what is sent.
        expect(find.textContaining('device token'), findsOneWidget);
        await tester.tap(find.text(BuyStrings.pushYes));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
        await settleFrames(tester, 3);
        expect(env.push.calls, 1);
        expect(env.inbox.pushed, ['android']);
        // Never asked again.
        await tester.tap(find.text('ask'));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
        await settleFrames(tester, 3);
        expect(find.text(BuyStrings.pushTitle), findsNothing);
      },
    );

    testWidgets('saying_no_to_push_registers_nothing', (tester) async {
      SharedPreferences.setMockInitialValues({});
      final env = BuyEnv();
      await pumpBuy(
        tester,
        Consumer(
          builder: (context, ref, _) => TextButton(
            onPressed: () => maybeAskForPush(context, ref),
            child: const Text('ask'),
          ),
        ),
        env,
      );
      await tester.tap(find.text('ask'));
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 50)),
      );
      await settleFrames(tester, 3);
      await tester.tap(find.text(BuyStrings.pushNo));
      await settleFrames(tester, 3);
      expect(env.push.calls, 0);
      expect(env.inbox.pushed, isEmpty);
    });
  });
}
