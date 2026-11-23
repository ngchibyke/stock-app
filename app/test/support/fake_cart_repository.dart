import 'dart:async';

import 'package:market/core/api/api_error.dart';
import 'package:market/features/cart/cart.dart';
import 'package:market/features/cart/cart_repository.dart';

/// One cart line, with its total worked out the way the server does.
CartLine fakeLine(
  String variantId, {
  String product = 'Ankara dress',
  String storeId = 's1',
  String store = 'Ada Fashion',
  int unit = 1500000,
  int quantity = 1,
  bool available = true,
  Map<String, String> attributes = const {'Size': 'M'},
}) => CartLine(
  variantId: variantId,
  productId: 'p-$variantId',
  productName: product,
  storeId: storeId,
  storeName: store,
  attributes: attributes,
  unitPriceKobo: unit,
  quantity: quantity,
  lineTotalKobo: unit * quantity,
  available: available,
);

/// A [CartRepository] that holds lines the way the server would and records
/// every call. Totals come from [totalFor], so a test can make the server's
/// total differ from anything the app could add up itself.
class FakeCartRepository implements CartRepository {
  FakeCartRepository([List<CartLine> lines = const []]) : lines = [...lines];

  final List<CartLine> lines;
  final List<(String, int)> sets = [];
  int gets = 0;

  /// The total the "server" reports. Defaults to the sum of available lines.
  int Function(List<CartLine> lines)? totalFor;

  /// When set, [set] waits for it before answering.
  Completer<void>? hold;

  /// When set, [set] throws it.
  ApiError? failWith;

  Cart _cart() => Cart(
    items: [...lines],
    totalKobo:
        totalFor?.call(lines) ??
        lines
            .where((l) => l.available)
            .fold(0, (sum, l) => sum + l.lineTotalKobo),
  );

  @override
  Future<Cart> get() async {
    gets++;
    return _cart();
  }

  @override
  Future<Cart> set(String variantId, int quantity) async {
    sets.add((variantId, quantity));
    await hold?.future;
    if (failWith case final e?) throw e;
    final i = lines.indexWhere((l) => l.variantId == variantId);
    if (quantity == 0) {
      if (i >= 0) lines.removeAt(i);
    } else if (i >= 0) {
      final l = lines[i];
      lines[i] = fakeLine(
        l.variantId,
        product: l.productName,
        storeId: l.storeId,
        store: l.storeName,
        unit: l.unitPriceKobo,
        quantity: quantity,
        available: l.available,
        attributes: l.attributes,
      );
    } else {
      lines.add(fakeLine(variantId, quantity: quantity));
    }
    return _cart();
  }
}
