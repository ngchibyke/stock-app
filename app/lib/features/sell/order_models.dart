import 'package:flutter/foundation.dart';

import '../../core/json_read.dart';

@immutable
class OrderLine {
  const OrderLine({
    required this.name,
    required this.quantity,
    required this.unitPriceKobo,
    required this.label,
  });

  final String name;
  final int quantity;
  final int unitPriceKobo;
  final String label;
}

/// How an order reaches the buyer.
enum DeliveryMethod {
  courier,
  self,
  collect;

  static DeliveryMethod? fromWire(Object? v) {
    for (final m in values) {
      if (m.name == v) return m;
    }
    return null;
  }
}

/// A step the seller side can take, in the order they are taken.
enum OrderStep { ready, pickedUp, outForDelivery, delivered }

/// An order as its store's team sees it.
@immutable
class SellOrder {
  const SellOrder({
    required this.id,
    required this.status,
    required this.lines,
    required this.totalKobo,
    required this.createdAt,
    this.method,
    this.branchName,
    this.buyerName,
    this.expectedPayoutKobo,
    this.held = false,
    this.disputeId,
    this.doneBy = const {},
    this.shipBy,
  });

  static SellOrder? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final created = DateTime.tryParse(readString(j['created_at']) ?? '');
    if (id == null || created == null) return null;
    final dispute = j['dispute'];
    final done = j['done_by'];
    return SellOrder(
      id: id,
      status: readString(j['status']) ?? '',
      method: DeliveryMethod.fromWire(j['delivery_method']),
      branchName: readString(j['branch_name']),
      buyerName: readString(j['buyer_display_name']),
      totalKobo: readInt(j['total_kobo']) ?? 0,
      expectedPayoutKobo: readInt(j['expected_payout_kobo']),
      held: j['held'] == true,
      disputeId: dispute is Map ? readString(dispute['id']) : null,
      createdAt: created,
      shipBy: DateTime.tryParse(readString(j['ship_by']) ?? ''),
      doneBy: {
        if (done is Map)
          for (final e in done.entries)
            if (e.key is String && e.value is Map)
              e.key as String:
                  readString((e.value as Map)['display_name']) ?? '',
      },
      lines: [
        for (final i in readObjects(j['items']))
          OrderLine(
            name: readString(i['product_name']) ?? '',
            quantity: readInt(i['quantity']) ?? 1,
            unitPriceKobo: readInt(i['unit_price_kobo']) ?? 0,
            label: _attrLabel(i['attributes']),
          ),
      ],
    );
  }

  static String _attrLabel(Object? a) => a is Map
      ? [
          for (final v in a.values)
            if (v is String && v.isNotEmpty) v,
        ].join(' / ')
      : '';

  final String id;

  /// `paid`, `picked_up`, `out_for_delivery`, `delivered`, `completed`,
  /// `cancelled`, `refunded`, ...
  final String status;
  final DeliveryMethod? method;
  final String? branchName;
  final String? buyerName;
  final List<OrderLine> lines;
  final int totalKobo;
  final int? expectedPayoutKobo;
  final bool held;
  final String? disputeId;
  final Map<String, String> doneBy;
  final DateTime createdAt;
  final DateTime? shipBy;

  bool get needsShipping => status == 'paid';
  bool get isOpen => const {
    'paid',
    'picked_up',
    'out_for_delivery',
    'delivered',
  }.contains(status);

  /// The next thing the team does, or null when there is nothing to do now.
  /// A courier or collect order is made ready; a self-delivery order goes
  /// picked up, out for delivery, then delivered with the buyer's code. A
  /// collect order is handed over with the code once ready.
  OrderStep? nextStep() {
    if (held || method == null) return null;
    final alreadyReady = doneBy.containsKey('ready');
    switch ((method!, status)) {
      case (DeliveryMethod.courier, 'paid'):
        return alreadyReady ? null : OrderStep.ready;
      case (DeliveryMethod.collect, 'paid'):
        return alreadyReady ? OrderStep.delivered : OrderStep.ready;
      case (DeliveryMethod.self, 'paid'):
        return OrderStep.pickedUp;
      case (DeliveryMethod.self, 'picked_up'):
        return OrderStep.outForDelivery;
      case (DeliveryMethod.self, 'out_for_delivery'):
        return OrderStep.delivered;
      default:
        return null;
    }
  }

  /// Whether the step asks for the buyer's six-digit handover code.
  static bool needsCode(OrderStep s) => s == OrderStep.delivered;

  @override
  String toString() => 'SellOrder($id)';
}

class OrdersPage {
  const OrdersPage({required this.items, this.nextCursor});

  factory OrdersPage.fromJson(Map<String, dynamic> j) => OrdersPage(
    items: [for (final o in readObjects(j['items'])) ?SellOrder.fromJson(o)],
    nextCursor: readString(j['next_cursor']),
  );

  final List<SellOrder> items;
  final String? nextCursor;
}

@immutable
class SellReview {
  const SellReview({
    required this.id,
    required this.stars,
    required this.createdAt,
    this.body,
    this.reply,
  });

  static SellReview? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final created = DateTime.tryParse(readString(j['created_at']) ?? '');
    if (id == null || created == null) return null;
    return SellReview(
      id: id,
      stars: readInt(j['stars']) ?? 0,
      body: readString(j['body']),
      reply: readString(j['reply']),
      createdAt: created,
    );
  }

  final String id;
  final int stars;
  final String? body;
  final String? reply;
  final DateTime createdAt;
}

@immutable
class DisputeStatement {
  const DisputeStatement({required this.body, required this.photoKeys});

  static DisputeStatement? fromJson(Object? j) {
    if (j is! Map<String, dynamic>) return null;
    return DisputeStatement(
      body: readString(j['body']) ?? '',
      photoKeys: [
        if (j['photo_keys'] is List)
          for (final k in j['photo_keys'] as List)
            if (k is String && k.isNotEmpty) k,
      ],
    );
  }

  final String body;
  final List<String> photoKeys;
}

@immutable
class SellDispute {
  const SellDispute({
    required this.id,
    required this.orderId,
    required this.status,
    required this.reason,
    required this.respondBy,
    required this.buyer,
    this.seller,
    this.returnRequested = false,
  });

  static SellDispute? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final buyer = DisputeStatement.fromJson(j['buyer_statement']);
    if (id == null || buyer == null) return null;
    return SellDispute(
      id: id,
      orderId: readString(j['order_id']) ?? '',
      status: readString(j['status']) ?? '',
      reason: readString(j['reason']) ?? '',
      respondBy:
          DateTime.tryParse(readString(j['seller_respond_by']) ?? '') ??
          DateTime.fromMillisecondsSinceEpoch(0),
      buyer: buyer,
      seller: DisputeStatement.fromJson(j['seller_statement']),
      returnRequested: j['return_request'] != null,
    );
  }

  final String id;
  final String orderId;
  final String status;
  final String reason;
  final DateTime respondBy;
  final DisputeStatement buyer;
  final DisputeStatement? seller;
  final bool returnRequested;

  bool get canRespond => seller == null && status == 'open';
}
