import 'package:flutter/foundation.dart';

import '../../core/json_read.dart';

/// How a group of goods reaches the buyer.
enum BuyMethod {
  courier,
  self,
  collect;

  static BuyMethod? fromWire(Object? v) {
    for (final m in values) {
      if (m.name == v) return m;
    }
    return null;
  }
}

@immutable
class DeliveryOption {
  const DeliveryOption({
    required this.id,
    required this.method,
    required this.feeKobo,
    required this.promisedDate,
    this.courierName,
    this.eta,
  });

  static DeliveryOption? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final method = BuyMethod.fromWire(j['method']);
    if (id == null || method == null) return null;
    return DeliveryOption(
      id: id,
      method: method,
      feeKobo: readInt(j['fee_kobo']) ?? 0,
      promisedDate: readString(j['promised_date']) ?? '',
      courierName: readString(j['courier_name']),
      eta: readString(j['delivery_eta']),
    );
  }

  final String id;
  final BuyMethod method;
  final int feeKobo;

  /// `YYYY-MM-DD`: the date promised if paid now.
  final String promisedDate;
  final String? courierName;
  final String? eta;
}

/// One branch's share of a store's goods, with the ways to send it.
@immutable
class DeliveryGroup {
  const DeliveryGroup({
    required this.groupId,
    required this.branchName,
    required this.options,
    this.unavailable,
  });

  static DeliveryGroup? fromJson(Map<String, dynamic> j) {
    final id = readString(j['group_id']);
    if (id == null) return null;
    final branch = j['branch'];
    return DeliveryGroup(
      groupId: id,
      branchName: branch is Map ? readString(branch['name']) ?? '' : '',
      unavailable: readString(j['unavailable']),
      options: [
        for (final o in readObjects(j['options'])) ?DeliveryOption.fromJson(o),
      ],
    );
  }

  final String groupId;
  final String branchName;
  final List<DeliveryOption> options;

  /// Set when nothing can be sent for this group.
  final String? unavailable;
}

@immutable
class ShortLine {
  const ShortLine({
    required this.variantId,
    required this.wanted,
    required this.available,
  });

  final String variantId;
  final int wanted;
  final int available;
}

@immutable
class StoreDelivery {
  const StoreDelivery({
    required this.storeId,
    required this.storeName,
    required this.groups,
    required this.short,
  });

  static StoreDelivery? fromJson(Map<String, dynamic> j) {
    final id = readString(j['store_id']);
    if (id == null) return null;
    return StoreDelivery(
      storeId: id,
      storeName: readString(j['store_name']) ?? '',
      groups: [
        for (final g in readObjects(j['groups'])) ?DeliveryGroup.fromJson(g),
      ],
      short: [
        for (final u in readObjects(j['unavailable']))
          if (readString(u['variant_id']) case final v?)
            ShortLine(
              variantId: v,
              wanted: readInt(u['wanted']) ?? 0,
              available: readInt(u['available']) ?? 0,
            ),
      ],
    );
  }

  final String storeId;
  final String storeName;
  final List<DeliveryGroup> groups;
  final List<ShortLine> short;

  /// Whether every group has a way to send it and no line is short.
  bool get ready =>
      short.isEmpty &&
      groups.isNotEmpty &&
      groups.every((g) => g.unavailable == null && g.options.isNotEmpty);
}

/// A created checkout. The payment address is held in memory only.
@immutable
class CreatedCheckout {
  const CreatedCheckout({
    required this.checkoutId,
    required this.paymentUrl,
    required this.totalKobo,
  });

  final String checkoutId;
  final String paymentUrl;
  final int totalKobo;

  @override
  String toString() => 'CreatedCheckout($checkoutId)';
}

/// The server's word on a checkout: `pending`, `paid`, `expired`, `failed` ...
@immutable
class CheckoutStatus {
  const CheckoutStatus({required this.status, required this.orderIds});

  final String status;
  final List<String> orderIds;

  bool get paid => status == 'paid';
  bool get pending => status == 'pending';
}

@immutable
class BuyLine {
  const BuyLine({
    required this.name,
    required this.quantity,
    required this.unitPriceKobo,
  });

  final String name;
  final int quantity;
  final int unitPriceKobo;
}

/// An order as its buyer sees it.
@immutable
class BuyOrder {
  const BuyOrder({
    required this.id,
    required this.status,
    required this.totalKobo,
    required this.createdAt,
    required this.lines,
    this.disputeId,
    this.disputeStatus,
    this.refundedKobo,
  });

  static BuyOrder? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final created = DateTime.tryParse(readString(j['created_at']) ?? '');
    if (id == null || created == null) return null;
    final d = j['dispute'];
    return BuyOrder(
      id: id,
      status: readString(j['status']) ?? '',
      totalKobo: readInt(j['total_kobo']) ?? 0,
      createdAt: created,
      refundedKobo: readInt(j['refunded_kobo']),
      disputeId: d is Map ? readString(d['id']) : null,
      disputeStatus: d is Map ? readString(d['status']) : null,
      lines: [
        for (final i in readObjects(j['items']))
          BuyLine(
            name: readString(i['product_name']) ?? '',
            quantity: readInt(i['quantity']) ?? 1,
            unitPriceKobo: readInt(i['unit_price_kobo']) ?? 0,
          ),
      ],
    );
  }

  final String id;
  final String status;
  final int totalKobo;
  final DateTime createdAt;
  final List<BuyLine> lines;
  final String? disputeId;
  final String? disputeStatus;
  final int? refundedKobo;

  /// The buyer may cancel until the order is on its way.
  bool get canCancel => status == 'paid';

  /// Delivered, waiting for the buyer to confirm.
  bool get canConfirm => status == 'delivered';

  bool get canReview => status == 'completed';

  /// A conversation about the order opens once it is paid.
  bool get canMessage => const {
    'paid',
    'picked_up',
    'out_for_delivery',
    'delivered',
    'completed',
  }.contains(status);

  /// Still on its way to the buyer.
  bool get isOpenForDelivery =>
      const {'paid', 'picked_up', 'out_for_delivery'}.contains(status);

  /// A problem can be reported on an order that is delivered or finished, until
  /// the server says otherwise (it refuses a second report).
  bool get canReport =>
      const {'delivered', 'completed'}.contains(status) && disputeId == null;
}

class BuyOrdersPage {
  const BuyOrdersPage({required this.items, this.nextCursor});

  factory BuyOrdersPage.fromJson(Map<String, dynamic> j) => BuyOrdersPage(
    items: [for (final o in readObjects(j['items'])) ?BuyOrder.fromJson(o)],
    nextCursor: readString(j['next_cursor']),
  );

  final List<BuyOrder> items;
  final String? nextCursor;
}

@immutable
class TrackStep {
  const TrackStep({
    required this.key,
    required this.label,
    required this.current,
    this.at,
  });

  final String key;
  final String label;
  final bool current;
  final DateTime? at;
}

/// What the buyer needs to follow an order. The handover code and the pickup
/// point are personal: held in memory, never printed.
@immutable
class Tracking {
  const Tracking({
    required this.status,
    required this.steps,
    this.promisedDate,
    this.delayNote,
    this.trackingUrl,
    this.handoverCode,
    this.handoverNote,
    this.pickupBranch,
    this.pickupAddress,
    this.pickupPhone,
    this.courierName,
  });

  factory Tracking.fromJson(Map<String, dynamic> j) {
    final pickup = j['pickup'];
    final contact = j['contact'];
    return Tracking(
      status: readString(j['status']) ?? '',
      promisedDate: readString(j['promised_date']),
      delayNote: readString(j['delay_note']),
      trackingUrl: _https(j['tracking_url']),
      handoverCode: readString(j['handover_code']),
      handoverNote: readString(j['handover_note']),
      pickupBranch: pickup is Map ? readString(pickup['branch_name']) : null,
      pickupAddress: pickup is Map ? readString(pickup['address']) : null,
      pickupPhone: pickup is Map ? readString(pickup['phone']) : null,
      courierName: contact is Map ? readString(contact['name']) : null,
      steps: [
        for (final s in readObjects(j['steps']))
          if (readString(s['key']) case final k?)
            TrackStep(
              key: k,
              label: readString(s['label']) ?? k,
              current: s['current'] == true,
              at: DateTime.tryParse(readString(s['at']) ?? ''),
            ),
      ],
    );
  }

  static String? _https(Object? v) {
    final s = readString(v);
    final u = s == null ? null : Uri.tryParse(s);
    return u != null && u.scheme == 'https' && u.host.isNotEmpty ? s : null;
  }

  final String status;
  final List<TrackStep> steps;
  final String? promisedDate;
  final String? delayNote;
  final String? trackingUrl;
  final String? handoverCode;
  final String? handoverNote;
  final String? pickupBranch;
  final String? pickupAddress;
  final String? pickupPhone;
  final String? courierName;

  @override
  String toString() => 'Tracking($status)';
}

/// The reasons a buyer can give, as the server names them.
enum DisputeReason {
  notReceived('not_received'),
  damaged('damaged'),
  wrongItem('wrong_item'),
  notAsDescribed('not_as_described'),
  incomplete('incomplete'),
  other('other');

  const DisputeReason(this.wire);

  final String wire;
}

@immutable
class BuyDispute {
  const BuyDispute({
    required this.id,
    required this.status,
    required this.respondBy,
    this.sellerAnswer,
    this.returnRequested = false,
    this.decision,
    this.refundKobo,
  });

  static BuyDispute? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    if (id == null) return null;
    final seller = j['seller_statement'];
    final decision = j['decision'];
    return BuyDispute(
      id: id,
      status: readString(j['status']) ?? '',
      respondBy: DateTime.tryParse(readString(j['seller_respond_by']) ?? ''),
      sellerAnswer: seller is Map ? readString(seller['body']) : null,
      returnRequested: j['return_request'] != null,
      decision: decision is Map ? readString(decision['decision']) : null,
      refundKobo: decision is Map ? readInt(decision['refund_kobo']) : null,
    );
  }

  final String id;
  final String status;
  final DateTime? respondBy;
  final String? sellerAnswer;
  final bool returnRequested;
  final String? decision;
  final int? refundKobo;

  /// `awaiting_seller`, `awaiting_return`, `awaiting_decision`, `decided`, `withdrawn`.
  bool get open => status.startsWith('awaiting');

  bool get awaitingSeller => status == 'awaiting_seller';
}

@immutable
class Notice {
  const Notice({
    required this.id,
    required this.kind,
    required this.title,
    required this.body,
    required this.createdAt,
    required this.read,
    this.orderId,
    this.disputeId,
    this.conversationId,
  });

  static Notice? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final created = DateTime.tryParse(readString(j['created_at']) ?? '');
    if (id == null || created == null) return null;
    return Notice(
      id: id,
      kind: readString(j['kind']) ?? '',
      title: readString(j['title']) ?? '',
      body: readString(j['body']) ?? '',
      createdAt: created,
      read: j['read'] == true,
      orderId: readString(j['order_id']),
      disputeId: readString(j['dispute_id']),
      conversationId: readString(j['conversation_id']),
    );
  }

  final String id;
  final String kind;
  final String title;
  final String body;
  final DateTime createdAt;
  final bool read;
  final String? orderId;
  final String? disputeId;
  final String? conversationId;
}

class Inbox {
  const Inbox({required this.items, required this.unread});

  factory Inbox.fromJson(Map<String, dynamic> j) => Inbox(
    items: [for (final n in readObjects(j['items'])) ?Notice.fromJson(n)],
    unread: readInt(j['unread']) ?? 0,
  );

  final List<Notice> items;
  final int unread;
}
