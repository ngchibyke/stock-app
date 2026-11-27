import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';
import '../../core/json_read.dart';
import 'listing_models.dart';
import 'order_models.dart';
import 'sell_models.dart';

ApiError _badResponse() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);

/// Everything the seller side asks of the server. Personal data (addresses,
/// phones, the NIN) passes through and is kept nowhere.
abstract class SellRepository {
  /// Redeems an access code; the person becomes a seller.
  Future<void> redeemCode(String code);

  Future<IdentityStatus> identityStatus();

  /// Throws an [ApiError]: 422 `validation_failed`; 409 `identity_in_progress`,
  /// `identity_in_review`, `identity_already_verified`, `identity_not_accepted`;
  /// 429 `identity_rate_limited`; 503 `identity_unavailable`.
  Future<IdentityStart> startIdentity(String nin);

  Future<List<SellStore>> myStores();
  Future<SellStore> createStore(NewStore store);
  Future<PayoutAccount?> payoutAccount(String storeId);

  /// 403 `password_required`-style errors come back as an [ApiError].
  Future<PayoutAccount> setPayoutAccount(
    String storeId, {
    required String bankCode,
    required String accountNumber,
    String? password,
  });

  Future<void> setStoreDetails(
    String storeId, {
    String? name,
    String? description,
    bool? courier,
    bool? self,
  });

  Future<List<Branch>> branches(String storeId);
  Future<Branch> addBranch(String storeId, BranchInput input);
  Future<Branch> editBranch(String storeId, String branchId, BranchInput input);
  Future<void> closeBranch(String storeId, String branchId);

  Future<Team> team(String storeId);
  Future<void> invite(
    String storeId, {
    required String phone,
    required StoreRole role,
    String? branchId,
  });
  Future<void> revokeInvite(String storeId, String inviteId);
  Future<void> removeMember(String storeId, String userId);

  Future<List<MyInvite>> myInvites();
  Future<void> answerInvite(String inviteId, {required bool accept});
}

class ApiSellRepository implements SellRepository {
  ApiSellRepository(this._api);

  final ApiClient _api;

  static String _id(String id) => Uri.encodeComponent(id);

  Map<String, dynamic> _object(Object? json) =>
      json is Map<String, dynamic> ? json : throw _badResponse();

  @override
  Future<void> redeemCode(String code) async {
    await _api.post('/seller/redeem', body: {'code': code});
  }

  @override
  Future<IdentityStatus> identityStatus() async =>
      IdentityStatus.fromJson(_object(await _api.get('/me/identity')));

  @override
  Future<IdentityStart> startIdentity(String nin) async {
    final j = _object(
      await _api.post('/me/identity/start', body: {'nin': nin}),
    );
    final attempt = readString(j['attempt_id']);
    final token = readString(j['session_token']);
    if (attempt == null || token == null) throw _badResponse();
    return IdentityStart(
      attemptId: attempt,
      sessionToken: token,
      sdk: _sdk(j['sdk']),
    );
  }

  IdentitySdk? _sdk(Object? raw) {
    if (raw is! Map<String, dynamic>) return null;
    final partner = readString(raw['partner_id']);
    final type = readString(raw['id_type']);
    final country = readString(raw['country']);
    final sandbox = raw['sandbox'];
    if (partner == null ||
        type == null ||
        country == null ||
        sandbox is! bool) {
      return null;
    }
    return IdentitySdk(
      partnerId: partner,
      idType: type,
      country: country,
      sandbox: sandbox,
    );
  }

  @override
  Future<List<SellStore>> myStores() async {
    final json = await _api.get('/stores/mine');
    if (json is! List) throw _badResponse();
    return [for (final r in readObjects(json)) ?SellStore.fromJson(r)];
  }

  @override
  Future<SellStore> createStore(NewStore store) async {
    final j = _object(await _api.post('/stores', body: store.toJson()));
    // The create answer has no role: the creator owns it.
    final created = SellStore.fromJson({...j, 'my_role': 'owner'});
    return created ?? (throw _badResponse());
  }

  @override
  Future<PayoutAccount?> payoutAccount(String storeId) async {
    try {
      return PayoutAccount.fromJson(
        _object(await _api.get('/stores/${_id(storeId)}/payout-account')),
      );
    } on ApiError catch (e) {
      if (e.status == 404) return null;
      rethrow;
    }
  }

  @override
  Future<PayoutAccount> setPayoutAccount(
    String storeId, {
    required String bankCode,
    required String accountNumber,
    String? password,
  }) async => PayoutAccount.fromJson(
    _object(
      await _api.put(
        '/stores/${_id(storeId)}/payout-account',
        body: {
          'bank_code': bankCode,
          'account_number': accountNumber,
          if (password != null && password.isNotEmpty) 'password': password,
        },
      ),
    ),
  );

  @override
  Future<void> setStoreDetails(
    String storeId, {
    String? name,
    String? description,
    bool? courier,
    bool? self,
  }) async {
    await _api.patch(
      '/stores/${_id(storeId)}',
      body: {
        'name': ?name,
        'description': ?description,
        'delivery_courier': ?courier,
        'delivery_self': ?self,
      },
    );
  }

  @override
  Future<List<Branch>> branches(String storeId) async {
    final json = await _api.get('/stores/${_id(storeId)}/branches');
    final rows = json is Map ? json['branches'] ?? json['items'] : json;
    return [for (final r in readObjects(rows)) ?Branch.fromJson(r)];
  }

  @override
  Future<Branch> addBranch(String storeId, BranchInput input) async =>
      Branch.fromJson(
        _object(
          await _api.post(
            '/stores/${_id(storeId)}/branches',
            body: input.toJson(),
          ),
        ),
      ) ??
      (throw _badResponse());

  @override
  Future<Branch> editBranch(
    String storeId,
    String branchId,
    BranchInput input,
  ) async =>
      Branch.fromJson(
        _object(
          await _api.patch(
            '/stores/${_id(storeId)}/branches/${_id(branchId)}',
            body: input.toJson(),
          ),
        ),
      ) ??
      (throw _badResponse());

  @override
  Future<void> closeBranch(String storeId, String branchId) async {
    await _api.post('/stores/${_id(storeId)}/branches/${_id(branchId)}/close');
  }

  @override
  Future<Team> team(String storeId) async =>
      Team.fromJson(_object(await _api.get('/stores/${_id(storeId)}/team')));

  @override
  Future<void> invite(
    String storeId, {
    required String phone,
    required StoreRole role,
    String? branchId,
  }) async {
    await _api.post(
      '/stores/${_id(storeId)}/invites',
      body: {'phone': phone, 'role': role.name, 'branch_id': ?branchId},
    );
  }

  @override
  Future<void> revokeInvite(String storeId, String inviteId) async {
    await _api.delete('/stores/${_id(storeId)}/invites/${_id(inviteId)}');
  }

  @override
  Future<void> removeMember(String storeId, String userId) async {
    await _api.delete('/stores/${_id(storeId)}/team/${_id(userId)}');
  }

  @override
  Future<List<MyInvite>> myInvites() async {
    final json = await _api.get('/me/invites');
    return [
      for (final r in readObjects(
        json is Map ? json['invites'] ?? json['items'] : json,
      ))
        ?MyInvite.fromJson(r),
    ];
  }

  @override
  Future<void> answerInvite(String inviteId, {required bool accept}) async {
    await _api.post(
      '/me/invites/${_id(inviteId)}/${accept ? 'accept' : 'decline'}',
    );
  }
}

final sellRepositoryProvider = Provider<SellRepository>(
  (ref) => ApiSellRepository(ref.watch(apiClientProvider)),
);

/// What the app needs from products, orders, reviews and disputes.
abstract class WorkRepository {
  /// A store's products, by its slug (the server lists by slug, creates by id).
  Future<List<Listing>> listings(String storeSlug);
  Future<Listing> createListing(
    String storeId,
    ListingInput input, {
    required bool singleBranch,
  });
  Future<Listing> editListing(String id, ListingInput input);
  Future<void> setPublished(String id, bool published);
  Future<void> deleteListing(String id);
  Future<void> setStock(String branchId, Map<String, int> stockByVariant);

  Future<UploadTicket> newUpload({
    required String purpose,
    required String contentType,
    required int bytes,
  });
  Future<String> finishUpload(String uploadId);

  Future<OrdersPage> orders(String storeId, {String? before});
  Future<SellOrder> orderStep(
    String orderId,
    OrderStep step, {
    String? handoverCode,
  });

  Future<List<SellReview>> reviews(String storeSlug);
  Future<void> replyToReview(String reviewId, String body);

  Future<SellDispute> dispute(String disputeId);
  Future<SellDispute> respondToDispute(
    String disputeId,
    String body,
    List<String> photoKeys,
  );
  Future<SellDispute> returnReceived(String disputeId);

  /// A link to one of the dispute's photos that works for about a minute.
  Future<String> disputePhotoUrl(String disputeId, String key);
}

class ApiWorkRepository implements WorkRepository {
  ApiWorkRepository(this._api);

  final ApiClient _api;

  static String _id(String id) => Uri.encodeComponent(id);

  Map<String, dynamic> _object(Object? json) =>
      json is Map<String, dynamic> ? json : throw _badResponse();

  @override
  Future<List<Listing>> listings(String key) async {
    final json = await _api.get('/stores/${_id(key)}/products');
    if (json is! List) throw _badResponse();
    return [for (final r in readObjects(json)) ?Listing.fromJson(r)];
  }

  @override
  Future<Listing> createListing(
    String storeId,
    ListingInput input, {
    required bool singleBranch,
  }) async =>
      Listing.fromJson(
        _object(
          await _api.post(
            '/stores/${_id(storeId)}/products',
            body: input.toCreateJson(singleBranch),
          ),
        ),
      ) ??
      (throw _badResponse());

  @override
  Future<Listing> editListing(String id, ListingInput input) async =>
      Listing.fromJson(
        _object(
          await _api.patch('/products/${_id(id)}', body: input.toPatchJson()),
        ),
      ) ??
      (throw _badResponse());

  @override
  Future<void> setPublished(String id, bool published) async {
    await _api.patch('/products/${_id(id)}', body: {'is_published': published});
  }

  @override
  Future<void> deleteListing(String id) async {
    await _api.delete('/products/${_id(id)}');
  }

  @override
  Future<void> setStock(
    String branchId,
    Map<String, int> stockByVariant,
  ) async {
    await _api.put(
      '/branches/${_id(branchId)}/stock',
      body: {
        'items': [
          for (final e in stockByVariant.entries)
            {'variant_id': e.key, 'stock': e.value},
        ],
      },
    );
  }

  @override
  Future<UploadTicket> newUpload({
    required String purpose,
    required String contentType,
    required int bytes,
  }) async {
    final j = _object(
      await _api.post(
        '/uploads',
        body: {'purpose': purpose, 'content_type': contentType, 'bytes': bytes},
      ),
    );
    final id = readString(j['upload_id']);
    final url = readString(j['url']);
    if (id == null || url == null) throw _badResponse();
    final headers = j['headers'];
    return UploadTicket(
      uploadId: id,
      url: url,
      headers: {
        if (headers is Map)
          for (final e in headers.entries)
            if (e.key is String && e.value is String)
              e.key as String: e.value as String,
      },
    );
  }

  @override
  Future<String> finishUpload(String uploadId) async {
    final j = _object(await _api.post('/uploads/${_id(uploadId)}/finalize'));
    return readString(j['key']) ?? (throw _badResponse());
  }

  @override
  Future<OrdersPage> orders(String storeId, {String? before}) async =>
      OrdersPage.fromJson(
        _object(
          await _api.get(
            '/stores/${_id(storeId)}/orders',
            query: {'limit': 25, 'before': ?before},
          ),
        ),
      );

  @override
  Future<SellOrder> orderStep(
    String orderId,
    OrderStep step, {
    String? handoverCode,
  }) async {
    final path = switch (step) {
      OrderStep.ready => 'ready',
      OrderStep.pickedUp => 'picked-up',
      OrderStep.outForDelivery => 'out-for-delivery',
      OrderStep.delivered => 'delivered',
    };
    final j = _object(
      await _api.post(
        '/orders/${_id(orderId)}/$path',
        body: handoverCode == null ? null : {'handover_code': handoverCode},
      ),
    );
    return SellOrder.fromJson(j) ?? (throw _badResponse());
  }

  @override
  Future<List<SellReview>> reviews(String storeSlug) async {
    final j = _object(await _api.get('/stores/${_id(storeSlug)}/reviews'));
    return [for (final r in readObjects(j['reviews'])) ?SellReview.fromJson(r)];
  }

  @override
  Future<void> replyToReview(String reviewId, String body) async {
    await _api.post('/reviews/${_id(reviewId)}/reply', body: {'body': body});
  }

  @override
  Future<SellDispute> dispute(String disputeId) async =>
      SellDispute.fromJson(
        _object(await _api.get('/disputes/${_id(disputeId)}')),
      ) ??
      (throw _badResponse());

  @override
  Future<SellDispute> respondToDispute(
    String disputeId,
    String body,
    List<String> photoKeys,
  ) async =>
      SellDispute.fromJson(
        _object(
          await _api.post(
            '/disputes/${_id(disputeId)}/response',
            body: {'body': body, 'photo_keys': photoKeys},
          ),
        ),
      ) ??
      (throw _badResponse());

  @override
  Future<String> disputePhotoUrl(String disputeId, String key) async {
    final j = _object(
      await _api.get('/disputes/${_id(disputeId)}/photo', query: {'key': key}),
    );
    return readString(j['url']) ?? (throw _badResponse());
  }

  @override
  Future<SellDispute> returnReceived(String disputeId) async =>
      SellDispute.fromJson(
        _object(await _api.post('/disputes/${_id(disputeId)}/return-received')),
      ) ??
      (throw _badResponse());
}

final workRepositoryProvider = Provider<WorkRepository>(
  (ref) => ApiWorkRepository(ref.watch(apiClientProvider)),
);
