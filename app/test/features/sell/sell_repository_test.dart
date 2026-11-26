import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/sell/listing_models.dart';
import 'package:market/features/sell/order_models.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_repository.dart';

import '../../support/fakes.dart';

ApiClient _client(FakeAdapter a) => ApiClient(
  baseUrl: 'https://api.test',
  readToken: () async => 'tok',
  onUnauthorized: () {},
  adapter: a,
);

void main() {
  test('the_nin_goes_in_the_body_never_in_the_path_or_query', () async {
    final a = FakeAdapter()
      ..reply(200, {
        'attempt_id': 'a1',
        'provider': 'fake',
        'session_token': 'tok',
        'expires_in_secs': 1800,
      });
    final start = await ApiSellRepository(_client(a))
        .startIdentity('12345678901');
    final r = a.requests.single;
    expect(r.method, 'POST');
    expect(r.path, '/me/identity/start');
    expect(r.query, isEmpty);
    expect(r.path, isNot(contains('1234')));
    expect(r.body, {'nin': '12345678901'});
    expect(start.attemptId, 'a1');
    expect('$start', isNot(contains('tok')));
  });

  test('a_refused_start_surfaces_the_server_code', () async {
    final a = FakeAdapter()
      ..reply(409, {
        'error': {
          'code': 'identity_not_accepted',
          'message': 'We could not verify this NIN. Contact support.',
        },
      });
    await expectLater(
      ApiSellRepository(_client(a)).startIdentity('12345678901'),
      throwsA(
        isA<ApiError>().having((e) => e.code, 'code', 'identity_not_accepted'),
      ),
    );
  });

  test('redeem_status_and_stores', () async {
    final a = FakeAdapter()
      ..reply(200, {'ok': true})
      ..reply(200, {'status': 'passed'})
      ..reply(200, [
        {
          'id': 's1',
          'name': 'Shop',
          'slug': 'shop',
          'status': 'active',
          'my_role': 'owner',
        },
        {'id': 's2', 'my_role': 'weird'},
        'junk',
      ])
      ..reply(403, {
        'error': {'code': 'not_a_seller', 'message': 'x'},
      });
    final repo = ApiSellRepository(_client(a));
    await repo.redeemCode('ABCD1234');
    expect(a.requests[0].path, '/seller/redeem');
    expect(a.requests[0].body, {'code': 'ABCD1234'});
    expect((await repo.identityStatus()).state, IdentityState.passed);
    final stores = await repo.myStores();
    expect(stores.map((s) => s.id), ['s1']);
    await expectLater(repo.myStores(), throwsA(isA<ApiError>()));
  });

  test('create_store_sends_the_pickup_as_the_first_branch_and_reads_the_owner_role', () async {
    final a = FakeAdapter()
      ..reply(201, {
        'id': 's1',
        'name': 'Shop',
        'slug': 'shop',
        'status': 'pending',
      });
    final s = await ApiSellRepository(_client(a)).createStore(
      const NewStore(
        name: 'Shop',
        description: 'Things',
        pickupAddress: '12 Quiet Close',
        district: 'fashion',
        areaCouncil: 'amac',
        businessPhone: '08031234567',
        courier: true,
        selfDelivery: true,
        selfDeliveryFeeKobo: 150000,
      ),
    );
    expect(s.role, StoreRole.owner);
    final body = a.requests.single.body as Map;
    expect(body['pickup_state'], 'FCT');
    expect(body['district'], 'fashion');
    expect(body['self_delivery_fee_kobo'], 150000);
    expect(body['pickup_area_council'], 'amac');
  });

  test(
    'payout_account_reads_404_as_none_and_sends_the_password_only_when_given',
    () async {
      final a = FakeAdapter()
        ..reply(404, {
          'error': {'code': 'not_found', 'message': 'x'},
        })
        ..reply(200, {
          'bank_code': '058',
          'last4': '6785',
          'status': 'verified',
          'account_name': 'ADA OBI',
          'changed_at': '2026-11-20T00:00:00Z',
        })
        ..reply(200, {
          'bank_code': '058',
          'last4': '6785',
          'status': 'pending',
          'pending_reason': 'name_mismatch',
        });
      final repo = ApiSellRepository(_client(a));
      expect(await repo.payoutAccount('s1'), isNull);
      final first = await repo.setPayoutAccount(
        's1',
        bankCode: '058',
        accountNumber: '0123456785',
      );
      expect(first.verified, isTrue);
      expect((a.requests[1].body as Map).containsKey('password'), isFalse);
      final again = await repo.setPayoutAccount(
        's1',
        bankCode: '058',
        accountNumber: '0123456785',
        password: 'pw',
      );
      expect(again.pendingReason, 'name_mismatch');
      expect((a.requests[2].body as Map)['password'], 'pw');
      expect(a.requests[2].method, 'PUT');
    },
  );

  test('branches_team_and_invites_use_the_store_routes', () async {
    final a = FakeAdapter()
      ..reply(200, [
        {
          'id': 'b1',
          'name': 'Garki',
          'collect_enabled': true,
          'position': 0,
          'pickup_address': '1 Road',
        },
      ])
      ..reply(201, {'id': 'b2', 'name': 'Wuse', 'collect_enabled': false})
      ..reply(200, {'id': 'b2', 'name': 'Wuse 2', 'collect_enabled': true})
      ..reply(204)
      ..reply(200, {'members': [], 'invites': []})
      ..reply(201, {})
      ..reply(204)
      ..reply(204)
      ..reply(200, [
        {
          'id': 'i1',
          'role': 'staff',
          'store': {'name': 'Shop'},
        },
      ])
      ..reply(200, {});
    final repo = ApiSellRepository(_client(a));
    final list = await repo.branches('s1');
    expect(list.single.collectEnabled, isTrue);
    await repo.addBranch(
      's1',
      const BranchInput(
        name: 'Wuse',
        address: 'A',
        areaCouncil: 'amac',
        phone: '0803',
      ),
    );
    await repo.editBranch('s1', 'b2', const BranchInput(collectEnabled: true));
    await repo.closeBranch('s1', 'b2');
    await repo.team('s1');
    await repo.invite(
      's1',
      phone: '08031234567',
      role: StoreRole.staff,
      branchId: 'b1',
    );
    await repo.revokeInvite('s1', 'i1');
    await repo.removeMember('s1', 'u1');
    final invites = await repo.myInvites();
    await repo.answerInvite('i1', accept: true);
    expect(invites.single.storeName, 'Shop');
    expect(
      [for (final r in a.requests) '${r.method} ${r.path}'],
      [
        'GET /stores/s1/branches',
        'POST /stores/s1/branches',
        'PATCH /stores/s1/branches/b2',
        'POST /stores/s1/branches/b2/close',
        'GET /stores/s1/team',
        'POST /stores/s1/invites',
        'DELETE /stores/s1/invites/i1',
        'DELETE /stores/s1/team/u1',
        'GET /me/invites',
        'POST /me/invites/i1/accept',
      ],
    );
    expect(a.requests[5].body, {
      'phone': '08031234567',
      'role': 'staff',
      'branch_id': 'b1',
    });
    // A patch sends only what changed.
    expect(a.requests[2].body, {'collect_enabled': true});
  });

  test('work_calls_use_the_product_order_review_and_dispute_routes', () async {
    final product = {
      'id': 'p1',
      'name': 'Shirt',
      'description': '',
      'category': 'fashion',
      'condition': 'new',
      'base_price_kobo': 100,
      'is_published': true,
      'photo_keys': ['k'],
      'photo_urls': ['https://x/k'],
      'variants': [
        {
          'id': 'v1',
          'attributes': {'option': 'L'},
          'stock': 3,
          'available_stock': 3,
        },
      ],
    };
    final order = {
      'id': 'o1',
      'status': 'paid',
      'created_at': '2026-11-20T10:00:00Z',
      'total_kobo': 100,
      'delivery_method': 'self',
    };
    final a = FakeAdapter()
      ..reply(200, [product])
      ..reply(201, product)
      ..reply(200, product)
      ..reply(204)
      ..reply(204)
      ..reply(204)
      ..reply(201, {
        'upload_id': 'u1',
        'url': 'https://files.test/put',
        'headers': {'Content-Type': 'image/jpeg'},
      })
      ..reply(200, {'upload_id': 'u1', 'key': 'product/u1.jpg'})
      ..reply(200, {
        'items': [order],
        'next_cursor': 'abc',
      })
      ..reply(200, order)
      ..reply(200, order)
      ..reply(200, {
        'reviews': [
          {'id': 'r1', 'stars': 5, 'created_at': '2026-11-20T10:00:00Z'},
        ],
      })
      ..reply(201, {})
      ..reply(200, {
        'id': 'd1',
        'order_id': 'o1',
        'status': 'open',
        'reason': 'x',
        'seller_respond_by': '2026-12-01T00:00:00Z',
        'buyer_statement': {
          'body': 'bad',
          'photo_keys': ['dispute/1.jpg'],
        },
      })
      ..reply(200, {'url': 'https://files.test/signed'});
    final repo = ApiWorkRepository(_client(a));
    final listings = await repo.listings('s1');
    expect(listings.single.variants.single.stock, 3);
    expect(listings.single.photoUrls.single, 'https://x/k');
    const input = ListingInput(
      name: 'Shirt',
      description: '',
      category: 'fashion',
      condition: 'new',
      priceKobo: 100,
      photoKeys: ['k'],
    );
    await repo.createListing('s1', input, singleBranch: true);
    await repo.editListing('p1', input);
    await repo.setPublished('p1', false);
    await repo.deleteListing('p1');
    await repo.setStock('b1', {'v1': 4});
    final ticket = await repo.newUpload(
      purpose: 'product',
      contentType: 'image/jpeg',
      bytes: 300000,
    );
    expect('$ticket', isNot(contains('files.test')));
    expect(await repo.finishUpload(ticket.uploadId), 'product/u1.jpg');
    final page = await repo.orders('s1', before: 'cur');
    expect(page.nextCursor, 'abc');
    await repo.orderStep('o1', OrderStep.delivered, handoverCode: '123456');
    await repo.orderStep('o1', OrderStep.pickedUp);
    expect((await repo.reviews('shop')).single.stars, 5);
    await repo.replyToReview('r1', 'Thanks');
    final d = await repo.dispute('d1');
    expect(d.buyer.photoKeys.single, 'dispute/1.jpg');
    expect(
      await repo.disputePhotoUrl('d1', 'dispute/1.jpg'),
      'https://files.test/signed',
    );
    expect(
      [for (final r in a.requests) '${r.method} ${r.path}'],
      [
        'GET /stores/s1/products',
        'POST /stores/s1/products',
        'PATCH /products/p1',
        'PATCH /products/p1',
        'DELETE /products/p1',
        'PUT /branches/b1/stock',
        'POST /uploads',
        'POST /uploads/u1/finalize',
        'GET /stores/s1/orders',
        'POST /orders/o1/delivered',
        'POST /orders/o1/picked-up',
        'GET /stores/shop/reviews',
        'POST /reviews/r1/reply',
        'GET /disputes/d1',
        'GET /disputes/d1/photo',
      ],
    );
    expect(a.requests[9].body, {'handover_code': '123456'});
    expect(a.requests[10].body, isNull);
    expect(
      a.requests[8].query,
      {'limit': 25, 'before': 'cur'}.map((k, v) => MapEntry(k, '$v')),
    );
    expect(a.requests.last.query, {'key': 'dispute/1.jpg'});
    expect((a.requests[1].body as Map)['photo_keys'], ['k']);
  });
}
