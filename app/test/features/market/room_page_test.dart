import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/app.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/core/ui/states.dart';
import 'package:market/features/cart/cart.dart';
import 'package:market/features/cart/cart_repository.dart';
import 'package:market/features/market/data/asset_resolver.dart';
import 'package:market/features/market/data/market_source.dart';
import 'package:market/features/market/model/market.dart';
import 'package:market/features/market/model/room.dart';
import 'package:market/features/market/room/depth_room.dart';
import 'package:market/features/market/room/room_page.dart';
import 'package:market/features/market/room/shelf_card.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../support/fake_asset_resolver.dart';
import '../../support/fake_catalog_repository.dart';

const _type = StoreType(
  id: 'boutique',
  roomImage: 'room.jpg',
  depthImage: 'depth.png',
  slots: [
    ShelfSlot(
      id: 's1',
      rect: Rect.fromLTWH(0.1, 0.2, 0.25, 0.15),
      depth: 0.3,
      tiltY: 0,
    ),
    ShelfSlot(
      id: 's2',
      rect: Rect.fromLTWH(0.4, 0.2, 0.25, 0.15),
      depth: 0.6,
      tiltY: 0,
    ),
    ShelfSlot(
      id: 's3',
      rect: Rect.fromLTWH(0.7, 0.2, 0.25, 0.15),
      depth: 0.9,
      tiltY: 0,
    ),
  ],
  lightTint: Color(0xFFFFFFFF),
  shadowOpacity: 0.3,
);

final _placement = Placement(
  plotId: 'p1',
  store: fakeStore('ada'),
  storeType: 'boutique',
);

RoomView _room({int products = 2, bool hasMore = false}) => RoomView(
  store: fakeStore('ada'),
  type: _type,
  placed: [
    for (var i = 0; i < products; i++)
      PlacedProduct(
        slotId: 's${i + 1}',
        product: fakeCard('p$i', name: 'Thing $i'),
      ),
  ],
  hasMore: hasMore,
);

class _NoCart extends CartNotifier {
  @override
  Future<Cart> build() async => Cart.empty;
}

class _Harness {
  int loads = 0;
  int backs = 0;
  int seeAll = 0;
  final opened = <String>[];

  Future<void> pump(
    WidgetTester tester,
    Future<RoomView> Function() load,
  ) async {
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      ProviderScope(
        key: UniqueKey(),
        retry: noRetry,
        overrides: [
          assetResolverProvider.overrideWithValue(FakeAssetResolver()),
          cartProvider.overrideWith(_NoCart.new),
          storeTypeProvider.overrideWith((ref, id) async => _type),
          roomProvider.overrideWith((ref, slug) {
            loads++;
            return load();
          }),
        ],
        child: MaterialApp(
          theme: lightTheme(),
          home: RoomPage(
            placement: _placement,
            onBack: () => backs++,
            onOpenProduct: opened.add,
            onSeeAll: () => seeAll++,
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
  }
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  testWidgets('room_shows_skeleton_cards_while_loading', (tester) async {
    final h = _Harness();
    final never = Completer<RoomView>();
    await h.pump(tester, () => never.future);
    // The room itself shows at once; each slot holds a skeleton card.
    expect(find.byType(DepthRoom), findsOneWidget);
    expect(find.byKey(const Key('shelf-skeleton')), findsNWidgets(3));
    expect(find.byType(ShelfCard), findsNothing);
    expect(find.text('Store ada'), findsOneWidget);
  });

  testWidgets('room_error_has_retry_and_no_spinner', (tester) async {
    final h = _Harness();
    await h.pump(
      tester,
      () async => throw ApiError(status: 500, code: 'server', message: 'x'),
    );
    expect(find.byType(ErrorState), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsNothing);
    expect(find.byType(DepthRoom), findsOneWidget);
    await tester.tap(find.text(Strings.retry));
    await tester.pump();
    expect(h.loads, 2);
  });

  testWidgets('closed_store_says_so_and_goes_back', (tester) async {
    final h = _Harness();
    await h.pump(
      tester,
      () async =>
          throw ApiError(status: 404, code: 'store_not_found', message: 'x'),
    );
    expect(find.text(Strings.storeClosed), findsOneWidget);
    expect(find.text(Strings.retry), findsNothing);
    await tester.tap(find.text(Strings.backToMarket));
    expect(h.backs, 1);
  });

  testWidgets('empty_room_says_so', (tester) async {
    final h = _Harness();
    await h.pump(tester, () async => _room(products: 0));
    expect(find.text(Strings.emptyShelves), findsOneWidget);
    await tester.tap(find.text(Strings.backToMarket));
    expect(h.backs, 1);
  });

  testWidgets('see_all_only_when_has_more', (tester) async {
    final h = _Harness();
    await h.pump(tester, () async => _room());
    expect(find.text(Strings.seeAll), findsNothing);

    final h2 = _Harness();
    await h2.pump(tester, () async => _room(hasMore: true));
    await tester.tap(find.text(Strings.seeAll));
    expect(h2.seeAll, 1);
  });

  testWidgets('tapping_a_card_opens_its_product', (tester) async {
    final h = _Harness();
    await h.pump(tester, () async => _room());
    expect(find.byType(ShelfCard), findsNWidgets(2));
    expect(find.byKey(const Key('shelf-skeleton')), findsNothing);
    await tester.tap(find.byType(ShelfCard).at(1));
    expect(h.opened, ['p1']);
  });

  testWidgets('the_top_bar_back_goes_to_the_market', (tester) async {
    final h = _Harness();
    await h.pump(tester, () async => _room());
    await tester.tap(find.bySemanticsLabel(Strings.backToMarket));
    expect(h.backs, 1);
  });
}
