import '../../core/json_read.dart';

/// One line of the cart as the server prices it. Every amount is the server's.
class CartLine {
  const CartLine({
    required this.variantId,
    required this.productId,
    required this.productName,
    required this.storeId,
    required this.storeName,
    required this.attributes,
    required this.unitPriceKobo,
    required this.quantity,
    required this.lineTotalKobo,
    required this.available,
  });

  /// Null when the row has no usable variant id.
  static CartLine? fromJson(Object? json) {
    if (json is! Map<String, dynamic>) return null;
    final variantId = readString(json['variant_id']);
    if (variantId == null) return null;
    final attrs = json['attributes'];
    return CartLine(
      variantId: variantId,
      productId: readString(json['product_id']) ?? '',
      productName: readString(json['product_name']) ?? '',
      storeId: readString(json['store_id']) ?? '',
      storeName: readString(json['store_name']) ?? '',
      attributes: {
        if (attrs is Map<String, dynamic>)
          for (final e in attrs.entries)
            if (e.value case final String v) e.key: v,
      },
      unitPriceKobo: readInt(json['unit_price_kobo']) ?? 0,
      quantity: readInt(json['quantity']) ?? 0,
      lineTotalKobo: readInt(json['line_total_kobo']) ?? 0,
      available: json['available'] == true,
    );
  }

  final String variantId;
  final String productId;
  final String productName;
  final String storeId;
  final String storeName;
  final Map<String, String> attributes;
  final int unitPriceKobo;
  final int quantity;
  final int lineTotalKobo;

  /// False when the product or its store can no longer be bought.
  final bool available;
}

/// The buyer's cart. Held in memory only.
class Cart {
  const Cart({required this.items, required this.totalKobo});

  static const empty = Cart(items: [], totalKobo: 0);

  /// Throws a [FormatException] when [json] is not a cart.
  factory Cart.fromJson(Object? json) {
    if (json is! Map<String, dynamic>) {
      throw const FormatException('cart is not an object');
    }
    final items = json['items'];
    return Cart(
      items: [
        if (items is List)
          for (final row in items) ?CartLine.fromJson(row),
      ],
      totalKobo: readInt(json['total_kobo']) ?? 0,
    );
  }

  final List<CartLine> items;

  /// The server's total of the available lines.
  final int totalKobo;

  /// Things the buyer can still buy: quantities of available lines.
  int get count =>
      items.where((l) => l.available).fold(0, (sum, l) => sum + l.quantity);

  /// Lines by store name, in the order each store first appears.
  Map<String, List<CartLine>> get byStore {
    final out = <String, List<CartLine>>{};
    for (final l in items) {
      (out[l.storeName] ??= []).add(l);
    }
    return out;
  }

  int quantityOf(String variantId) {
    for (final l in items) {
      if (l.variantId == variantId) return l.quantity;
    }
    return 0;
  }
}
