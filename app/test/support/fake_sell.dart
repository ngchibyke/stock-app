import 'package:market/core/api/api_error.dart';
import 'package:market/features/sell/listing_models.dart';
import 'package:market/features/sell/order_models.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_repository.dart';

ApiError apiError(String code, {int status = 409}) =>
    ApiError(status: status, code: code, message: code);

/// An in-memory [SellRepository] that records what it was asked.
class FakeSellRepository implements SellRepository {
  List<SellStore> stores = [];
  IdentityStatus identity = const IdentityStatus(
    state: IdentityState.unverified,
  );
  PayoutAccount? payout;
  List<Branch> branchRows = [];
  Team teamRows = const Team(members: [], invites: []);
  List<MyInvite> invitesRows = [];

  /// Set to make a call fail.
  Object? failRedeem;
  Object? failStart;
  Object? failCreate;
  Object? failPayout;
  Object? failBranch;

  final List<String> redeemed = [];
  final List<String> ninsSent = [];
  final List<NewStore> created = [];
  final List<Map<String, Object?>> payoutSet = [];
  final List<Map<String, Object?>> invited = [];
  final List<Map<String, Object?>> branchSaved = [];
  final List<String> closed = [];
  final List<String> answered = [];
  int identityCalls = 0;

  @override
  Future<void> redeemCode(String code) async {
    redeemed.add(code);
    if (failRedeem != null) throw failRedeem!;
  }

  @override
  Future<IdentityStatus> identityStatus() async {
    identityCalls++;
    return identity;
  }

  @override
  Future<IdentityStart> startIdentity(String nin) async {
    ninsSent.add(nin);
    if (failStart != null) throw failStart!;
    return const IdentityStart(attemptId: 'a1', sessionToken: 'tok');
  }

  @override
  Future<List<SellStore>> myStores() async => [...stores];

  @override
  Future<SellStore> createStore(NewStore store) async {
    created.add(store);
    if (failCreate != null) throw failCreate!;
    final made = SellStore(
      id: 'st${created.length}',
      name: store.name,
      slug: 'slug${created.length}',
      status: 'pending',
      role: StoreRole.owner,
    );
    stores = [...stores, made];
    return made;
  }

  @override
  Future<PayoutAccount?> payoutAccount(String storeId) async => payout;

  @override
  Future<PayoutAccount> setPayoutAccount(
    String storeId, {
    required String bankCode,
    required String accountNumber,
    String? password,
  }) async {
    payoutSet.add({
      'bank': bankCode,
      'number': accountNumber,
      'password': password,
    });
    if (failPayout != null) throw failPayout!;
    return payout = PayoutAccount(
      bankCode: bankCode,
      last4: accountNumber.substring(6),
      status: 'verified',
      accountName: 'ADA OBI',
    );
  }

  @override
  Future<void> setStoreDetails(
    String storeId, {
    String? name,
    String? description,
    bool? courier,
    bool? self,
  }) async {}

  @override
  Future<List<Branch>> branches(String storeId) async => [...branchRows];

  @override
  Future<Branch> addBranch(String storeId, BranchInput input) async {
    branchSaved.add(input.toJson());
    if (failBranch != null) throw failBranch!;
    final b = Branch(
      id: 'b${branchRows.length + 1}',
      name: input.name ?? '',
      collectEnabled: input.collectEnabled ?? false,
      closed: false,
    );
    branchRows = [...branchRows, b];
    return b;
  }

  @override
  Future<Branch> editBranch(
    String storeId,
    String branchId,
    BranchInput input,
  ) async {
    branchSaved.add({'id': branchId, ...input.toJson()});
    return branchRows.firstWhere((b) => b.id == branchId);
  }

  @override
  Future<void> closeBranch(String storeId, String branchId) async {
    closed.add(branchId);
    if (failBranch != null) throw failBranch!;
  }

  @override
  Future<Team> team(String storeId) async => teamRows;

  @override
  Future<void> invite(
    String storeId, {
    required String phone,
    required StoreRole role,
    String? branchId,
  }) async {
    invited.add({'phone': phone, 'role': role.name, 'branch': branchId});
  }

  @override
  Future<void> revokeInvite(String storeId, String inviteId) async {}

  @override
  Future<void> removeMember(String storeId, String userId) async {}

  @override
  Future<List<MyInvite>> myInvites() async => [...invitesRows];

  @override
  Future<void> answerInvite(String inviteId, {required bool accept}) async {
    answered.add('$inviteId:${accept ? 'accept' : 'decline'}');
  }
}

/// An in-memory [WorkRepository].
class FakeWorkRepository implements WorkRepository {
  List<Listing> listingRows = [];
  List<SellOrder> orderRows = [];
  List<SellReview> reviewRows = [];
  SellDispute? disputeRow;
  Object? failStep;

  final List<ListingInput> created = [];
  final List<String> steps = [];
  final List<String?> codes = [];
  final List<String> replies = [];
  final List<String> uploaded = [];
  final List<String> answers = [];

  @override
  Future<List<Listing>> listings(String storeSlugOrId) async => [
    ...listingRows,
  ];

  @override
  Future<Listing> createListing(
    String storeId,
    ListingInput input, {
    required bool singleBranch,
  }) async {
    created.add(input);
    return Listing(
      id: 'p${created.length}',
      name: input.name,
      description: input.description,
      category: input.category,
      condition: input.condition,
      basePriceKobo: input.priceKobo,
      published: false,
      photoKeys: input.photoKeys,
      photoUrls: const [],
      variants: const [],
    );
  }

  @override
  Future<Listing> editListing(String id, ListingInput input) async =>
      createListing('', input, singleBranch: true);

  @override
  Future<void> setPublished(String id, bool published) async {}

  @override
  Future<void> deleteListing(String id) async {}

  @override
  Future<void> setStock(
    String branchId,
    Map<String, int> stockByVariant,
  ) async {}

  @override
  Future<UploadTicket> newUpload({
    required String purpose,
    required String contentType,
    required int bytes,
  }) async => UploadTicket(
    uploadId: 'u${uploaded.length + 1}',
    url: 'https://files.test/put',
    headers: {'Content-Type': contentType},
  );

  @override
  Future<String> finishUpload(String uploadId) async {
    uploaded.add(uploadId);
    return 'key/$uploadId.jpg';
  }

  @override
  Future<OrdersPage> orders(String storeId, {String? before}) async =>
      OrdersPage(items: [...orderRows]);

  @override
  Future<SellOrder> orderStep(
    String orderId,
    OrderStep step, {
    String? handoverCode,
  }) async {
    steps.add('$orderId:${step.name}');
    codes.add(handoverCode);
    if (failStep != null) throw failStep!;
    final o = orderRows.firstWhere((o) => o.id == orderId);
    final next = SellOrder(
      id: o.id,
      status: switch (step) {
        OrderStep.ready => 'paid',
        OrderStep.pickedUp => 'picked_up',
        OrderStep.outForDelivery => 'out_for_delivery',
        OrderStep.delivered => 'delivered',
      },
      method: o.method,
      branchName: o.branchName,
      lines: o.lines,
      totalKobo: o.totalKobo,
      createdAt: o.createdAt,
      doneBy: step == OrderStep.ready ? {'ready': 'Ada'} : o.doneBy,
    );
    orderRows = [for (final x in orderRows) x.id == orderId ? next : x];
    return next;
  }

  @override
  Future<List<SellReview>> reviews(String storeSlug) async => [...reviewRows];

  @override
  Future<void> replyToReview(String reviewId, String body) async {
    replies.add('$reviewId:$body');
  }

  @override
  Future<SellDispute> dispute(String disputeId) async => disputeRow!;

  @override
  Future<SellDispute> respondToDispute(
    String disputeId,
    String body,
    List<String> photoKeys,
  ) async {
    answers.add(body);
    return disputeRow!;
  }

  @override
  Future<SellDispute> returnReceived(String disputeId) async => disputeRow!;

  @override
  Future<String> disputePhotoUrl(String disputeId, String key) async =>
      'https://files.test/$key';
}
