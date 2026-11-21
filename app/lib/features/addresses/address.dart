import 'package:flutter/foundation.dart';

import '../../core/strings.dart';

/// The six area councils the service delivers to. [wire] is the value the
/// server uses; [label] is what the person reads. The server may one day send a
/// council this app does not know: [fromWire] answers null for it and the page
/// shows [Strings.unknownCouncil], never the raw value.
enum AreaCouncil {
  amac('amac', 'AMAC (Abuja Municipal)'),
  bwari('bwari', 'Bwari'),
  gwagwalada('gwagwalada', 'Gwagwalada'),
  kuje('kuje', 'Kuje'),
  kwali('kwali', 'Kwali'),
  abaji('abaji', 'Abaji');

  const AreaCouncil(this.wire, this.label);

  final String wire;
  final String label;

  static AreaCouncil? fromWire(Object? value) {
    if (value is! String) return null;
    for (final c in values) {
      if (c.wire == value) return c;
    }
    return null;
  }
}

/// Server limits, mirrored at the form fields so a longer value is stopped
/// there.
abstract final class AddressLimits {
  static const int recipientName = 100;
  static const int addressLine = 300;
  static const int landmark = 200;
  static const int postcode = 11;
}

String? _optionalString(Object? value) {
  if (value is! String) return null;
  final trimmed = value.trim();
  return trimmed.isEmpty ? null : trimmed;
}

/// One saved delivery address, as answered by `GET /me/addresses`.
///
/// Personal data: held in memory only, never written to disk or logged.
@immutable
class Address {
  const Address({
    required this.id,
    required this.areaCouncil,
    required this.postcode,
    required this.recipientName,
    required this.phone,
    required this.addressLine,
    required this.landmark,
  });

  /// Never throws: a missing or odd field becomes a plain fallback (an empty
  /// string, or null for the optional ones). A row with no usable [id] is
  /// dropped by the repository, since it could be neither shown safely nor
  /// deleted.
  factory Address.fromJson(Map<String, dynamic> json) {
    final id = json['id'];
    String text(String key) {
      final v = json[key];
      return v is String ? v : '';
    }

    return Address(
      id: id is String ? id : '',
      areaCouncil: AreaCouncil.fromWire(json['area_council']),
      postcode: _optionalString(json['postcode']),
      recipientName: text('recipient_name'),
      phone: text('phone'),
      addressLine: text('address_line'),
      landmark: _optionalString(json['landmark']),
    );
  }

  final String id;

  /// Null for a council this app does not know.
  final AreaCouncil? areaCouncil;
  final String? postcode;
  final String recipientName;
  final String phone;
  final String addressLine;
  final String? landmark;

  /// What the list shows for the council; never a raw wire value.
  String get councilLabel => areaCouncil?.label ?? Strings.unknownCouncil;

  // Never includes the address.
  @override
  String toString() => 'Address';
}

/// The body of `POST /me/addresses`.
@immutable
class NewAddress {
  const NewAddress({
    required this.recipientName,
    required this.phone,
    required this.addressLine,
    required this.areaCouncil,
    this.landmark,
    this.postcode,
  });

  final String recipientName;
  final String phone;
  final String addressLine;
  final String? landmark;
  final AreaCouncil areaCouncil;
  final String? postcode;

  /// The server's exact keys. A blank landmark or postcode is left out.
  Map<String, Object> toJson() {
    final landmark = _optionalString(this.landmark);
    final postcode = _optionalString(this.postcode);
    return {
      'recipient_name': recipientName.trim(),
      'phone': phone.trim(),
      'address_line': addressLine.trim(),
      'landmark': ?landmark,
      'area_council': areaCouncil.wire,
      'postcode': ?postcode,
    };
  }

  // Never includes the address.
  @override
  String toString() => 'NewAddress';
}
