/// Small readers for hand-parsed server JSON. Each answers null for a missing
/// value or one of the wrong type, so a bad field never throws.
library;

String? readString(Object? v) => v is String && v.isNotEmpty ? v : null;

double? readDouble(Object? v) => v is num && v.isFinite ? v.toDouble() : null;

int? readInt(Object? v) =>
    v is int ? v : (v is num && v == v.roundToDouble() ? v.toInt() : null);

List<Map<String, dynamic>> readObjects(Object? v) => [
  if (v is List)
    for (final item in v)
      if (item is Map<String, dynamic>) item,
];
