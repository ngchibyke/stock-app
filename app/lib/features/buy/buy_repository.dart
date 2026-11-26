import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';
import '../../core/json_read.dart';
import 'buy_models.dart';

ApiError _bad() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);

Map<String, dynamic> _obj(Object? j) =>
    j is Map<String, dynamic> ? j : throw _bad();

String _id(String s) => Uri.encodeComponent(s);

abstract class CheckoutRepository {
  Future<List<StoreDelivery>> options(String addressId);

  /// [groups] maps each group id to the chosen option id. [key] makes a retry
  /// the same checkout. Throws an [ApiError]: 409 `allocation_changed`,
  /// `checkout_busy`, and others.
  Future<CreatedCheckout> create({
    required String method,
    required String addressId,
    required Map<String, String> groups,
    required String key,
  });

  Future<CheckoutStatus> status(String checkoutId);
}

class ApiCheckoutRepository implements CheckoutRepository {
  ApiCheckoutRepository(this._api);

  final ApiClient _api;

  @override
  Future<List<StoreDelivery>> options(String addressId) async {
    final j = _obj(
      await _api.post(
        '/checkout/delivery-options',
        body: {'address_id': addressId},
      ),
    );
    return [
      for (final s in readObjects(j['stores'])) ?StoreDelivery.fromJson(s),
    ];
  }

  @override
  Future<CreatedCheckout> create({
    required String method,
    required String addressId,
    required Map<String, String> groups,
    required String key,
  }) async {
    final j = _obj(
      await _api.post(
        '/checkout',
        body: {
          'method': method,
          'address_id': addressId,
          'groups': [
            for (final e in groups.entries)
              {'group_id': e.key, 'option_id': e.value},
          ],
        },
        headers: {'Idempotency-Key': key},
      ),
    );
    final id = readString(j['checkout_id']);
    final url = readString(j['payment_url']);
    if (id == null || url == null) throw _bad();
    return CreatedCheckout(
      checkoutId: id,
      paymentUrl: url,
      totalKobo: readInt(j['total_kobo']) ?? 0,
    );
  }

  @override
  Future<CheckoutStatus> status(String checkoutId) async {
    final j = _obj(await _api.get('/checkout/${_id(checkoutId)}'));
    return CheckoutStatus(
      status: readString(j['status']) ?? '',
      orderIds: [
        for (final o in readObjects(j['orders'])) ?readString(o['id']),
      ],
    );
  }
}

final checkoutRepositoryProvider = Provider<CheckoutRepository>(
  (ref) => ApiCheckoutRepository(ref.watch(apiClientProvider)),
);

abstract class BuyOrdersRepository {
  Future<BuyOrdersPage> orders({String? before});
  Future<BuyOrder> order(String id);
  Future<Tracking> tracking(String id);
  Future<BuyOrder> cancel(String id);
  Future<BuyOrder> received(String id);

  Future<BuyDispute> openDispute(
    String orderId, {
    required DisputeReason reason,
    required String body,
    required List<String> photoKeys,
  });
  Future<BuyDispute> dispute(String id);
  Future<BuyDispute> withdraw(String id);
  Future<void> review(
    String orderId, {
    required int stars,
    String? body,
    required List<String> photoKeys,
  });
}

class ApiBuyOrdersRepository implements BuyOrdersRepository {
  ApiBuyOrdersRepository(this._api);

  final ApiClient _api;

  BuyOrder _order(Object? j) => BuyOrder.fromJson(_obj(j)) ?? (throw _bad());
  BuyDispute _dispute(Object? j) =>
      BuyDispute.fromJson(_obj(j)) ?? (throw _bad());

  @override
  Future<BuyOrdersPage> orders({String? before}) async =>
      BuyOrdersPage.fromJson(
        _obj(
          await _api.get('/orders', query: {'limit': 20, 'before': ?before}),
        ),
      );

  @override
  Future<BuyOrder> order(String id) async =>
      _order(await _api.get('/orders/${_id(id)}'));

  @override
  Future<Tracking> tracking(String id) async =>
      Tracking.fromJson(_obj(await _api.get('/orders/${_id(id)}/tracking')));

  @override
  Future<BuyOrder> cancel(String id) async =>
      _order(await _api.post('/orders/${_id(id)}/cancel'));

  @override
  Future<BuyOrder> received(String id) async =>
      _order(await _api.post('/orders/${_id(id)}/received'));

  @override
  Future<BuyDispute> openDispute(
    String orderId, {
    required DisputeReason reason,
    required String body,
    required List<String> photoKeys,
  }) async => _dispute(
    await _api.post(
      '/orders/${_id(orderId)}/dispute',
      body: {'reason': reason.wire, 'body': body, 'photo_keys': photoKeys},
    ),
  );

  @override
  Future<BuyDispute> dispute(String id) async =>
      _dispute(await _api.get('/disputes/${_id(id)}'));

  @override
  Future<BuyDispute> withdraw(String id) async =>
      _dispute(await _api.post('/disputes/${_id(id)}/withdraw'));

  @override
  Future<void> review(
    String orderId, {
    required int stars,
    String? body,
    required List<String> photoKeys,
  }) async {
    await _api.post(
      '/orders/${_id(orderId)}/review',
      body: {'stars': stars, 'body': ?body, 'photo_keys': photoKeys},
    );
  }
}

final buyOrdersRepositoryProvider = Provider<BuyOrdersRepository>(
  (ref) => ApiBuyOrdersRepository(ref.watch(apiClientProvider)),
);

abstract class InboxRepository {
  Future<Inbox> inbox();
  Future<void> markRead({List<String>? ids, bool all = false});
  Future<void> registerPush(String token, String platform);
  Future<void> removePush();
}

class ApiInboxRepository implements InboxRepository {
  ApiInboxRepository(this._api);

  final ApiClient _api;

  @override
  Future<Inbox> inbox() async => Inbox.fromJson(
    _obj(await _api.get('/me/notifications', query: {'limit': 50})),
  );

  @override
  Future<void> markRead({List<String>? ids, bool all = false}) async {
    await _api.post(
      '/me/notifications/read',
      body: {if (all) 'all': true, 'ids': ?ids},
    );
  }

  @override
  Future<void> registerPush(String token, String platform) async {
    await _api.put(
      '/me/push-device',
      body: {'token': token, 'platform': platform},
    );
  }

  @override
  Future<void> removePush() async {
    await _api.delete('/me/push-device');
  }
}

final inboxRepositoryProvider = Provider<InboxRepository>(
  (ref) => ApiInboxRepository(ref.watch(apiClientProvider)),
);
