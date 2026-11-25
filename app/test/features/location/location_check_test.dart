import 'dart:async';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:geolocator_platform_interface/geolocator_platform_interface.dart';
import 'package:market/features/location/location_check.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';

/// Stands in for the phone's location service.
class _FakePlatform extends GeolocatorPlatform with MockPlatformInterfaceMixin {
  bool servicesOn = true;
  LocationPermission permission = LocationPermission.whileInUse;
  LocationPermission afterRequest = LocationPermission.whileInUse;
  Future<Position> Function()? fix;
  int requests = 0;
  int fixes = 0;

  @override
  Future<bool> isLocationServiceEnabled() async => servicesOn;

  @override
  Future<LocationPermission> checkPermission() async => permission;

  @override
  Future<LocationPermission> requestPermission() async {
    requests++;
    return afterRequest;
  }

  @override
  Future<Position> getCurrentPosition({LocationSettings? locationSettings}) {
    fixes++;
    return fix!();
  }
}

Position _at(double lat, double lng) => Position(
  latitude: lat,
  longitude: lng,
  timestamp: DateTime(2026, 11, 25),
  accuracy: 1000,
  altitude: 0,
  altitudeAccuracy: 0,
  heading: 0,
  headingAccuracy: 0,
  speed: 0,
  speedAccuracy: 0,
);

void main() {
  late _FakePlatform platform;
  const check = GeolocatorLocationCheck();

  setUp(() {
    platform = _FakePlatform();
    GeolocatorPlatform.instance = platform;
  });

  test('a phone in Wuse is inside, one in Lagos is outside', () async {
    platform.fix = () async => _at(9.0765, 7.3986);
    expect(await check.currentArea(), Area.inside);
    platform.fix = () async => _at(6.5244, 3.3792);
    expect(await check.currentArea(), Area.outside);
  });

  test('location services off: unknown, and no fix is asked for', () async {
    platform.servicesOn = false;
    platform.fix = () async => _at(6.5244, 3.3792);
    expect(await check.currentArea(), Area.unknown);
    expect(platform.fixes, 0);
  });

  test('not allowed: unknown, no fix and never a prompt', () async {
    for (final p in [
      LocationPermission.denied,
      LocationPermission.deniedForever,
    ]) {
      platform.permission = p;
      platform.fix = () async => _at(6.5244, 3.3792);
      expect(await check.currentArea(), Area.unknown);
    }
    expect(platform.fixes, 0);
    expect(platform.requests, 0);
  });

  test('denied for good is never asked again', () async {
    platform.permission = LocationPermission.deniedForever;
    expect(await check.requestPermission(), isFalse);
    expect(platform.requests, 0);
  });

  test('not yet asked: one prompt, and its answer counts', () async {
    platform.permission = LocationPermission.denied;
    platform.afterRequest = LocationPermission.whileInUse;
    expect(await check.requestPermission(), isTrue);
    expect(platform.requests, 1);
  });

  test('a fix that never arrives gives unknown after 10 seconds', () {
    fakeAsync((async) {
      platform.fix = () => Completer<Position>().future;
      Area? result;
      check.currentArea().then((a) => result = a);
      async.elapse(const Duration(seconds: 9));
      expect(result, isNull);
      async.elapse(const Duration(seconds: 2));
      expect(result, Area.unknown);
    });
  });

  test('a failing fix gives unknown, never an error', () async {
    platform.fix = () async => throw const PermissionDeniedException('no');
    expect(await check.currentArea(), Area.unknown);
  });
}
