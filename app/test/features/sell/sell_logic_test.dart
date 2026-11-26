import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:market/features/sell/identity/nin_input.dart';
import 'package:market/features/sell/listing_models.dart';
import 'package:market/features/sell/order_models.dart';
import 'package:market/features/sell/photos/photo_prep.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_strings.dart';
import 'package:market/features/sell/sell_widgets.dart';

import '../../support/fake_sell.dart';

TextEditingValue _type(String text) => ninFormatter.formatEditUpdate(
  TextEditingValue.empty,
  TextEditingValue(
    text: text,
    selection: TextSelection.collapsed(offset: text.length),
  ),
);

SellOrder _order({
  String status = 'paid',
  DeliveryMethod? method = DeliveryMethod.courier,
  bool held = false,
  Map<String, String> done = const {},
}) => SellOrder(
  id: 'o1',
  status: status,
  method: method,
  lines: const [],
  totalKobo: 1000,
  createdAt: DateTime(2026),
  held: held,
  doneBy: done,
);

void main() {
  group('NIN input', () {
    test('keeps_digits_only_and_stops_at_eleven', () {
      expect(_type('1234 5678-901x').text, '12345678901');
      expect(_type('123456789012345').text, '12345678901');
      expect(_type('abc').text, '');
      expect(_type('123').text, '123');
      expect(isCompleteNin('12345678901'), isTrue);
      expect(isCompleteNin('1234567890'), isFalse);
    });

    test('models_that_hold_secrets_never_print_them', () {
      const start = IdentityStart(attemptId: 'a', sessionToken: 'secret-token');
      expect('$start', isNot(contains('secret')));
      const store = NewStore(
        name: 'Shop',
        description: '',
        pickupAddress: '12 Quiet Close',
        district: 'd',
        areaCouncil: 'amac',
        businessPhone: '08031234567',
        courier: true,
        selfDelivery: false,
      );
      expect('$store', isNot(contains('Quiet')));
      expect('$store', isNot(contains('0803')));
      const payout = PayoutAccount(
        bankCode: '058',
        last4: '6785',
        status: 'verified',
        accountName: 'ADA OBI',
      );
      expect('$payout', isNot(contains('6785')));
      expect('$payout', isNot(contains('ADA')));
      const input = BranchInput(address: '9 Hidden Road', phone: '08030000000');
      expect('$input', isNot(contains('Hidden')));
    });
  });

  group('models', () {
    test('a_store_with_a_role_parses_and_one_without_is_skipped', () {
      final s = SellStore.fromJson({
        'id': 's1',
        'name': 'Shop',
        'slug': 'shop',
        'status': 'active',
        'my_role': 'staff',
        'my_branch_id': 'b1',
      })!;
      expect(s.role, StoreRole.staff);
      expect(s.branchId, 'b1');
      expect(s.isActive, isTrue);
      expect(s.role.isManagerUp, isFalse);
      expect(SellStore.fromJson({'id': 's1'}), isNull);
      expect(SellStore.fromJson({'id': 's1', 'my_role': 'king'}), isNull);
    });

    test('identity_status_maps_the_wire_words', () {
      expect(
        IdentityStatus.fromJson({'status': 'in_progress'}).state,
        IdentityState.inProgress,
      );
      expect(
        IdentityStatus.fromJson({'status': 'passed'}).state,
        IdentityState.passed,
      );
      expect(
        IdentityStatus.fromJson({'status': 'nonsense'}).state,
        IdentityState.unverified,
      );
      final due = IdentityStatus.fromJson({
        'status': 'unverified',
        'required_by': '2026-12-01T00:00:00Z',
      });
      expect(due.requiredBy, isNotNull);
    });

    test('team_and_invites_parse', () {
      final t = Team.fromJson({
        'members': [
          {
            'user_id': 'u1',
            'display_name': 'Amina',
            'role': 'staff',
            'branch': {'id': 'b1', 'name': 'Garki'},
          },
          {'user_id': 'u2', 'role': 'wizard'},
        ],
        'invites': [
          {'id': 'i1', 'phone_last4': '4567', 'role': 'manager'},
        ],
      });
      expect(t.members.single.branch!.name, 'Garki');
      expect(t.invites.single.last4, '4567');
      final mine = MyInvite.fromJson({
        'id': 'i',
        'role': 'staff',
        'store': {'name': 'Ada Shop'},
        'branch': {'id': 'b', 'name': 'Wuse'},
      })!;
      expect(mine.storeName, 'Ada Shop');
      expect(mine.branchName, 'Wuse');
    });

    test('an_order_parses_with_its_method_branch_and_who_did_what', () {
      final o = SellOrder.fromJson({
        'id': 'o1',
        'status': 'paid',
        'delivery_method': 'collect',
        'branch_name': 'Garki',
        'buyer_display_name': 'Chidi',
        'total_kobo': 2500000,
        'expected_payout_kobo': 2300000,
        'created_at': '2026-11-20T10:00:00Z',
        'done_by': {
          'ready': {'user_id': 'u', 'display_name': 'Amina'},
        },
        'items': [
          {
            'product_name': 'Shirt',
            'quantity': 2,
            'unit_price_kobo': 1250000,
            'attributes': {'size': 'L'},
          },
        ],
        'dispute': {'id': 'd1', 'status': 'open'},
      })!;
      expect(o.method, DeliveryMethod.collect);
      expect(o.branchName, 'Garki');
      expect(o.doneBy['ready'], 'Amina');
      expect(o.lines.single.label, 'L');
      expect(o.disputeId, 'd1');
      expect(SellOrder.fromJson({'id': 'x'}), isNull);
    });

    test('the_next_step_follows_the_delivery_method', () {
      // Courier: make it ready once.
      expect(_order().nextStep(), OrderStep.ready);
      expect(_order(done: {'ready': 'Ada'}).nextStep(), isNull);
      // Self-delivery: picked up, out for delivery, delivered with the code.
      expect(
        _order(method: DeliveryMethod.self).nextStep(),
        OrderStep.pickedUp,
      );
      expect(
        _order(method: DeliveryMethod.self, status: 'picked_up').nextStep(),
        OrderStep.outForDelivery,
      );
      expect(
        _order(
          method: DeliveryMethod.self,
          status: 'out_for_delivery',
        ).nextStep(),
        OrderStep.delivered,
      );
      // Collect: ready, then handed over with the code.
      expect(
        _order(method: DeliveryMethod.collect).nextStep(),
        OrderStep.ready,
      );
      expect(
        _order(
          method: DeliveryMethod.collect,
          done: {'ready': 'Ada'},
        ).nextStep(),
        OrderStep.delivered,
      );
      // Nothing to do when held, unknown, finished.
      expect(_order(held: true).nextStep(), isNull);
      expect(_order(method: null).nextStep(), isNull);
      expect(_order(status: 'completed').nextStep(), isNull);
      expect(SellOrder.needsCode(OrderStep.delivered), isTrue);
      expect(SellOrder.needsCode(OrderStep.ready), isFalse);
    });

    test('a_product_builds_its_requests', () {
      const input = ListingInput(
        name: 'Shirt',
        description: 'Cotton',
        category: 'fashion',
        condition: 'new',
        priceKobo: 500000,
        photoKeys: ['k1'],
        variants: [
          VariantInput(attributes: {'option': 'L'}, stock: {'b1': 2, 'b2': 3}),
        ],
      );
      final one = input.toCreateJson(true);
      expect(one['base_price_kobo'], 500000);
      expect(((one['variants']! as List).single as Map)['stock'], 5);
      final many = input.toCreateJson(false);
      expect(((many['variants']! as List).single as Map)['stock'], {
        'b1': 2,
        'b2': 3,
      });
      expect(input.toPatchJson().containsKey('variants'), isFalse);
    });

    test('naira_amounts_become_kobo_exactly', () {
      expect(nairaToKobo('1500'), 150000);
      expect(nairaToKobo('1,500.5'), 150050);
      expect(nairaToKobo('0.07'), 7);
      expect(nairaToKobo(''), isNull);
      expect(nairaToKobo('12.345'), isNull);
      expect(nairaToKobo('abc'), isNull);
      expect(nairaToKobo('-5'), isNull);
    });
  });

  group('errors', () {
    test('every_message_is_plain_words', () {
      expect(
        sellError(apiError('identity_rate_limited', status: 429)),
        SellStrings.rateLimited,
      );
      expect(
        sellError(apiError('identity_required', status: 403)),
        SellStrings.identityRequired,
      );
      expect(
        sellError(apiError('not_allowed', status: 403)),
        SellStrings.notAllowed,
      );
      expect(sellError(apiError('handover_code_wrong')), SellStrings.codeWrong);
      expect(
        sellError(apiError('code_revoked', status: 422)),
        SellStrings.codeRefused,
      );
      expect(
        sellError(apiError('something_new')),
        isNot(contains('something_new')),
      );
      expect(sellError(StateError('boom')), isNot(contains('boom')));
    });
  });

  group('photo prep', () {
    test('the_square_and_edge_maths', () {
      expect(squareSide(3000, 2000), 2000);
      expect(squareSide(800, 1200), 800);
      expect(targetEdge(2000), photoMaxEdge);
      expect(targetEdge(900), 900);
    });

    test('a_wide_photo_becomes_a_small_square_jpeg_with_no_location', () {
      final picture = img.Image(width: 3000, height: 2000);
      picture.exif.gpsIfd.data[0x0001] = img.IfdValueAscii('N');
      final original = Uint8List.fromList(img.encodeJpg(picture));
      expect(
        img.decodeJpg(original)!.exif.gpsIfd.isEmpty,
        isFalse,
        reason: 'the test picture carries a location',
      );

      final out = preparePhotoSync(original)!;
      final decoded = img.decodeJpg(out)!;
      expect(decoded.width, photoMaxEdge);
      expect(decoded.height, photoMaxEdge);
      expect(decoded.exif.isEmpty, isTrue);
      expect(out.length, lessThan(original.length));
    });

    test('a_small_photo_is_cut_square_but_never_enlarged', () {
      final out = preparePhotoSync(
        Uint8List.fromList(img.encodeJpg(img.Image(width: 600, height: 400))),
      )!;
      final decoded = img.decodeJpg(out)!;
      expect((decoded.width, decoded.height), (400, 400));
    });

    test('something_that_is_not_a_picture_gives_null', () {
      expect(preparePhotoSync(Uint8List.fromList([1, 2, 3, 4])), isNull);
    });
  });
}
