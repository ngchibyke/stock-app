import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/features/buy/buy_models.dart';
import 'package:market/features/buy/buy_repository.dart';

import '../../support/fakes.dart';

ApiClient _client(FakeAdapter a) => ApiClient(
  baseUrl: 'https://api.test',
  readToken: () async => 'tok',
  onUnauthorized: () {},
  adapter: a,
);

void main() {
  test('checkout_sends_ids_only_with_an_idempotency_key_header', () async {
    final a = FakeAdapter()
      ..reply(200, {
        'stores': [
          {
            'store_id': 's1',
            'store_name': 'Shop',
            'groups': [],
            'unavailable': [],
          },
        ],
      })
      ..reply(201, {
        'checkout_id': 'c1',
        'reference': 'CK1',
        'payment_url': 'https://pay.test/x',
        'total_kobo': 1650000,
        'orders': [],
      })
      ..reply(200, {
        'checkout_id': 'c1',
        'status': 'paid',
        'orders': [
          {'id': 'o1'},
          {'id': 'o2'},
        ],
      });
    final repo = ApiCheckoutRepository(_client(a));
    final stores = await repo.options('addr1');
    expect(stores.single.storeName, 'Shop');
    expect(a.requests[0].path, '/checkout/delivery-options');
    expect(a.requests[0].body, {'address_id': 'addr1'});

    final created = await repo.create(
      method: 'courier',
      addressId: 'addr1',
      groups: {'g1': 'o1'},
      key: 'abc123',
    );
    expect(created.checkoutId, 'c1');
    expect(created.totalKobo, 1650000);
    final r = a.requests[1];
    expect(r.path, '/checkout');
    expect(r.headers['Idempotency-Key'], 'abc123');
    expect(r.body, {
      'method': 'courier',
      'address_id': 'addr1',
      'groups': [
        {'group_id': 'g1', 'option_id': 'o1'},
      ],
    });
    // No amount is ever sent.
    expect(r.body.toString(), isNot(contains('kobo')));

    final status = await repo.status('c1');
    expect(status.paid, isTrue);
    expect(status.orderIds, ['o1', 'o2']);
    expect(a.requests[2].path, '/checkout/c1');
  });

  test(
    'orders_tracking_actions_disputes_and_reviews_use_the_buyer_routes',
    () async {
      final order = {
        'id': 'o1',
        'status': 'paid',
        'created_at': '2026-11-20T10:00:00Z',
        'total_kobo': 100,
        'items': [],
      };
      final dispute = {
        'id': 'd1',
        'status': 'awaiting_seller',
        'seller_respond_by': '2026-12-01T00:00:00Z',
      };
      final a = FakeAdapter()
        ..reply(200, {
          'items': [order],
          'next_cursor': 'abc',
        })
        ..reply(200, order)
        ..reply(200, {'status': 'paid', 'steps': [], 'handover_code': '123456'})
        ..reply(200, order)
        ..reply(200, order)
        ..reply(201, dispute)
        ..reply(200, dispute)
        ..reply(200, dispute)
        ..reply(201, {});
      final repo = ApiBuyOrdersRepository(_client(a));
      final page = await repo.orders(before: 'cur');
      expect(page.nextCursor, 'abc');
      await repo.order('o1');
      expect((await repo.tracking('o1')).handoverCode, '123456');
      await repo.cancel('o1');
      await repo.received('o1');
      await repo.openDispute(
        'o1',
        reason: DisputeReason.notAsDescribed,
        body: 'Wrong colour',
        photoKeys: ['k1'],
      );
      await repo.dispute('d1');
      await repo.withdraw('d1');
      await repo.review('o1', stars: 5, body: null, photoKeys: const []);
      expect(
        [for (final r in a.requests) '${r.method} ${r.path}'],
        [
          'GET /orders',
          'GET /orders/o1',
          'GET /orders/o1/tracking',
          'POST /orders/o1/cancel',
          'POST /orders/o1/received',
          'POST /orders/o1/dispute',
          'GET /disputes/d1',
          'POST /disputes/d1/withdraw',
          'POST /orders/o1/review',
        ],
      );
      expect(a.requests[0].query, {'limit': '20', 'before': 'cur'});
      expect(a.requests[5].body, {
        'reason': 'not_as_described',
        'body': 'Wrong colour',
        'photo_keys': ['k1'],
      });
      expect((a.requests[8].body as Map).containsKey('body'), isFalse);
    },
  );

  test('the_inbox_and_push_routes', () async {
    final a = FakeAdapter()
      ..reply(200, {
        'items': [
          {
            'id': 'n1',
            'kind': 'order_paid',
            'title': 'T',
            'body': 'B',
            'created_at': '2026-11-20T10:00:00Z',
            'read': false,
            'order_id': 'o1',
          },
        ],
        'unread': 1,
      })
      ..reply(200, {})
      ..reply(200, {})
      ..reply(204)
      ..reply(204);
    final repo = ApiInboxRepository(_client(a));
    final box = await repo.inbox();
    expect(box.unread, 1);
    expect(box.items.single.orderId, 'o1');
    await repo.markRead(ids: ['n1']);
    await repo.markRead(all: true);
    await repo.registerPush('device-token', 'android');
    await repo.removePush();
    expect(a.requests[1].body, {
      'ids': ['n1'],
    });
    expect(a.requests[2].body, {'all': true});
    expect(a.requests[3].method, 'PUT');
    expect(a.requests[3].path, '/me/push-device');
    expect(a.requests[3].body, {
      'token': 'device-token',
      'platform': 'android',
    });
    expect(a.requests[4].method, 'DELETE');
  });
}
