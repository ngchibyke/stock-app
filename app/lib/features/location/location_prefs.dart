import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// What the phone remembers about the location hint. Never the position:
/// only whether we asked, and the day the banner was last closed.
class LocationPrefs {
  LocationPrefs(this._prefs);

  static const String askedKey = 'location.asked';
  static const String closedOnKey = 'location.banner_closed_on';

  final SharedPreferences _prefs;

  bool get asked => _prefs.getBool(askedKey) ?? false;

  Future<void> markAsked() => _prefs.setBool(askedKey, true);

  bool closedOn(DateTime day) => _prefs.getString(closedOnKey) == _day(day);

  Future<void> closeToday(DateTime now) =>
      _prefs.setString(closedOnKey, _day(now));

  static String _day(DateTime d) =>
      '${d.year.toString().padLeft(4, '0')}-'
      '${d.month.toString().padLeft(2, '0')}-'
      '${d.day.toString().padLeft(2, '0')}';
}

final locationPrefsProvider = FutureProvider<LocationPrefs>(
  (ref) async => LocationPrefs(await SharedPreferences.getInstance()),
);

/// Today, for the banner's once-a-day rule; replaced in tests.
final locationClockProvider = Provider<DateTime Function()>(
  (ref) => DateTime.now,
);
