import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/features/buy/buy_models.dart';
import 'package:market/features/buy/buy_strings.dart';
import 'package:market/features/buy/checkout_page.dart';
import 'package:market/features/buy/dispute_pages.dart';
import 'package:market/features/buy/inbox.dart';
import 'package:market/features/buy/payment_rules.dart';

import '../../support/fake_buy.dart';

DeliveryOption _opt(String id, BuyMethod m, int fee) =>
    DeliveryOption(id: id, method: m, feeKobo: fee, promisedDate: '2026-12-01');

StoreDelivery _store(
  List<DeliveryGroup> groups, {
  List<ShortLine> short = const [],
}) => StoreDelivery(
  storeId: 's',
  storeName: 'Shop',
  groups: groups,
  short: short,
);

void main() {
  group('payment rules', () {
    const ret = 'https://app.test/checkout/return';

    test('the_return_page_matches_on_host_and_path_and_ignores_the_query', () {
      expect(
        isReturnUrl('https://app.test/checkout/return?ref=CK1&status=ok', ret),
        isTrue,
      );
      expect(isReturnUrl('https://app.test/checkout/return/', ret), isTrue);
      expect(isReturnUrl('https://app.test/checkout/other', ret), isFalse);
      expect(isReturnUrl('https://evil.test/checkout/return', ret), isFalse);
      expect(isReturnUrl('http://app.test/checkout/return', ret), isFalse);
      expect(isReturnUrl('https://app.test/checkout/return', null), isFalse);
      expect(isReturnUrl('not a url', ret), isFalse);
    });

    test('only_https_pages_and_the_return_page_may_open', () {
      expect(mayNavigate('https://bank.test/3ds', ret), isTrue);
      expect(mayNavigate('http://bank.test/3ds', ret), isFalse);
      expect(mayNavigate('intent://pay#Intent;scheme=x;end', ret), isFalse);
      expect(mayNavigate('file:///etc/passwd', ret), isFalse);
      expect(mayNavigate('javascript:alert(1)', ret), isFalse);
      expect(
        mayNavigate(
          'http://localhost:8080/checkout/return',
          'http://localhost:8080/checkout/return',
        ),
        isTrue,
      );
    });

    test('a_payment_starts_only_on_https_or_a_loopback_dev_host', () {
      expect(mayStartPayment('https://pay.test/x'), isTrue);
      expect(mayStartPayment('http://10.0.2.2:9000/x'), isTrue);
      expect(mayStartPayment('http://pay.test/x'), isFalse);
      expect(mayStartPayment('ftp://pay.test'), isFalse);
      expect(mayStartPayment(''), isFalse);
    });

    test('the_return_url_is_read_from_the_config_with_the_same_rules', () {
      expect(
        AppConfig.fromJson({'payment_return_url': 'https://a.test/r'})
            .paymentReturnUrl,
        'https://a.test/r',
      );
      expect(
        AppConfig.fromJson({'payment_return_url': 'http://localhost:8080/r'})
            .paymentReturnUrl,
        'http://localhost:8080/r',
      );
      expect(
        AppConfig.fromJson({'payment_return_url': 'http://a.test/r'})
            .paymentReturnUrl,
        isNull,
      );
      expect(AppConfig.fromJson({}).paymentReturnUrl, isNull);
      final back = AppConfig.fromJson(
        const AppConfig(paymentReturnUrl: 'https://a.test/r').toJson(),
      );
      expect(back.paymentReturnUrl, 'https://a.test/r');
    });
  });

  group('delivery choice', () {
    test('only_methods_every_group_can_do_are_offered', () {
      final stores = [
        _store([
          DeliveryGroup(
            groupId: 'g1',
            branchName: 'A',
            options: [
              _opt('1', BuyMethod.courier, 100),
              _opt('2', BuyMethod.collect, 0),
            ],
          ),
          DeliveryGroup(
            groupId: 'g2',
            branchName: 'B',
            options: [_opt('3', BuyMethod.courier, 200)],
          ),
        ]),
      ];
      expect(commonMethods(stores), [BuyMethod.courier]);
      expect(commonMethods(const []), isEmpty);
    });

    test('each_group_gets_the_cheapest_option_of_the_chosen_method', () {
      final stores = [
        _store([
          DeliveryGroup(
            groupId: 'g1',
            branchName: 'A',
            options: [
              _opt('dear', BuyMethod.courier, 900),
              _opt('cheap', BuyMethod.courier, 400),
              _opt('c', BuyMethod.collect, 0),
            ],
          ),
        ]),
      ];
      expect(choose(stores, BuyMethod.courier)['g1']!.id, 'cheap');
      expect(choose(stores, BuyMethod.collect)['g1']!.feeKobo, 0);
      expect(choose(stores, BuyMethod.self), isEmpty);
    });

    test(
      'a_store_is_ready_only_when_every_group_can_send_and_no_line_is_short',
      () {
        final ok = DeliveryGroup(
          groupId: 'g',
          branchName: 'A',
          options: [_opt('1', BuyMethod.courier, 1)],
        );
        expect(_store([ok]).ready, isTrue);
        expect(
          _store(
            [ok],
            short: const [ShortLine(variantId: 'v', wanted: 3, available: 1)],
          ).ready,
          isFalse,
        );
        expect(
          _store([
            const DeliveryGroup(
              groupId: 'g',
              branchName: 'A',
              options: [],
              unavailable: 'no_options',
            ),
          ]).ready,
          isFalse,
        );
        expect(_store(const []).ready, isFalse);
      },
    );

    test('options_parse_from_the_server_shape', () {
      final s = StoreDelivery.fromJson({
        'store_id': 's1',
        'store_name': 'Ada Shop',
        'groups': [
          {
            'group_id': 'g1',
            'branch': {'id': 'b', 'name': 'Garki', 'area_council': 'amac'},
            'lines': [],
            'options': [
              {
                'id': 'o1',
                'method': 'courier',
                'fee_kobo': 150000,
                'courier_name': 'Fast',
                'delivery_eta': '2 days',
                'promised_date': '2026-12-02',
              },
              {'id': 'o2', 'method': 'teleport', 'fee_kobo': 1},
            ],
          },
        ],
        'unavailable': [
          {'variant_id': 'v9', 'wanted': 4, 'available': 2},
        ],
      })!;
      expect(s.groups.single.options.single.courierName, 'Fast');
      expect(s.groups.single.branchName, 'Garki');
      expect(s.short.single.available, 2);
    });

    test('an_idempotency_key_is_random_hex_the_server_accepts', () {
      final keys = {for (var i = 0; i < 20; i++) newIdempotencyKey()};
      expect(keys, hasLength(20));
      for (final k in keys) {
        expect(RegExp(r'^[a-f0-9]{32}$').hasMatch(k), isTrue);
      }
    });
  });

  group('models', () {
    test('an_order_knows_what_the_buyer_may_do', () {
      BuyOrder o(String status, {String? dispute}) => BuyOrder(
        id: 'o',
        status: status,
        totalKobo: 1,
        createdAt: DateTime(2026),
        lines: const [],
        disputeId: dispute,
      );
      expect(o('paid').canCancel, isTrue);
      expect(o('picked_up').canCancel, isFalse);
      expect(o('delivered').canConfirm, isTrue);
      expect(o('paid').canConfirm, isFalse);
      expect(o('completed').canReview, isTrue);
      expect(o('delivered').canReport, isTrue);
      expect(o('delivered', dispute: 'd').canReport, isFalse);
      expect(o('cancelled').canReport, isFalse);
    });

    test('tracking_keeps_only_https_links_and_hides_secrets_from_text', () {
      final t = Tracking.fromJson({
        'status': 'out_for_delivery',
        'tracking_url': 'http://track.test/1',
        'handover_code': '123456',
        'pickup': {
          'branch_name': 'Garki',
          'address': '1 Road',
          'phone': '0803',
        },
        'steps': [
          {
            'key': 'paid',
            'label': 'Paid',
            'at': '2026-11-20T10:00:00Z',
            'current': false,
          },
          {'key': 'x', 'current': true},
        ],
      });
      expect(t.trackingUrl, isNull);
      expect(t.handoverCode, '123456');
      expect(t.pickupBranch, 'Garki');
      expect(t.steps, hasLength(2));
      expect(t.steps.last.label, 'x');
      expect('$t', isNot(contains('123456')));
      expect(
        '${const CreatedCheckout(checkoutId: 'c', paymentUrl: 'https://pay.test/secret', totalKobo: 1)}',
        isNot(contains('secret')),
      );
    });

    test('a_dispute_is_open_while_the_market_has_not_decided', () {
      BuyDispute d(String s) => BuyDispute(id: 'd', status: s, respondBy: null);
      expect(d('awaiting_seller').open, isTrue);
      expect(d('awaiting_return').open, isTrue);
      expect(d('decided').open, isFalse);
      expect(d('withdrawn').open, isFalse);
      expect(d('awaiting_seller').awaitingSeller, isTrue);
    });

    test('notices_parse_and_lead_to_the_right_page_by_their_ids_only', () {
      final n = Notice.fromJson({
        'id': 'n1',
        'kind': 'order_paid',
        'title': 'T',
        'body': 'B',
        'created_at': '2026-11-20T10:00:00Z',
        'read': false,
        'order_id': 'o1',
      })!;
      expect(pageForNotice(n), isNotNull);
      final d = Notice.fromJson({
        'id': 'n2',
        'kind': 'dispute_decided',
        'created_at': '2026-11-20T10:00:00Z',
        'dispute_id': 'd1',
      })!;
      expect(pageForNotice(d), isA<DisputeStatusPage>());
      final none = Notice.fromJson({
        'id': 'n3',
        'kind': 'moderation_muted',
        'created_at': '2026-11-20T10:00:00Z',
      })!;
      expect(pageForNotice(none), isNull);
      expect(Notice.fromJson({'id': 'x'}), isNull);
    });

    test('errors_are_plain_words', () {
      expect(
        buyError(apiError('allocation_changed')),
        BuyStrings.optionsChanged,
      );
      expect(
        buyError(apiError('too_many_pending_checkouts', status: 429)),
        BuyStrings.tooManyPending,
      );
      expect(buyError(apiError('checkout_busy')), BuyStrings.busy);
      expect(
        buyError(apiError('cart_empty', status: 422)),
        BuyStrings.cartEmpty,
      );
      expect(
        buyError(apiError('weird_code', status: 502)),
        BuyStrings.payFailed,
      );
      expect(buyError(apiError('weird_code')), isNot(contains('weird')));
    });
  });
}
