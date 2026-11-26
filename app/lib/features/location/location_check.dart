import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:geolocator/geolocator.dart';

import 'fct_area.dart';

/// Where the phone roughly is, as far as delivery is concerned.
enum Area { inside, outside, unknown }

/// Asks the phone where it is. The position is used once, in memory, to
/// pick an [Area]; it is never stored or sent anywhere.
abstract class LocationCheck {
  /// Shows the system prompt if needed; true when location may be read.
  Future<bool> requestPermission();

  /// The phone's [Area], without prompting. [Area.unknown] when location is
  /// off, not allowed, or slow.
  Future<Area> currentArea();
}

class GeolocatorLocationCheck implements LocationCheck {
  const GeolocatorLocationCheck();

  static const Duration _timeout = Duration(seconds: 10);

  @override
  Future<bool> requestPermission() async {
    try {
      var permission = await Geolocator.checkPermission();
      if (permission == LocationPermission.denied) {
        permission = await Geolocator.requestPermission();
      }
      return _allowed(permission);
    } on Exception {
      return false;
    }
  }

  @override
  Future<Area> currentArea() async {
    try {
      if (!await Geolocator.isLocationServiceEnabled()) return Area.unknown;
      if (!_allowed(await Geolocator.checkPermission())) return Area.unknown;
      final position = await Geolocator.getCurrentPosition(
        locationSettings: const LocationSettings(
          accuracy: LocationAccuracy.low,
          timeLimit: _timeout,
        ),
      ).timeout(_timeout);
      return inFct(position.latitude, position.longitude)
          ? Area.inside
          : Area.outside;
    } on Object {
      // Off, refused, timed out: no hint rather than a wrong one.
      return Area.unknown;
    }
  }

  static bool _allowed(LocationPermission p) =>
      p == LocationPermission.whileInUse || p == LocationPermission.always;
}

final locationCheckProvider = Provider<LocationCheck>(
  (ref) => const GeolocatorLocationCheck(),
);
