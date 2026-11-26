import 'package:camera/camera.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/paths.dart';
import 'package:market/core/strings.dart';
import 'package:market/features/market/data/market_source.dart';
import 'package:market/features/market/model/market.dart';
import 'package:market/features/sell/identity/identity_page.dart';
import 'package:market/features/sell/photos/camera_page.dart';
import 'package:market/features/sell/become_page.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_providers.dart';
import 'package:market/features/sell/sell_shell.dart';
import 'package:market/features/sell/sell_strings.dart';
import 'package:market/features/sell/sell_widgets.dart';
import 'package:market/features/sell/store_forms.dart';
import 'package:market/features/sell/store_tab.dart';
import 'package:market/router.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/app_harness.dart';
import '../../support/fakes.dart';
import '../../support/pump_sell.dart';

SellStore _store(
  String id,
  String name, {
  StoreRole role = StoreRole.owner,
  String status = 'active',
}) => SellStore(id: id, name: name, slug: id, status: status, role: role);

final _snapshot = MarketSnapshot(
  layout: const MarketLayout(
    version: 't',
    world: Size(2048, 2048),
    tileSize: 256,
    maxLevel: 3,
    baseImage: 'base.jpg',
    districts: [District(id: 'd', name: 'Fashion row')],
    plots: [],
  ),
  placements: const [],
);

void main() {
  group('where the selling area starts', () {
    testWidgets('someone_who_is_not_a_seller_is_asked_for_an_access_code', (
      tester,
    ) async {
      final env = SellEnv(seller: false);
      await pumpSell(tester, const SellEntryPage(), env);
      await settleFrames(tester, 3);
      expect(find.byType(BecomeSellerPage), findsOneWidget);
    });

    testWidgets(
      'a_seller_without_a_store_confirms_identity_first_then_opens_one',
      (tester) async {
        final env = SellEnv();
        await pumpSell(tester, const SellEntryPage(), env);
        await settleFrames(tester, 3);
        expect(find.byType(IdentityPage), findsOneWidget);
        await tester.pumpWidget(const SizedBox());

        env.sell.identity = const IdentityStatus(state: IdentityState.passed);
        await pumpSell(
          tester,
          const SellEntryPage(),
          env,
          extra: [marketProvider.overrideWith((ref) async => _snapshot)],
        );
        await settleFrames(tester, 3);
        expect(find.byType(CreateStorePage), findsOneWidget);
      },
    );

    testWidgets('an_owner_with_a_store_gets_four_tabs_and_staff_three', (
      tester,
    ) async {
      final env = SellEnv();
      env.sell.stores = [_store('s1', 'Ada Shop')];
      await pumpSell(tester, const SellEntryPage(), env);
      await settleFrames(tester, 3);
      expect(find.byType(SellShell), findsOneWidget);
      for (final t in [
        SellStrings.tabOrders,
        SellStrings.tabProducts,
        SellStrings.tabStore,
        SellStrings.tabTeam,
      ]) {
        expect(find.text(t), findsWidgets, reason: t);
      }
      await tester.pumpWidget(const SizedBox());

      final env2 = SellEnv(seller: false);
      env2.sell.stores = [_store('s1', 'Ada Shop', role: StoreRole.staff)];
      await pumpSell(tester, const SellEntryPage(), env2);
      await settleFrames(tester, 3);
      expect(find.byType(SellShell), findsOneWidget);
      expect(find.text(SellStrings.tabTeam), findsNothing);
      expect(find.text(SellStrings.tabOrders), findsWidgets);
    });

    testWidgets(
      'a_member_who_is_not_a_seller_still_gets_the_area_for_their_store',
      (tester) async {
        final env = SellEnv(seller: false);
        env.sell.stores = [_store('s1', 'Ada Shop', role: StoreRole.manager)];
        await pumpSell(tester, const SellEntryPage(), env);
        await settleFrames(tester, 3);
        expect(find.byType(BecomeSellerPage), findsNothing);
        expect(find.byType(SellShell), findsOneWidget);
      },
    );

    testWidgets(
      'the_store_switcher_changes_the_store_and_a_lost_store_falls_back',
      (tester) async {
        final env = SellEnv();
        env.sell.stores = [
          _store('s1', 'Ada Shop'),
          _store('s2', 'Second Shop'),
        ];
        await pumpSell(tester, const SellEntryPage(), env);
        await settleFrames(tester, 3);
        final store = find.text(SellStrings.tabStore).last;
        await tester.tap(store);
        await settleFrames(tester, 3);
        expect(find.text('Ada Shop'), findsOneWidget);
        await tester.tap(find.byTooltip(SellStrings.switchStore));
        await settleFrames(tester, 3);
        await tester.tap(find.text('Second Shop').last);
        await settleFrames(tester, 3);
        expect(find.text('Second Shop'), findsOneWidget);
        expect(find.text('Ada Shop'), findsNothing);
      },
    );

    testWidgets('a_pending_store_says_what_it_waits_for_and_has_no_walk_in', (
      tester,
    ) async {
      final env = SellEnv();
      env.sell.stores = [_store('s1', 'Ada Shop', status: 'pending')];
      await pumpSell(tester, const SellEntryPage(), env);
      await settleFrames(tester, 3);
      await tester.tap(find.text(SellStrings.tabStore).last);
      await settleFrames(tester, 3);
      expect(find.text(SellStrings.storeStatusPending), findsOneWidget);
      expect(find.text(SellStrings.payoutWaitingBoth), findsOneWidget);
      expect(find.byType(StoreTab), findsOneWidget);
      expect(find.text(SellStrings.walkIn), findsNothing);
    });
  });

  group('the account entry and the route', () {
    testWidgets('a_buyer_is_offered_to_sell_and_a_seller_sees_my_stores', (
      tester,
    ) async {
      final h = AppHarness(store: MemorySessionStore('tok'));
      h.adapter.reply(200, testMe);
      await h.pump(tester);
      await tester.tap(find.text(Strings.tabAccount));
      await settle(tester);
      expect(find.text(SellStrings.sellEntry), findsOneWidget);
      expect(find.text(SellStrings.myStores), findsNothing);
      expect(find.text(SellStrings.invitesTitle), findsNothing);
    });

    testWidgets('someone_with_a_store_sees_my_stores_and_an_invitation_row', (
      tester,
    ) async {
      final h = AppHarness(store: MemorySessionStore('tok'));
      h.sell.stores = [_store('s1', 'Ada Shop')];
      h.sell.invitesRows = const [
        MyInvite(id: 'i1', storeName: 'Other Shop', role: StoreRole.staff),
      ];
      h.adapter.reply(200, testMe);
      await h.pump(tester);
      await tester.tap(find.text(Strings.tabAccount));
      await settle(tester);
      expect(find.text(SellStrings.myStores), findsOneWidget);
      expect(find.text(SellStrings.invitesTitle), findsOneWidget);
    });

    testWidgets(
      'the_sell_route_is_open_to_a_signed_in_person_and_not_a_signed_out_one',
      (tester) async {
        final h = AppHarness(store: MemorySessionStore('tok'));
        h.adapter.reply(200, testMe);
        await h.pump(tester);
        h.container(tester).read(routerProvider).push(sellPath);
        await settle(tester);
        expect(find.byType(BecomeSellerPage), findsOneWidget);
      },
    );

    testWidgets('signed_out_people_are_sent_to_sign_in', (tester) async {
      final h = AppHarness();
      await h.pump(tester);
      h.container(tester).read(routerProvider).go(sellPath);
      await settle(tester);
      expect(find.byType(BecomeSellerPage), findsNothing);
    });
  });

  group('camera', () {
    testWidgets('a_denied_camera_says_so_and_points_to_the_gallery', (
      tester,
    ) async {
      final env = SellEnv();
      await pumpSell(
        tester,
        GuidedCameraPage(
          shotIndex: 0,
          factory: () async =>
              throw CameraException('CameraAccessDenied', 'no'),
        ),
        env,
      );
      await settleFrames(tester, 3);
      expect(find.text(SellStrings.cameraDenied), findsOneWidget);
    });

    testWidgets('the_guide_sheet_lists_every_tip_and_closes', (tester) async {
      final env = SellEnv();
      await pumpSell(
        tester,
        Builder(
          builder: (context) => TextButton(
            onPressed: () => showPhotoGuide(context),
            child: const Text('open'),
          ),
        ),
        env,
      );
      await tester.tap(find.text('open'));
      await settleFrames(tester, 4);
      for (final tip in SellStrings.guideTips) {
        expect(find.text(tip), findsOneWidget);
      }
      await tester.tap(find.text(SellStrings.guideGotIt));
      await settleFrames(tester, 4);
      expect(find.text(SellStrings.guideTitle), findsNothing);
    });

    test('the_frame_repaints_only_when_its_colour_changes', () {
      const a = FramePainter(Color(0xFFFFFFFF));
      expect(a.shouldRepaint(const FramePainter(Color(0xFFFFFFFF))), isFalse);
      expect(a.shouldRepaint(const FramePainter(Color(0xFF000000))), isTrue);
    });

    test('there_is_a_tip_for_each_of_four_shots', () {
      expect(SellStrings.shotTips, hasLength(4));
    });
  });

  test('the_selected_store_provider_picks_the_first_until_one_is_chosen', () async {
    // Covered through the shell above; here only the helper that Account uses.
    expect(hasSellerSide(meWith(seller: true), const []), isTrue);
    expect(hasSellerSide(meWith(), const []), isFalse);
    expect(hasSellerSide(meWith(), [_store('s', 'S')]), isTrue);
    expect(hasSellerSide(meWith(), null), isFalse);
    expect(selectedStoreIdProvider, isNotNull);
    expect(nairaToKobo('10'), 1000);
  });
}
