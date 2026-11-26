import 'package:market/features/buy/buy_models.dart';
import 'package:market/features/buy/buy_repository.dart';

export 'fake_sell.dart' show apiError;

class FakeCheckoutRepository implements CheckoutRepository {
  List<StoreDelivery> stores = [];
  Object? failCreate;
  Object? failOptions;

  /// What `status` answers, in order; the last one repeats.
  List<CheckoutStatus> statuses = [
    const CheckoutStatus(status: 'pending', orderIds: []),
  ];

  final List<String> optionCalls = [];
  final List<Map<String, Object?>> created = [];
  int statusCalls = 0;

  @override
  Future<List<StoreDelivery>> options(String addressId) async {
    optionCalls.add(addressId);
    if (failOptions != null) throw failOptions!;
    return stores;
  }

  @override
  Future<CreatedCheckout> create({
    required String method,
    required String addressId,
    required Map<String, String> groups,
    required String key,
  }) async {
    created.add({
      'method': method,
      'address': addressId,
      'groups': groups,
      'key': key,
    });
    if (failCreate != null) {
      final e = failCreate!;
      // A failure is only for the first call, so a refresh and retry can work.
      failCreate = null;
      throw e;
    }
    return const CreatedCheckout(
      checkoutId: 'c1',
      paymentUrl: 'https://pay.test/x',
      totalKobo: 1000,
    );
  }

  @override
  Future<CheckoutStatus> status(String checkoutId) async {
    final i = statusCalls < statuses.length ? statusCalls : statuses.length - 1;
    statusCalls++;
    return statuses[i];
  }
}

class FakeBuyOrdersRepository implements BuyOrdersRepository {
  List<BuyOrder> orderRows = [];
  Tracking trackingRow = const Tracking(status: 'paid', steps: []);
  BuyDispute? disputeRow;
  Object? failAction;

  final List<String> actions = [];
  final List<Map<String, Object?>> disputes = [];
  final List<Map<String, Object?>> reviews = [];

  @override
  Future<BuyOrdersPage> orders({String? before}) async =>
      BuyOrdersPage(items: [...orderRows]);

  @override
  Future<BuyOrder> order(String id) async =>
      orderRows.firstWhere((o) => o.id == id);

  @override
  Future<Tracking> tracking(String id) async => trackingRow;

  BuyOrder _set(String id, String status) {
    final o = orderRows.firstWhere((o) => o.id == id);
    final next = BuyOrder(
      id: o.id,
      status: status,
      totalKobo: o.totalKobo,
      createdAt: o.createdAt,
      lines: o.lines,
    );
    orderRows = [for (final x in orderRows) x.id == id ? next : x];
    return next;
  }

  @override
  Future<BuyOrder> cancel(String id) async {
    actions.add('cancel:$id');
    if (failAction != null) throw failAction!;
    return _set(id, 'cancelled');
  }

  @override
  Future<BuyOrder> received(String id) async {
    actions.add('received:$id');
    if (failAction != null) throw failAction!;
    return _set(id, 'completed');
  }

  @override
  Future<BuyDispute> openDispute(
    String orderId, {
    required DisputeReason reason,
    required String body,
    required List<String> photoKeys,
  }) async {
    disputes.add({
      'order': orderId,
      'reason': reason.wire,
      'body': body,
      'photos': photoKeys,
    });
    if (failAction != null) throw failAction!;
    return disputeRow = BuyDispute(
      id: 'd1',
      status: 'awaiting_seller',
      respondBy: DateTime(2026, 12, 1),
    );
  }

  @override
  Future<BuyDispute> dispute(String id) async => disputeRow!;

  @override
  Future<BuyDispute> withdraw(String id) async {
    actions.add('withdraw:$id');
    return disputeRow = BuyDispute(
      id: id,
      status: 'withdrawn',
      respondBy: null,
    );
  }

  @override
  Future<void> review(
    String orderId, {
    required int stars,
    String? body,
    required List<String> photoKeys,
  }) async {
    reviews.add({
      'order': orderId,
      'stars': stars,
      'body': body,
      'photos': photoKeys,
    });
  }
}

class FakeInboxRepository implements InboxRepository {
  Inbox box = const Inbox(items: [], unread: 0);
  final List<String> read = [];
  final List<String> pushed = [];

  @override
  Future<Inbox> inbox() async => box;

  @override
  Future<void> markRead({List<String>? ids, bool all = false}) async {
    read.addAll(all ? ['all'] : ids ?? []);
  }

  @override
  Future<void> registerPush(String token, String platform) async {
    pushed.add(platform);
  }

  @override
  Future<void> removePush() async {}
}
