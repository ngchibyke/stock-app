import 'package:flutter/foundation.dart';

import '../../core/json_read.dart';

/// One buyable version of a product, as the team sees it.
@immutable
class ListingVariant {
  const ListingVariant({
    required this.id,
    required this.attributes,
    required this.stock,
    this.priceKobo,
  });

  static ListingVariant? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    if (id == null) return null;
    final attrs = j['attributes'];
    return ListingVariant(
      id: id,
      attributes: {
        if (attrs is Map)
          for (final e in attrs.entries)
            if (e.key is String && e.value is String)
              e.key as String: e.value as String,
      },
      stock: readInt(j['stock']) ?? readInt(j['available_stock']) ?? 0,
      priceKobo: readInt(j['price_kobo']),
    );
  }

  final String id;
  final Map<String, String> attributes;
  final int stock;
  final int? priceKobo;

  String get label => attributes.isEmpty ? '' : attributes.values.join(' / ');
}

@immutable
class Listing {
  const Listing({
    required this.id,
    required this.name,
    required this.description,
    required this.category,
    required this.condition,
    required this.basePriceKobo,
    required this.published,
    required this.photoKeys,
    required this.photoUrls,
    required this.variants,
    this.brand,
    this.weightGrams = 500,
  });

  static Listing? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    if (id == null) return null;
    List<String> strings(Object? v) => [
      if (v is List)
        for (final s in v)
          if (s is String && s.isNotEmpty) s,
    ];
    return Listing(
      id: id,
      name: readString(j['name']) ?? '',
      description: readString(j['description']) ?? '',
      category: readString(j['category']) ?? '',
      condition: readString(j['condition']) ?? 'new',
      brand: readString(j['brand']),
      basePriceKobo: readInt(j['base_price_kobo']) ?? 0,
      published: j['is_published'] == true,
      photoKeys: strings(j['photo_keys']),
      photoUrls: strings(j['photo_urls']),
      weightGrams: readInt(j['weight_grams']) ?? 500,
      variants: [
        for (final v in readObjects(j['variants'])) ?ListingVariant.fromJson(v),
      ],
    );
  }

  final String id;
  final String name;
  final String description;
  final String category;
  final String condition;
  final String? brand;
  final int basePriceKobo;
  final bool published;
  final List<String> photoKeys;
  final List<String> photoUrls;
  final int weightGrams;
  final List<ListingVariant> variants;

  int get totalStock => variants.fold(0, (a, v) => a + v.stock);
}

/// A variant to save: a label for its attributes (for example a size) and the
/// stock at each branch (a store with one branch sends one number).
@immutable
class VariantInput {
  const VariantInput({required this.attributes, required this.stock});

  final Map<String, String> attributes;

  /// Branch id to count.
  final Map<String, int> stock;

  Object toJson(bool singleBranch) => {
    'attributes': attributes,
    'stock': singleBranch ? stock.values.fold(0, (a, b) => a + b) : stock,
  };
}

/// The fields of a product to create or change.
@immutable
class ListingInput {
  const ListingInput({
    required this.name,
    required this.description,
    required this.category,
    required this.condition,
    required this.priceKobo,
    required this.photoKeys,
    this.brand,
    this.weightGrams = 500,
    this.variants = const [],
  });

  final String name;
  final String description;
  final String category;
  final String condition;
  final int priceKobo;
  final List<String> photoKeys;
  final String? brand;
  final int weightGrams;
  final List<VariantInput> variants;

  Map<String, Object?> toCreateJson(bool singleBranch) => {
    'name': name,
    'description': description,
    'category': category,
    'condition': condition,
    if (brand != null) 'brand': brand,
    'photo_keys': photoKeys,
    'weight_grams': weightGrams,
    'base_price_kobo': priceKobo,
    'variants': [for (final v in variants) v.toJson(singleBranch)],
  };

  Map<String, Object?> toPatchJson() => {
    'name': name,
    'description': description,
    'category': category,
    'condition': condition,
    if (brand != null) 'brand': brand,
    'photo_keys': photoKeys,
    'weight_grams': weightGrams,
    'base_price_kobo': priceKobo,
  };
}

/// A ticket to upload one photo straight to the object store.
@immutable
class UploadTicket {
  const UploadTicket({
    required this.uploadId,
    required this.url,
    required this.headers,
  });

  final String uploadId;
  final String url;
  final Map<String, String> headers;

  @override
  String toString() => 'UploadTicket(<redacted>)';
}
