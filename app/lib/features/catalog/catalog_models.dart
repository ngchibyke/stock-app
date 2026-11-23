import '../../core/json_read.dart';

/// A store as the directory, the market and product cards show it.
class StoreSummary {
  const StoreSummary({
    required this.id,
    required this.slug,
    required this.name,
    this.logoUrl,
    this.ratingAverage,
    this.ratingCount = 0,
  });

  /// Null when the row has no usable id, slug or name.
  static StoreSummary? fromJson(Object? json) {
    if (json is! Map<String, dynamic>) return null;
    final id = readString(json['id']);
    final slug = readString(json['slug']);
    final name = readString(json['name']);
    if (id == null || slug == null || name == null) return null;
    return StoreSummary(
      id: id,
      slug: slug,
      name: name,
      logoUrl: readString(json['logo_url']),
      ratingAverage: readDouble(json['rating_average']),
      ratingCount: readInt(json['rating_count']) ?? 0,
    );
  }

  final String id;
  final String slug;
  final String name;
  final String? logoUrl;

  /// Null with no reviews.
  final double? ratingAverage;
  final int ratingCount;

  Map<String, dynamic> toJson() => {
    'id': id,
    'slug': slug,
    'name': name,
    'logo_url': logoUrl,
    'rating_average': ratingAverage,
    'rating_count': ratingCount,
  };
}

/// One product in a list or on a shelf (`ProductCard` on the server).
class ProductCard {
  const ProductCard({
    required this.id,
    required this.name,
    required this.category,
    required this.condition,
    required this.priceFromKobo,
    required this.inStock,
    required this.store,
    this.photoUrl,
  });

  /// Null when the row is unusable (no id, name, price or store).
  static ProductCard? fromJson(Object? json) {
    if (json is! Map<String, dynamic>) return null;
    final id = readString(json['id']);
    final name = readString(json['name']);
    final price = readInt(json['price_from_kobo']);
    final store = StoreSummary.fromJson(json['store']);
    if (id == null ||
        name == null ||
        price == null ||
        price < 0 ||
        store == null) {
      return null;
    }
    return ProductCard(
      id: id,
      name: name,
      category: readString(json['category']) ?? 'other',
      condition: readString(json['condition']) ?? 'new',
      priceFromKobo: price,
      photoUrl: readString(json['photo_url']),
      inStock: json['in_stock'] == true,
      store: store,
    );
  }

  final String id;
  final String name;
  final String category;
  final String condition;
  final int priceFromKobo;
  final String? photoUrl;
  final bool inStock;
  final StoreSummary store;

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'category': category,
    'condition': condition,
    'price_from_kobo': priceFromKobo,
    'photo_url': photoUrl,
    'in_stock': inStock,
    'store': store.toJson(),
  };
}

class Category {
  const Category({required this.slug, required this.name});

  final String slug;
  final String name;
}

/// One buyable version of a product (a size, a colour).
class Variant {
  const Variant({
    required this.id,
    required this.attributes,
    required this.availableStock,
    required this.priceKobo,
  });

  final String id;
  final Map<String, String> attributes;

  /// Units a buyer can still buy.
  final int availableStock;

  /// The price a buyer pays: the variant's own, else the product's base price.
  final int priceKobo;
}

class ProductDetail {
  const ProductDetail({
    required this.id,
    required this.storeId,
    required this.name,
    required this.description,
    required this.condition,
    required this.photoUrls,
    required this.variants,
    this.brand,
  });

  final String id;
  final String storeId;
  final String name;
  final String description;
  final String condition;
  final String? brand;
  final List<String> photoUrls;
  final List<Variant> variants;
}

class StoreDetail {
  const StoreDetail({
    required this.summary,
    required this.description,
    this.bannerUrl,
  });

  final StoreSummary summary;
  final String description;
  final String? bannerUrl;
}

/// One page of a cursor-paged list.
class Page<T> {
  const Page({required this.items, this.nextCursor});

  final List<T> items;
  final String? nextCursor;
}
