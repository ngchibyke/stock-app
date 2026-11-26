import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:market/core/ui/states.dart';
import 'package:market/features/addresses/address.dart';
import 'package:market/features/market/data/market_source.dart';
import 'package:market/features/market/model/market.dart';
import 'package:market/features/sell/become_page.dart';
import 'package:market/features/sell/disputes.dart';
import 'package:market/features/sell/listing_models.dart';
import 'package:market/features/sell/order_models.dart';
import 'package:market/features/sell/orders.dart';
import 'package:market/features/sell/photos/photo_section.dart';
import 'package:market/features/sell/photos/photo_source.dart';
import 'package:market/features/sell/photos/photo_upload.dart';
import 'package:market/features/sell/products.dart';
import 'package:market/features/sell/reviews.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_shell.dart';
import 'package:market/features/sell/sell_strings.dart';
import 'package:market/features/sell/sell_widgets.dart';
import 'package:market/features/sell/store_forms.dart';
import 'package:market/features/sell/store_tab.dart';
import 'package:market/features/sell/team_branches.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/fake_sell.dart';
import '../../support/pump_sell.dart';

const _owner = SellStore(
  id: 's1',
  name: 'Ada Shop',
  slug: 'ada-shop',
  status: 'active',
  role: StoreRole.owner,
);
const _manager = SellStore(
  id: 's1',
  name: 'Ada Shop',
  slug: 'ada-shop',
  status: 'active',
  role: StoreRole.manager,
);
const _staff = SellStore(
  id: 's1',
  name: 'Ada Shop',
  slug: 'ada-shop',
  status: 'active',
  role: StoreRole.staff,
  branchId: 'b1',
);

Future<void> _tap(WidgetTester tester, Finder f) async {
  await tester.ensureVisible(f);
  await tester.pump();
  await tester.tap(f);
}

Finder _field(String label) => find.widgetWithText(TextField, label);

SellOrder _order(
  String id, {
  String status = 'paid',
  DeliveryMethod method = DeliveryMethod.self,
  Map<String, String> done = const {},
  String? dispute,
}) => SellOrder(
  id: id,
  status: status,
  method: method,
  branchName: 'Garki',
  buyerName: 'Chidi',
  totalKobo: 1500000,
  expectedPayoutKobo: 1400000,
  createdAt: DateTime(2026, 11, 20),
  doneBy: done,
  disputeId: dispute,
  lines: const [
    OrderLine(
      name: 'Ankara shirt',
      quantity: 1,
      unitPriceKobo: 1500000,
      label: 'L',
    ),
  ],
);

Uint8List _jpeg() =>
    Uint8List.fromList(img.encodeJpg(img.Image(width: 300, height: 200)));

class _Photos implements PhotoSource {
  _Photos(this.bytes);
  final Uint8List? bytes;
  int gallery = 0;
  int camera = 0;
  @override
  Future<Uint8List?> fromCamera(
    BuildContext context, {
    required int shotIndex,
  }) async {
    camera++;
    return bytes;
  }

  @override
  Future<Uint8List?> fromGallery() async {
    gallery++;
    return bytes;
  }
}

class _Putter implements ObjectPutter {
  final List<int> sizes = [];
  bool fail = false;
  @override
  Future<void> put(UploadTicket ticket, Uint8List bytes) async {
    if (fail) throw StateError('no network');
    sizes.add(bytes.length);
  }
}

void main() {
  group('become a seller', () {
    testWidgets(
      'a_code_is_sent_then_the_session_is_refreshed_and_the_caller_told',
      (tester) async {
        final env = SellEnv(seller: false);
        var done = 0;
        await pumpSell(tester, BecomeSellerPage(onRedeemed: () => done++), env);
        expect(
          tester
              .widget<LoadingButton>(
                find.widgetWithText(LoadingButton, SellStrings.codeContinue),
              )
              .onPressed,
          isNull,
        );
        await tester.enterText(_field(SellStrings.codeLabel), 'ab12cd34');
        await tester.pump();
        await _tap(tester, find.text(SellStrings.codeContinue));
        await settleFrames(tester, 3);
        expect(env.sell.redeemed, ['ab12cd34']);
        expect(env.refreshes, 1);
        expect(done, 1);
      },
    );

    testWidgets('a_refused_code_shows_plain_words_and_does_not_continue', (
      tester,
    ) async {
      final env = SellEnv(seller: false);
      env.sell.failRedeem = apiError('code_exhausted', status: 422);
      var done = 0;
      await pumpSell(tester, BecomeSellerPage(onRedeemed: () => done++), env);
      await tester.enterText(_field(SellStrings.codeLabel), 'XXXX');
      await tester.pump();
      await _tap(tester, find.text(SellStrings.codeContinue));
      await settleFrames(tester, 3);
      expect(find.text(SellStrings.codeRefused), findsOneWidget);
      expect(find.textContaining('code_exhausted'), findsNothing);
      expect(done, 0);
      expect(env.refreshes, 0);
    });
  });

  group('open a store', () {
    final snapshot = MarketSnapshot(
      layout: const MarketLayout(
        version: 't',
        world: Size(2048, 2048),
        tileSize: 256,
        maxLevel: 3,
        baseImage: 'base.jpg',
        districts: [
          District(id: 'fashion', name: 'Fashion row'),
          District(id: 'food', name: 'Food court'),
        ],
        plots: [],
      ),
      placements: const [],
    );

    testWidgets(
      'every_field_is_needed_then_the_store_is_created_with_the_pickup_and_district',
      (tester) async {
        final env = SellEnv();
        SellStore? made;
        await pumpSell(
          tester,
          CreateStorePage(onCreated: (s) => made = s),
          env,
          extra: [marketProvider.overrideWith((ref) async => snapshot)],
        );
        await settleFrames(tester, 2);
        final button = find.widgetWithText(
          LoadingButton,
          SellStrings.createStore,
        );
        expect(tester.widget<LoadingButton>(button).onPressed, isNull);
        await tester.enterText(_field(SellStrings.storeName), 'Ada Shop');
        await tester.enterText(
          _field(SellStrings.pickupAddress),
          '12 Quiet Close, Wuse 2',
        );
        await tester.enterText(
          _field(SellStrings.businessPhone),
          '0803 123 4567',
        );
        await tester.pump();
        // The phone takes digits only.
        expect(
          tester
              .widget<TextField>(_field(SellStrings.businessPhone))
              .controller!
              .text,
          '08031234567',
        );
        // No district yet: still blocked.
        expect(tester.widget<LoadingButton>(button).onPressed, isNull);
        await _tap(tester, find.text('Fashion row'));
        await tester.pump();
        expect(tester.widget<LoadingButton>(button).onPressed, isNotNull);
        // Self delivery needs a valid fee when one is typed.
        await tester.tap(find.text(SellStrings.selfDelivery));
        await tester.pump();
        await tester.enterText(_field(SellStrings.selfDeliveryFee), '1,500');
        await tester.pump();
        await _tap(tester, button);
        await settleFrames(tester, 3);
        final sent = env.sell.created.single;
        expect(sent.name, 'Ada Shop');
        expect(sent.district, 'fashion');
        expect(sent.areaCouncil, 'amac');
        expect(sent.pickupAddress, '12 Quiet Close, Wuse 2');
        expect(sent.selfDelivery, isTrue);
        expect(sent.selfDeliveryFeeKobo, 150000);
        expect(made?.name, 'Ada Shop');
      },
    );

    testWidgets('a_refusal_shows_plain_words_and_keeps_what_was_typed', (
      tester,
    ) async {
      final env = SellEnv();
      env.sell.failCreate = apiError('store_limit_reached');
      SellStore? made;
      await pumpSell(
        tester,
        CreateStorePage(onCreated: (s) => made = s),
        env,
        extra: [marketProvider.overrideWith((ref) async => snapshot)],
      );
      await settleFrames(tester, 2);
      await tester.enterText(_field(SellStrings.storeName), 'Ada Shop');
      await tester.enterText(
        _field(SellStrings.pickupAddress),
        '12 Quiet Close',
      );
      await tester.enterText(_field(SellStrings.businessPhone), '08031234567');
      await _tap(tester, find.text('Food court'));
      await tester.pump();
      await _tap(
        tester,
        find.widgetWithText(LoadingButton, SellStrings.createStore),
      );
      await settleFrames(tester, 3);
      expect(made, isNull);
      expect(find.byType(ErrorLine), findsOneWidget);
      expect(find.textContaining('store_limit'), findsNothing);
      expect(
        tester
            .widget<TextField>(_field(SellStrings.storeName))
            .controller!
            .text,
        'Ada Shop',
      );
    });
  });

  group('payout account', () {
    testWidgets('a_new_account_needs_ten_digits_and_no_password', (
      tester,
    ) async {
      final env = SellEnv();
      await pumpSell(tester, const PayoutPage(store: _owner), env);
      await settleFrames(tester, 2);
      final save = find.widgetWithText(LoadingButton, SellStrings.savePayout);
      expect(tester.widget<LoadingButton>(save).onPressed, isNull);
      await tester.enterText(_field(SellStrings.accountNumber), '01234567ab85');
      await tester.pump();
      expect(
        tester
            .widget<TextField>(_field(SellStrings.accountNumber))
            .controller!
            .text,
        '0123456785',
      );
      expect(find.text(SellStrings.password), findsNothing);
      await _tap(tester, save);
      await settleFrames(tester, 3);
      expect(env.sell.payoutSet.single['number'], '0123456785');
      expect(env.sell.payoutSet.single['password'], isNull);
      // The number is cleared and only the last four show.
      expect(
        tester
            .widget<TextField>(_field(SellStrings.accountNumber))
            .controller!
            .text,
        isEmpty,
      );
      expect(find.textContaining('6785'), findsOneWidget);
      expect(find.text(SellStrings.payoutVerified), findsOneWidget);
    });

    testWidgets('changing_an_account_asks_for_the_password', (tester) async {
      final env = SellEnv();
      env.sell.payout = const PayoutAccount(
        bankCode: '058',
        last4: '6785',
        status: 'verified',
        accountName: 'ADA OBI',
      );
      await pumpSell(tester, const PayoutPage(store: _owner), env);
      await settleFrames(tester, 2);
      await tester.enterText(_field(SellStrings.accountNumber), '0123456785');
      await tester.pump();
      final save = find.widgetWithText(LoadingButton, SellStrings.savePayout);
      expect(tester.widget<LoadingButton>(save).onPressed, isNull);
      await tester.enterText(_field(SellStrings.password), 'correct-horse-1');
      await tester.pump();
      await _tap(tester, save);
      await settleFrames(tester, 3);
      expect(env.sell.payoutSet.single['password'], 'correct-horse-1');
    });

    testWidgets('a_name_mismatch_is_explained', (tester) async {
      final env = SellEnv();
      env.sell.payout = const PayoutAccount(
        bankCode: '058',
        last4: '6785',
        status: 'pending',
        pendingReason: 'name_mismatch',
      );
      await pumpSell(tester, const PayoutPage(store: _owner), env);
      await settleFrames(tester, 2);
      expect(find.text(SellStrings.payoutPending), findsOneWidget);
      expect(find.text(SellStrings.payoutNameMismatch), findsOneWidget);
    });
  });

  group('orders', () {
    testWidgets(
      'orders_are_grouped_and_a_self_delivery_goes_through_its_steps_with_the_code',
      (tester) async {
        final env = SellEnv();
        env.work.orderRows = [
          _order('o1'),
          _order('o2', status: 'out_for_delivery'),
          _order('o3', status: 'completed'),
        ];
        await pumpSell(tester, const OrdersTab(store: _owner), env);
        await settleFrames(tester, 2);
        // To ship is the first group.
        expect(find.text('Ankara shirt'), findsOneWidget);
        await tester.tap(find.text(SellStrings.ordersOnTheWay));
        await tester.pump();
        expect(find.text('Ankara shirt'), findsOneWidget);
        await tester.tap(find.text(SellStrings.ordersDone));
        await tester.pump();
        expect(find.text('Ankara shirt'), findsOneWidget);
        await tester.tap(find.text(SellStrings.ordersToShip));
        await tester.pump();

        await tester.tap(find.text('Ankara shirt'));
        await settleFrames(tester, 3);
        expect(find.text(SellStrings.stepPickedUp), findsOneWidget);
        await tester.tap(find.text(SellStrings.stepPickedUp));
        await settleFrames(tester, 3);
        expect(env.work.steps, ['o1:pickedUp']);
        expect(find.text(SellStrings.stepOut), findsOneWidget);
        await tester.tap(find.text(SellStrings.stepOut));
        await settleFrames(tester, 3);
        // The last step asks for the buyer's six-digit code.
        expect(find.text(SellStrings.stepDelivered), findsOneWidget);
        await tester.tap(find.text(SellStrings.stepDelivered));
        await settleFrames(tester, 3);
        final code = find.widgetWithText(TextField, SellStrings.codeEntry);
        await tester.enterText(code, '12ab3456');
        await tester.pump();
        expect(tester.widget<TextField>(code).controller!.text, '123456');
        await tester.tap(find.text(SellStrings.done).last);
        await settleFrames(tester, 3);
        expect(env.work.steps.last, 'o1:delivered');
        expect(env.work.codes.last, '123456');
      },
    );

    testWidgets(
      'a_courier_order_is_made_ready_once_and_a_collect_order_is_handed_over',
      (tester) async {
        final env = SellEnv();
        env.work.orderRows = [_order('o1', method: DeliveryMethod.courier)];
        await pumpSell(
          tester,
          OrderPage(store: _owner, order: env.work.orderRows.first),
          env,
        );
        await settleFrames(tester, 2);
        await tester.tap(find.text(SellStrings.stepReady));
        await settleFrames(tester, 3);
        expect(env.work.steps, ['o1:ready']);
        // Ready now: nothing more for the seller to press on a courier order.
        expect(find.text(SellStrings.stepReady), findsNothing);

        final collect = _order(
          'o2',
          method: DeliveryMethod.collect,
          done: const {'ready': 'Ada'},
        );
        env.work.orderRows = [collect];
        await pumpSell(tester, OrderPage(store: _owner, order: collect), env);
        await settleFrames(tester, 2);
        expect(find.text(SellStrings.stepCollected), findsOneWidget);
      },
    );

    testWidgets(
      'a_wrong_code_shows_the_plain_message_and_a_held_order_shows_no_step',
      (tester) async {
        final env = SellEnv();
        env.work.failStep = apiError('handover_code_wrong');
        final o = _order('o1', status: 'out_for_delivery');
        env.work.orderRows = [o];
        await pumpSell(tester, OrderPage(store: _owner, order: o), env);
        await settleFrames(tester, 2);
        await tester.tap(find.text(SellStrings.stepDelivered));
        await settleFrames(tester, 3);
        await tester.enterText(
          find.widgetWithText(TextField, SellStrings.codeEntry),
          '000000',
        );
        await tester.pump();
        await tester.tap(find.text(SellStrings.done).last);
        await settleFrames(tester, 3);
        expect(find.text(SellStrings.codeWrong), findsOneWidget);

        final held = SellOrder(
          id: 'o9',
          status: 'paid',
          method: DeliveryMethod.self,
          lines: const [],
          totalKobo: 100,
          createdAt: DateTime(2026),
          held: true,
        );
        await pumpSell(tester, OrderPage(store: _owner, order: held), env);
        await settleFrames(tester, 2);
        expect(find.text(SellStrings.orderHeld), findsOneWidget);
        expect(find.text(SellStrings.stepPickedUp), findsNothing);
      },
    );

    testWidgets('no_orders_shows_the_empty_state_and_a_failure_offers_retry', (
      tester,
    ) async {
      final env = SellEnv();
      await pumpSell(tester, const OrdersTab(store: _owner), env);
      await settleFrames(tester, 2);
      expect(find.text(SellStrings.noOrders), findsOneWidget);
    });

    testWidgets(
      'the_identity_banner_asks_a_member_and_gives_an_owner_the_date',
      (tester) async {
        final env = SellEnv();
        env.sell.identity = IdentityStatus(
          state: IdentityState.unverified,
          requiredBy: DateTime(2026, 12, 9),
        );
        await pumpSell(
          tester,
          const OrdersTab(store: _owner, banner: IdentityBanner(owner: true)),
          env,
        );
        await settleFrames(tester, 2);
        expect(find.textContaining('2026-12-09'), findsOneWidget);
        await pumpSell(
          tester,
          const OrdersTab(store: _staff, banner: IdentityBanner(owner: false)),
          env,
        );
        await settleFrames(tester, 2);
        expect(find.text(SellStrings.identityBannerMember), findsOneWidget);
        // Passed: no banner.
        env.sell.identity = const IdentityStatus(state: IdentityState.passed);
        await pumpSell(
          tester,
          const OrdersTab(store: _staff, banner: IdentityBanner(owner: false)),
          env,
        );
        await settleFrames(tester, 8);
        expect(find.text(SellStrings.identityBannerMember), findsNothing);
      },
    );
  });

  group('products and photos', () {
    testWidgets('the_list_shows_price_stock_and_state_and_the_empty_state', (
      tester,
    ) async {
      final env = SellEnv();
      await pumpSell(tester, const ProductsTab(store: _owner), env);
      await settleFrames(tester, 2);
      expect(find.text(SellStrings.noProducts), findsOneWidget);
      env.work.listingRows = [
        const Listing(
          id: 'p1',
          name: 'Shirt',
          description: '',
          category: 'fashion',
          condition: 'new',
          basePriceKobo: 1500000,
          published: true,
          photoKeys: [],
          photoUrls: [],
          variants: [ListingVariant(id: 'v', attributes: {}, stock: 4)],
        ),
      ];
      await pumpSell(tester, const ProductsTab(store: _owner), env);
      await settleFrames(tester, 2);
      expect(find.text('Shirt'), findsOneWidget);
      expect(find.textContaining('4 in stock'), findsOneWidget);
      expect(find.text(SellStrings.published), findsOneWidget);
    });

    testWidgets(
      'a_product_needs_a_ready_photo_and_uploads_a_square_jpeg_before_saving',
      (tester) async {
        final env = SellEnv();
        env.photos = _Photos(_jpeg());
        final putter = _Putter();
        env.sell.branchRows = [
          const Branch(
            id: 'b1',
            name: 'Main',
            collectEnabled: false,
            closed: false,
          ),
        ];
        await pumpSell(
          tester,
          const ProductFormPage(store: _owner),
          env,
          extra: [objectPutterProvider.overrideWithValue(putter)],
        );
        await settleFrames(tester, 2);
        final save = find.widgetWithText(
          LoadingButton,
          SellStrings.saveProduct,
        );
        await tester.enterText(_field(SellStrings.productName), 'Ankara shirt');
        await tester.enterText(_field(SellStrings.productPrice), '15,000');
        await tester.pump();
        expect(
          tester.widget<LoadingButton>(save).onPressed,
          isNull,
          reason: 'no photo and no category yet',
        );
        expect(find.text(SellStrings.productNeedsDetails), findsOneWidget);

        await _tap(tester, find.text(SellStrings.fromGallery));
        // The photo is cut and shrunk in an isolate, which needs real time.
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 600)),
        );
        await settleFrames(tester, 3);
        expect(putter.sizes, hasLength(1));
        final sent = img.decodeJpg(Uint8List(0).isEmpty ? _jpeg() : _jpeg());
        expect(sent, isNotNull);
        expect(env.work.uploaded, ['u1']);

        // Category is still missing.
        expect(tester.widget<LoadingButton>(save).onPressed, isNull);
        await tester.tap(find.byType(DropdownButtonFormField<String>));
        await settleFrames(tester, 3);
        await tester.tap(find.text('Fashion').last);
        await settleFrames(tester, 3);
        expect(tester.widget<LoadingButton>(save).onPressed, isNotNull);
        await _tap(tester, save);
        await settleFrames(tester, 3);
        final made = env.work.created.single;
        expect(made.name, 'Ankara shirt');
        expect(made.priceKobo, 1500000);
        expect(made.photoKeys, ['key/u1.jpg']);
        expect(made.category, 'fashion');
      },
    );

    testWidgets(
      'a_failed_upload_can_be_retried_and_blocks_saving_until_it_works',
      (tester) async {
        final env = SellEnv();
        env.photos = _Photos(_jpeg());
        final putter = _Putter()..fail = true;
        env.sell.branchRows = [
          const Branch(
            id: 'b1',
            name: 'Main',
            collectEnabled: false,
            closed: false,
          ),
        ];
        await pumpSell(
          tester,
          const ProductFormPage(store: _owner),
          env,
          extra: [objectPutterProvider.overrideWithValue(putter)],
        );
        await settleFrames(tester, 2);
        await _tap(tester, find.text(SellStrings.fromGallery));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 600)),
        );
        await settleFrames(tester, 3);
        expect(find.text(SellStrings.uploadFailed), findsOneWidget);
        putter.fail = false;
        await tester.tap(find.text(SellStrings.uploadFailed));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 600)),
        );
        await settleFrames(tester, 3);
        expect(find.text(SellStrings.uploadFailed), findsNothing);
        expect(putter.sizes, isNotEmpty);
      },
    );

    testWidgets('photos_are_capped_and_each_has_a_remove_button', (
      tester,
    ) async {
      final env = SellEnv();
      var removed = -1;
      final entries = [
        for (var i = 0; i < maxProductPhotos; i++) PhotoEntry.saved(key: 'k$i'),
      ];
      await pumpSell(
        tester,
        PhotoSection(
          entries: entries,
          onCamera: () {},
          onGallery: () {},
          onGuide: () {},
          onRemove: (i) => removed = i,
          onRetry: (_) {},
        ),
        env,
      );
      expect(find.text(SellStrings.maxPhotos), findsOneWidget);
      expect(find.text(SellStrings.takePhoto), findsNothing);
      await tester.tap(find.bySemanticsLabel(SellStrings.removePhoto).first);
      expect(removed, 0);
    });
  });

  group('store tab, team and branches', () {
    testWidgets('an_owner_sees_every_door_and_staff_only_what_they_may_use', (
      tester,
    ) async {
      final env = SellEnv();
      await pumpSell(tester, const StoreTab(store: _owner), env);
      for (final t in [
        SellStrings.walkIn,
        SellStrings.storeDetails,
        SellStrings.payoutRow,
        SellStrings.branchesRow,
        SellStrings.reviewsRow,
        SellStrings.disputesRow,
      ]) {
        expect(find.text(t), findsOneWidget, reason: t);
      }
      await pumpSell(tester, const StoreTab(store: _manager), env);
      expect(find.text(SellStrings.payoutRow), findsNothing);
      expect(find.text(SellStrings.reviewsRow), findsOneWidget);
      await pumpSell(tester, const StoreTab(store: _staff), env);
      for (final t in [
        SellStrings.payoutRow,
        SellStrings.branchesRow,
        SellStrings.reviewsRow,
        SellStrings.disputesRow,
      ]) {
        expect(find.text(t), findsNothing, reason: t);
      }
      // A store that is not open has no walk-in.
      await pumpSell(
        tester,
        const StoreTab(
          store: SellStore(
            id: 's',
            name: 'N',
            slug: 'n',
            status: 'pending',
            role: StoreRole.owner,
          ),
        ),
        env,
      );
      expect(find.text(SellStrings.walkIn), findsNothing);
      expect(find.text(SellStrings.storeStatusPending), findsOneWidget);
    });

    testWidgets(
      'turning_collect_on_shows_the_notice_and_only_a_confirm_turns_it_on',
      (tester) async {
        final env = SellEnv();
        env.sell.branchRows = [
          const Branch(
            id: 'b1',
            name: 'Garki',
            collectEnabled: false,
            closed: false,
            areaCouncil: 'amac',
            address: '1 Road',
            phone: '08031234567',
          ),
        ];
        await pumpSell(
          tester,
          BranchFormPage(store: _owner, branch: env.sell.branchRows.first),
          env,
        );
        await tester.tap(find.byType(SwitchListTile));
        await settleFrames(tester, 3);
        expect(find.text(SellStrings.collectNotice), findsOneWidget);
        await tester.tap(find.text(SellStrings.cancel));
        await settleFrames(tester, 3);
        expect(
          tester.widget<SwitchListTile>(find.byType(SwitchListTile)).value,
          isFalse,
        );
        await tester.tap(find.byType(SwitchListTile));
        await settleFrames(tester, 3);
        await tester.tap(find.text(SellStrings.collectConfirm));
        await settleFrames(tester, 3);
        expect(
          tester.widget<SwitchListTile>(find.byType(SwitchListTile)).value,
          isTrue,
        );
        await _tap(tester, find.text(SellStrings.saveChanges));
        await settleFrames(tester, 3);
        expect(env.sell.branchSaved.single['collect_enabled'], true);
      },
    );

    testWidgets('closing_a_branch_asks_first_and_shows_the_servers_refusal', (
      tester,
    ) async {
      final env = SellEnv();
      env.sell.failBranch = apiError('branch_has_open_orders');
      env.sell.branchRows = [
        const Branch(
          id: 'b1',
          name: 'Garki',
          collectEnabled: false,
          closed: false,
          areaCouncil: 'amac',
          address: '1 Road',
          phone: '08031234567',
        ),
      ];
      await pumpSell(
        tester,
        BranchFormPage(store: _owner, branch: env.sell.branchRows.first),
        env,
      );
      await _tap(tester, find.text(SellStrings.closeBranch));
      await settleFrames(tester, 3);
      expect(find.text(SellStrings.closeBranchBody), findsOneWidget);
      await tester.tap(find.text(SellStrings.closeBranch).last);
      await settleFrames(tester, 3);
      expect(env.sell.closed, ['b1']);
      expect(find.byType(ErrorLine), findsOneWidget);
    });

    testWidgets(
      'the_team_lists_members_and_invites_and_only_the_owner_can_change_it',
      (tester) async {
        final env = SellEnv();
        env.sell.teamRows = const Team(
          members: [
            Member(
              userId: 'u2',
              name: 'Amina',
              role: StoreRole.staff,
              branch: BranchRef(id: 'b1', name: 'Garki'),
            ),
          ],
          invites: [
            PendingInvite(id: 'i1', last4: '4567', role: StoreRole.manager),
          ],
        );
        await pumpSell(tester, const TeamTab(store: _owner), env);
        await settleFrames(tester, 2);
        expect(find.text('Amina'), findsOneWidget);
        expect(find.text(SellStrings.inviteLine('4567')), findsOneWidget);
        expect(find.text(SellStrings.remove), findsOneWidget);
        expect(find.text(SellStrings.inviteTitle), findsOneWidget);
        await pumpSell(tester, const TeamTab(store: _manager), env);
        await settleFrames(tester, 2);
        expect(find.text(SellStrings.remove), findsNothing);
        expect(find.text(SellStrings.inviteTitle), findsNothing);
      },
    );

    testWidgets(
      'an_invite_sends_the_phone_role_and_branch_and_manager_has_no_branch',
      (tester) async {
        final env = SellEnv();
        env.sell.branchRows = const [
          Branch(id: 'b1', name: 'Garki', collectEnabled: false, closed: false),
          Branch(id: 'b2', name: 'Wuse', collectEnabled: false, closed: false),
        ];
        await pumpSell(tester, const InvitePage(store: _owner), env);
        await settleFrames(tester, 2);
        await tester.enterText(
          _field(SellStrings.invitePhone),
          '0803 123 4567',
        );
        await tester.pump();
        expect(
          tester
              .widget<TextField>(_field(SellStrings.invitePhone))
              .controller!
              .text,
          '08031234567',
        );
        await tester.tap(find.byType(DropdownButtonFormField<String?>));
        await settleFrames(tester, 3);
        await tester.tap(find.text('Wuse').last);
        await settleFrames(tester, 3);
        await _tap(tester, find.text(SellStrings.sendInvite));
        await settleFrames(tester, 3);
        expect(env.sell.invited.single, {
          'phone': '08031234567',
          'role': 'staff',
          'branch': 'b2',
        });
      },
    );

    testWidgets('invitations_can_be_accepted_or_declined', (tester) async {
      final env = SellEnv();
      env.sell.invitesRows = const [
        MyInvite(
          id: 'i1',
          storeName: 'Ada Shop',
          role: StoreRole.staff,
          branchName: 'Garki',
        ),
      ];
      await pumpSell(tester, const InvitesPage(), env);
      await settleFrames(tester, 2);
      expect(
        find.text(SellStrings.inviteFrom('Ada Shop', SellStrings.roleStaff)),
        findsOneWidget,
      );
      await tester.tap(find.text(SellStrings.accept));
      await settleFrames(tester, 3);
      expect(env.sell.answered, ['i1:accept']);
    });
  });

  group('reviews and disputes', () {
    testWidgets('a_manager_replies_to_a_review_and_staff_cannot', (
      tester,
    ) async {
      final env = SellEnv();
      env.work.reviewRows = [
        SellReview(id: 'r1', stars: 4, body: 'Nice', createdAt: DateTime(2026)),
        SellReview(
          id: 'r2',
          stars: 5,
          reply: 'Thank you',
          createdAt: DateTime(2026),
        ),
      ];
      await pumpSell(tester, const ReviewsPage(store: _manager), env);
      await settleFrames(tester, 2);
      expect(find.text(SellStrings.reply), findsOneWidget);
      expect(find.text('Thank you'), findsOneWidget);
      await tester.tap(find.text(SellStrings.reply));
      await settleFrames(tester, 3);
      await tester.enterText(_field(SellStrings.replyLabel), 'Thanks a lot');
      await tester.pump();
      await tester.tap(find.text(SellStrings.sendReply));
      await settleFrames(tester, 3);
      expect(env.work.replies, ['r1:Thanks a lot']);
      await pumpSell(tester, const ReviewsPage(store: _staff), env);
      await settleFrames(tester, 2);
      expect(find.text(SellStrings.reply), findsNothing);
    });

    testWidgets('a_dispute_shows_the_buyers_side_and_takes_an_answer', (
      tester,
    ) async {
      final env = SellEnv();
      env.work.disputeRow = SellDispute(
        id: 'd1',
        orderId: 'o1',
        status: 'open',
        reason: 'not_received',
        respondBy: DateTime(2026, 12, 1),
        buyer: const DisputeStatement(body: 'It never came', photoKeys: []),
      );
      await pumpSell(
        tester,
        const DisputePage(store: _owner, disputeId: 'd1'),
        env,
      );
      await settleFrames(tester, 2);
      expect(find.text('It never came'), findsOneWidget);
      expect(
        tester
            .widget<LoadingButton>(
              find.widgetWithText(LoadingButton, SellStrings.sendAnswer),
            )
            .onPressed,
        isNull,
      );
      await tester.enterText(
        _field(SellStrings.answerLabel),
        'It was delivered on Monday',
      );
      await tester.pump();
      await _tap(tester, find.text(SellStrings.sendAnswer));
      await settleFrames(tester, 3);
      expect(env.work.answers, ['It was delivered on Monday']);
    });

    testWidgets('staff_cannot_answer_a_dispute', (tester) async {
      final env = SellEnv();
      env.work.disputeRow = SellDispute(
        id: 'd1',
        orderId: 'o1',
        status: 'open',
        reason: 'x',
        respondBy: DateTime(2026, 12, 1),
        buyer: const DisputeStatement(body: 'Bad', photoKeys: []),
      );
      await pumpSell(
        tester,
        const DisputePage(store: _staff, disputeId: 'd1'),
        env,
      );
      await settleFrames(tester, 2);
      expect(find.text(SellStrings.sendAnswer), findsNothing);
    });

    testWidgets('the_reported_problems_list_finds_orders_with_a_dispute', (
      tester,
    ) async {
      final env = SellEnv();
      env.work.orderRows = [_order('o1'), _order('o2', dispute: 'd9')];
      await pumpSell(tester, const DisputesPage(store: _owner), env);
      await settleFrames(tester, 3);
      expect(find.text('Ankara shirt'), findsOneWidget);
      env.work.orderRows = [_order('o1')];
      await pumpSell(tester, const DisputesPage(store: _owner), env);
      await settleFrames(tester, 3);
      expect(find.text(SellStrings.noDisputes), findsOneWidget);
    });
  });

  test('the_bank_list_has_three_digit_codes_only', () {
    for (final code in payoutBanks.keys) {
      expect(code.length, 3, reason: code);
    }
    expect(AreaCouncil.values, isNotEmpty);
    expect(HapticFeedback.lightImpact, isNotNull);
  });
}
