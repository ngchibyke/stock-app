import 'package:flutter/foundation.dart';

import '../../core/json_read.dart';

/// Reads a list of objects out of [json], under [key] when it is a map.
List<Map<String, dynamic>> _rows(Object? json, [String? key]) =>
    readObjects(key == null || json is! Map ? json : json[key]);

/// What the signed-in person is in a store.
enum StoreRole {
  owner,
  manager,
  staff;

  static StoreRole? fromWire(Object? v) {
    for (final r in values) {
      if (r.name == v) return r;
    }
    return null;
  }

  bool get isOwner => this == owner;

  /// Owners and managers answer reviews and disputes and edit products fully.
  bool get isManagerUp => this != staff;
}

/// One of the person's stores with their place in it (`GET /stores/mine`).
/// Staff get the public shape only, so every address field is optional.
@immutable
class SellStore {
  const SellStore({
    required this.id,
    required this.name,
    required this.slug,
    required this.status,
    required this.role,
    this.branchId,
    this.district,
    this.deliveryCourier = false,
    this.deliverySelf = false,
    this.description = '',
  });

  static SellStore? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final role = StoreRole.fromWire(j['my_role']);
    if (id == null || role == null) return null;
    return SellStore(
      id: id,
      name: readString(j['name']) ?? '',
      slug: readString(j['slug']) ?? id,
      status: readString(j['status']) ?? 'pending',
      role: role,
      branchId: readString(j['my_branch_id']),
      district: readString(j['district']),
      deliveryCourier: j['delivery_courier'] == true,
      deliverySelf: j['delivery_self'] == true,
      description: readString(j['description']) ?? '',
    );
  }

  final String id;
  final String name;
  final String slug;

  /// `pending`, `active` or `suspended`.
  final String status;
  final StoreRole role;
  final String? branchId;
  final String? district;
  final bool deliveryCourier;
  final bool deliverySelf;
  final String description;

  bool get isActive => status == 'active';
}

/// The identity check's state (`GET /me/identity`). Never says why a check failed.
enum IdentityState {
  unverified,
  inProgress,
  passed,
  review,
  failed,
  revoked;

  static IdentityState fromWire(Object? v) => switch (v) {
    'in_progress' => inProgress,
    'passed' => passed,
    'review' => review,
    'failed' => failed,
    'revoked' => revoked,
    _ => unverified,
  };
}

@immutable
class IdentityStatus {
  const IdentityStatus({required this.state, this.requiredBy});

  factory IdentityStatus.fromJson(Map<String, dynamic> j) => IdentityStatus(
    state: IdentityState.fromWire(j['status']),
    requiredBy: DateTime.tryParse(readString(j['required_by']) ?? ''),
  );

  final IdentityState state;

  /// When a store inside its grace period stops taking orders.
  final DateTime? requiredBy;
}

/// Settings for the provider's capture SDK. Not secret: the token is what authorises it.
@immutable
class IdentitySdk {
  const IdentitySdk({
    required this.partnerId,
    required this.idType,
    required this.country,
    required this.sandbox,
  });

  final String partnerId;
  final String idType;
  final String country;
  final bool sandbox;
}

/// The provider's job for the selfie. The session token goes to the capture SDK
/// only: never shown, logged or stored.
@immutable
class IdentityStart {
  const IdentityStart({
    required this.attemptId,
    required this.sessionToken,
    this.sdk,
  });

  final String attemptId;
  final String sessionToken;

  /// What the provider's SDK needs besides the token; null when the provider has no
  /// SDK step (the development fake).
  final IdentitySdk? sdk;

  @override
  String toString() => 'IdentityStart(<redacted>)';
}

@immutable
class PayoutAccount {
  const PayoutAccount({
    required this.bankCode,
    required this.last4,
    required this.status,
    this.accountName,
    this.pendingReason,
  });

  factory PayoutAccount.fromJson(Map<String, dynamic> j) => PayoutAccount(
    bankCode: readString(j['bank_code']) ?? '',
    last4: readString(j['last4']) ?? '',
    status: readString(j['status']) ?? 'pending',
    accountName: readString(j['account_name']),
    pendingReason: readString(j['pending_reason']),
  );

  final String bankCode;
  final String last4;
  final String status;
  final String? accountName;
  final String? pendingReason;

  bool get verified => status == 'verified';

  @override
  String toString() => 'PayoutAccount(<redacted>)';
}

/// The fields of a new store. Personal data: never printed.
@immutable
class NewStore {
  const NewStore({
    required this.name,
    required this.description,
    required this.pickupAddress,
    required this.district,
    required this.areaCouncil,
    required this.businessPhone,
    required this.courier,
    required this.selfDelivery,
    this.selfDeliveryFeeKobo,
  });

  final String name;
  final String description;
  final String pickupAddress;
  final String district;
  final String areaCouncil;
  final String businessPhone;
  final bool courier;
  final bool selfDelivery;
  final int? selfDeliveryFeeKobo;

  Map<String, Object?> toJson() => {
    'name': name,
    'description': description,
    'pickup_address': pickupAddress,
    'district': district,
    'pickup_state': 'FCT',
    'delivery_courier': courier,
    'delivery_self': selfDelivery,
    'pickup_area_council': areaCouncil,
    'business_phone': businessPhone,
    if (selfDelivery && selfDeliveryFeeKobo != null)
      'self_delivery_fee_kobo': selfDeliveryFeeKobo,
  };

  @override
  String toString() => 'NewStore(<redacted>)';
}

@immutable
class Branch {
  const Branch({
    required this.id,
    required this.name,
    required this.collectEnabled,
    required this.closed,
    this.areaCouncil,
    this.address,
    this.phone,
    this.postcode,
    this.feeKobo,
  });

  static Branch? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    if (id == null) return null;
    return Branch(
      id: id,
      name: readString(j['name']) ?? '',
      collectEnabled: j['collect_enabled'] == true,
      closed: j['closed_at'] != null,
      areaCouncil: readString(j['pickup_area_council']),
      address: readString(j['pickup_address']),
      phone: readString(j['business_phone']),
      postcode: readString(j['pickup_postcode']),
      feeKobo: readInt(j['self_delivery_fee_kobo']),
    );
  }

  final String id;
  final String name;
  final bool collectEnabled;
  final bool closed;
  final String? areaCouncil;

  /// Only owners and managers get the details below.
  final String? address;
  final String? phone;
  final String? postcode;
  final int? feeKobo;

  @override
  String toString() => 'Branch($id)';
}

/// A branch's fields on create or edit. Absent fields stay as they were.
@immutable
class BranchInput {
  const BranchInput({
    this.name,
    this.address,
    this.areaCouncil,
    this.phone,
    this.feeKobo,
    this.collectEnabled,
  });

  final String? name;
  final String? address;
  final String? areaCouncil;
  final String? phone;
  final int? feeKobo;
  final bool? collectEnabled;

  Map<String, Object?> toJson() => {
    if (name != null) 'name': name,
    if (address != null) 'pickup_address': address,
    if (areaCouncil != null) 'pickup_area_council': areaCouncil,
    if (phone != null) 'business_phone': phone,
    if (feeKobo != null) 'self_delivery_fee_kobo': feeKobo,
    if (collectEnabled != null) 'collect_enabled': collectEnabled,
  };

  @override
  String toString() => 'BranchInput(<redacted>)';
}

@immutable
class BranchRef {
  const BranchRef({required this.id, required this.name});

  static BranchRef? fromJson(Object? j) {
    if (j is! Map<String, dynamic>) return null;
    final id = readString(j['id']);
    return id == null
        ? null
        : BranchRef(id: id, name: readString(j['name']) ?? '');
  }

  final String id;
  final String name;
}

@immutable
class Member {
  const Member({
    required this.userId,
    required this.name,
    required this.role,
    this.branch,
  });

  final String userId;
  final String name;
  final StoreRole role;
  final BranchRef? branch;
}

@immutable
class PendingInvite {
  const PendingInvite({
    required this.id,
    required this.last4,
    required this.role,
    this.branch,
  });

  final String id;
  final String last4;
  final StoreRole role;
  final BranchRef? branch;
}

@immutable
class Team {
  const Team({required this.members, required this.invites});

  factory Team.fromJson(Map<String, dynamic> j) => Team(
    members: [
      for (final m in _rows(j, 'members'))
        if (readString(m['user_id']) case final id?)
          if (StoreRole.fromWire(m['role']) case final role?)
            Member(
              userId: id,
              name: readString(m['display_name']) ?? '',
              role: role,
              branch: BranchRef.fromJson(m['branch']),
            ),
    ],
    invites: [
      for (final i in _rows(j, 'invites'))
        if (readString(i['id']) case final id?)
          if (StoreRole.fromWire(i['role']) case final role?)
            PendingInvite(
              id: id,
              last4: readString(i['phone_last4']) ?? '',
              role: role,
              branch: BranchRef.fromJson(i['branch']),
            ),
    ],
  );

  final List<Member> members;
  final List<PendingInvite> invites;
}

/// An invitation to join a store, as the invited person sees it.
@immutable
class MyInvite {
  const MyInvite({
    required this.id,
    required this.storeName,
    required this.role,
    this.branchName,
  });

  static MyInvite? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final role = StoreRole.fromWire(j['role']);
    if (id == null || role == null) return null;
    final store = j['store'];
    return MyInvite(
      id: id,
      storeName: store is Map ? readString(store['name']) ?? '' : '',
      role: role,
      branchName: BranchRef.fromJson(j['branch'])?.name,
    );
  }

  final String id;
  final String storeName;
  final StoreRole role;
  final String? branchName;
}
