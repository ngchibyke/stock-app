import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/format/money.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/market/model/room.dart';
import 'package:market/features/market/room/shelf_card.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/fake_catalog_repository.dart';

const _type = StoreType(
  id: 'boutique',
  roomImage: 'room.jpg',
  depthImage: 'depth.png',
  slots: [],
  lightTint: Color(0xFFFFE0B0),
  shadowOpacity: 0.3,
);

ShelfSlot _slot({double depth = 0.5, double tilt = 0.1}) => ShelfSlot(
  id: 's1',
  rect: const Rect.fromLTWH(0.1, 0.1, 0.2, 0.15),
  depth: depth,
  tiltY: tilt,
);

Future<void> _pump(
  WidgetTester tester,
  Widget card, {
  double textScale = 1,
}) async {
  await tester.pumpWidget(
    MaterialApp(
      theme: lightTheme(),
      home: MediaQuery(
        data: MediaQueryData(textScaler: TextScaler.linear(textScale)),
        child: Scaffold(body: Center(child: card)),
      ),
    ),
  );
  await tester.pump();
}

void main() {
  testWidgets('card_shows_photo_name_label_and_from_price', (tester) async {
    var taps = 0;
    await _pump(
      tester,
      ShelfCard(
        product: fakeCard(
          'p1',
          name: 'Ankara dress',
          price: 1500000,
          photoUrl: 'https://cdn.test/p1.jpg',
        ),
        slot: _slot(),
        type: _type,
        size: const Size(90, 120),
        look: Offset.zero,
        onTap: () => taps++,
      ),
    );
    final price = formatNaira(1500000);
    expect(find.text(Strings.fromPrice(price)), findsOneWidget);
    expect(
      find.bySemanticsLabel(Strings.shelfCardLabel('Ankara dress', price)),
      findsOneWidget,
    );
    final photo = tester.widget<Image>(find.byKey(const Key('shelf-photo')));
    expect((photo.image as NetworkImage).url, 'https://cdn.test/p1.jpg');
    await tester.tap(find.byType(ShelfCard));
    expect(taps, 1);
  });

  testWidgets('card_without_photo_shows_the_neutral_tile', (tester) async {
    await _pump(
      tester,
      ShelfCard(
        product: fakeCard('p1'),
        slot: _slot(),
        type: _type,
        size: const Size(90, 120),
        look: Offset.zero,
        onTap: () {},
      ),
    );
    expect(find.byKey(const Key('shelf-photo')), findsNothing);
    expect(find.byIcon(Icons.shopping_bag_outlined), findsOneWidget);
  });

  testWidgets('card_drifts_more_when_nearer', (tester) async {
    double drift(double depth) {
      final card = ShelfCard(
        product: fakeCard('p1'),
        slot: _slot(depth: depth),
        type: _type,
        size: const Size(90, 120),
        look: const Offset(1, 0),
        onTap: () {},
      );
      return card.drift.dx;
    }

    expect(drift(1), closeTo(6, 1e-9));
    expect(drift(0.25), closeTo(1.5, 1e-9));
    expect(drift(1), greaterThan(drift(0.25)));
  });

  testWidgets('card_hit_area_is_at_least_48dp', (tester) async {
    var taps = 0;
    await _pump(
      tester,
      ShelfCard(
        product: fakeCard('p1'),
        slot: _slot(),
        type: _type,
        size: const Size(30, 36),
        look: Offset.zero,
        onTap: () => taps++,
      ),
    );
    final size = tester.getSize(find.byType(ShelfCard));
    expect(size.width, greaterThanOrEqualTo(48));
    expect(size.height, greaterThanOrEqualTo(48));
    // A tap near the edge of the 48 dp box, outside the small card, still counts.
    final box = tester.getRect(find.byType(ShelfCard));
    await tester.tapAt(box.topLeft + const Offset(2, 2));
    expect(taps, 1);
  });

  testWidgets('price_tag_at_200_percent_does_not_overflow', (tester) async {
    await _pump(
      tester,
      ShelfCard(
        product: fakeCard('p1', price: 123456789),
        slot: _slot(),
        type: _type,
        size: const Size(70, 90),
        look: Offset.zero,
        onTap: () {},
      ),
      textScale: 2,
    );
    expect(tester.takeException(), isNull);
  });
}
