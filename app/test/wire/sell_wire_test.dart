// Selling wire check: the seller's path, driven with the app's real repositories
// against a REAL local server: access code, identity (the server's fake provider),
// store, payout, branches, team, invitations, products, orders and reviews.
// The screens are covered by widget tests; this proves the wire.
//
// Skipped unless WIRE_BASE_URL and WIRE_OTP_LOG are set (see "Wire check" in
// README.md). Also needs WIRE_DATABASE_URL, the scratch database's URL: the check
// promotes one fresh account to Owner there (to issue an access code) and reads
// nothing else. Start the server with IDENTITY_PROVIDER=fake and a NIN_INDEX_KEY.
import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/sell/listing_models.dart';
import 'package:market/features/sell/order_models.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_repository.dart';

import 'wire_support.dart';

const _identitySecret = 'fake-identity-webhook-secret';
final _databaseUrl = Platform.environment['WIRE_DATABASE_URL'] ?? '';

class _Person {
  _Person(this.api, this.id, this.phone, this.password);

  final ApiClient api;
  final String id;
  final String phone;
  final String password;
  late final sell = ApiSellRepository(api);
  late final work = ApiWorkRepository(api);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() => HttpOverrides.global = null);

  Future<_Person> signUp(String name) async {
    String? token;
    final api = ApiClient(
      baseUrl: wireBaseUrl,
      readToken: () async => token,
      onUnauthorized: () {},
    );
    final auth = ApiAuthRepository(api);
    final phone = nextPhone();
    final password = freshPassword();
    late CodeChallenge challenge;
    final code = await codeAfter(() async {
      challenge = await auth.signUp(
        phone: phone,
        password: password,
        displayName: name,
      );
    });
    token = await auth.verifyPhone(
      verificationId: challenge.verificationId,
      code: code,
    );
    final me = await api.get('/me') as Map<String, dynamic>;
    return _Person(api, me['id'] as String, phone, password);
  }

  /// POSTs the fake provider's signed result for [providerRef].
  Future<int> deliver(String providerRef, Map<String, Object?> outcome) async {
    final body = utf8.encode(jsonEncode({'ref': providerRef, ...outcome}));
    final sig = Hmac(
      sha256,
      utf8.encode(_identitySecret),
    ).convert(body).toString();
    final client = HttpClient();
    try {
      final req = await client.postUrl(
        Uri.parse('$wireBaseUrl/webhooks/identity/fake'),
      );
      req.headers.set('x-identity-signature', sig);
      req.headers.contentType = ContentType.json;
      req.add(body);
      final res = await req.close();
      await res.drain<void>();
      return res.statusCode;
    } finally {
      client.close(force: true);
    }
  }

  // A different NIN each run: a NIN that has passed is taken for good, so a re-run on
  // the same scratch database would be refused.
  final nin =
      (10000000000 + DateTime.now().microsecondsSinceEpoch % 80000000000)
          .toString();

  test(
    'a_seller_goes_from_a_code_to_a_store_a_team_and_an_empty_order_list',
    () async {
      if (!isLoopbackBaseUrl(wireBaseUrl)) {
        fail('the wire check only talks to a loopback server');
      }
      if (_databaseUrl.isEmpty) {
        fail('set WIRE_DATABASE_URL to the scratch database');
      }

      // An Owner issues an access code.
      final owner = await signUp('Wire Owner');
      final promote = await Process.run('psql', [
        _databaseUrl,
        '-v',
        'ON_ERROR_STOP=1',
        '-c',
        "UPDATE users SET role = 'owner' WHERE id = '${owner.id}'",
      ]);
      expect(promote.exitCode, 0, reason: '${promote.stderr}');
      final issued = await owner.api.post(
        '/admin/access-codes',
        body: {'label': 'wire', 'max_uses': 1},
      ) as Map<String, dynamic>;
      final code = issued['code'] as String;

      // A person becomes a seller.
      final seller = await signUp('Ada');
      await expectLater(
        seller.sell.myStores(),
        throwsA(isA<ApiError>().having((e) => e.code, 'code', 'not_a_seller')),
        reason: 'a buyer has no seller side yet',
      );
      await seller.sell.redeemCode(code);

      // Identity, through the server's fake provider and its signed webhook.
      expect(
        (await seller.sell.identityStatus()).state,
        IdentityState.unverified,
      );
      await expectLater(
        seller.sell.startIdentity('1234'),
        throwsA(isA<ApiError>().having((e) => e.status, 'status', 422)),
      );
      final start = await seller.sell.startIdentity(nin);
      expect(start.sessionToken, isNotEmpty);
      expect(
        (await seller.sell.identityStatus()).state,
        IdentityState.inProgress,
      );
      expect(
        await deliver('fake-job-${start.attemptId}', {
          'outcome': 'passed',
          'full_name': 'Ada Obi',
          'dob': '1990-01-01',
        }),
        200,
      );
      expect((await seller.sell.identityStatus()).state, IdentityState.passed);

      // The same NIN on another account is refused the same plain way.
      final other = await signUp('Other');
      final otherCode =
          (await owner.api.post(
                '/admin/access-codes',
                body: {'label': 'wire2', 'max_uses': 1},
              ) as Map<String, dynamic>)['code']
              as String;
      await expectLater(
        other.sell.redeemCode(code),
        throwsA(
          isA<ApiError>().having((e) => e.code, 'code', 'code_exhausted'),
        ),
        reason: 'a used up code is refused',
      );
      await other.sell.redeemCode(otherCode);
      await expectLater(
        other.sell.startIdentity(nin),
        throwsA(
          isA<ApiError>().having(
            (e) => e.code,
            'code',
            'identity_not_accepted',
          ),
        ),
      );

      // A store, with the pickup as its first branch.
      final created = await seller.sell.createStore(
        const NewStore(
          name: 'Wire Boutique',
          description: 'Cloth and things',
          pickupAddress: '12 Quiet Close, Wuse 2',
          district: 'fashion',
          areaCouncil: 'amac',
          businessPhone: '08031234567',
          courier: true,
          selfDelivery: true,
          selfDeliveryFeeKobo: 100000,
        ),
      );
      expect(created.role, StoreRole.owner);
      final stores = await seller.sell.myStores();
      expect(stores.single.id, created.id);
      expect(
        stores.single.status,
        'pending',
        reason: 'no verified payout account yet',
      );

      // The payout account: the local server has no bank lookup, so it waits.
      expect(await seller.sell.payoutAccount(created.id), isNull);
      final payout = await seller.sell.setPayoutAccount(
        created.id,
        bankCode: '058',
        accountNumber: '0123456785',
      );
      expect(payout.verified, isFalse);
      expect(payout.last4, '6785');

      // Branches.
      var branches = await seller.sell.branches(created.id);
      expect(branches, hasLength(1));
      expect(branches.single.address, '12 Quiet Close, Wuse 2');
      final second = await seller.sell.addBranch(
        created.id,
        const BranchInput(
          name: 'Garki',
          address: '5 Market Road, Garki',
          areaCouncil: 'amac',
          phone: '08039876543',
          feeKobo: 150000,
          collectEnabled: false,
        ),
      );
      await seller.sell.editBranch(
        created.id,
        second.id,
        const BranchInput(collectEnabled: true),
      );
      branches = await seller.sell.branches(created.id);
      expect(
        branches.firstWhere((b) => b.id == second.id).collectEnabled,
        isTrue,
      );

      // A team member: invite by phone, accept from their own account.
      final member = await signUp('Amina');
      await seller.sell.invite(
        created.id,
        phone: member.phone,
        role: StoreRole.staff,
        branchId: second.id,
      );
      final team = await seller.sell.team(created.id);
      expect(team.invites, hasLength(1));
      final mine = await member.sell.myInvites();
      expect(mine.single.storeName, 'Wire Boutique');
      await member.sell.answerInvite(mine.single.id, accept: true);
      final memberStores = await member.sell.myStores();
      expect(memberStores.single.role, StoreRole.staff);
      expect(memberStores.single.branchId, second.id);
      expect((await seller.sell.team(created.id)).members.single.name, 'Amina');
      // Staff may look at the orders; until they confirm who they are they may not act.
      expect((await member.work.orders(created.id)).items, isEmpty);

      // Products: this server has no photo store, so a photo cannot be uploaded and the
      // server's own rule (1 to 10 photos) refuses a product with none. The list is empty.
      final categories =
          await seller.api.get('/categories', auth: false) as List;
      final category = (categories.first as Map)['slug'] as String;
      await expectLater(
        seller.work.createListing(
          created.id,
          ListingInput(
            name: 'Wire Ankara Dress',
            description: 'A dress',
            category: category,
            condition: 'new',
            priceKobo: 1500000,
            photoKeys: const [],
            variants: [
              VariantInput(
                attributes: const {'option': 'M'},
                stock: {branches.first.id: 3, second.id: 2},
              ),
            ],
          ),
          singleBranch: false,
        ),
        throwsA(isA<ApiError>().having((e) => e.status, 'status', 422)),
      );
      expect(await seller.work.listings(created.slug), isEmpty);
      await expectLater(
        seller.work.newUpload(
          purpose: 'product',
          contentType: 'image/jpeg',
          bytes: 1000,
        ),
        throwsA(
          isA<ApiError>().having((e) => e.code, 'code', 'uploads_unavailable'),
        ),
      );

      // Orders and reviews are empty, and they parse.
      final page = await seller.work.orders(created.id);
      expect(page.items, isEmpty);
      expect(page.nextCursor, isNull);
      expect(await seller.work.reviews(created.slug), isEmpty);
      expect(OrderStep.values, isNotEmpty);
    },
    skip: wireSkip,
    timeout: const Timeout(Duration(minutes: 2)),
  );
}
