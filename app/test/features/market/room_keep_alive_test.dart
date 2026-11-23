import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart' show Color;
import 'package:market/features/market/data/market_source.dart';
import 'package:market/features/market/model/market.dart';
import 'package:market/features/market/model/room.dart';

import '../../support/fake_catalog_repository.dart';

class _Source implements MarketSource {
  int rooms = 0;

  @override
  Future<RoomView> room(String slug) async {
    rooms++;
    return RoomView(
      store: fakeStore(slug),
      type: const StoreType(
        id: 'boutique',
        roomImage: 'r',
        depthImage: 'd',
        slots: [],
        lightTint: Color(0xFFFFFFFF),
        shadowOpacity: 0.3,
      ),
      placed: const [],
      hasMore: false,
    );
  }

  @override
  Future<MarketSnapshot> market() => throw UnimplementedError();

  @override
  Future<StoreType> storeType(String id) => throw UnimplementedError();
}

void main() {
  late _Source source;
  late ProviderContainer container;

  /// Lets [minutes] pass, then the tick on which Riverpod drops what closed.
  Future<void> wait(WidgetTester tester, int minutes) async {
    await tester.pump(Duration(minutes: minutes));
    await tester.pump();
  }

  Future<void> load(WidgetTester tester) async {
    await container.read(roomProvider('ada').future);
  }

  setUp(() {
    source = _Source();
    container = ProviderContainer(
      overrides: [marketSourceProvider.overrideWith((ref) async => source)],
    );
  });
  tearDown(() => container.dispose());

  bool held() => container.read(roomProvider('ada').exists);

  testWidgets('a_prefetched_room_nobody_opens_is_dropped_after_5_minutes', (
    tester,
  ) async {
    await load(tester);
    await wait(tester, 4);
    expect(held(), isTrue);
    await wait(tester, 2);
    expect(held(), isFalse);
    expect(source.rooms, 1);
  });

  testWidgets('coming_back_within_5_minutes_resets_the_clock', (tester) async {
    final sub = container.listen(roomProvider('ada'), (_, _) {});
    await load(tester);
    sub.close();
    await wait(tester, 4);
    final again = container.listen(roomProvider('ada'), (_, _) {});
    await wait(tester, 4);
    again.close();
    // Eight minutes after the first visit, but only four since the second.
    await wait(tester, 4);
    expect(held(), isTrue);
    await wait(tester, 2);
    expect(held(), isFalse);
    expect(source.rooms, 1);
  });

  testWidgets('no_timer_outlives_the_container', (tester) async {
    await load(tester);
    container.dispose();
    container = ProviderContainer();
    // The test binding fails here if a five-minute timer is still pending.
  });
}
