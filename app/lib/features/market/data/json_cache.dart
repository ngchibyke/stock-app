import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

/// Keeps the last market and rooms on the phone so the app can show them
/// offline. Only public market data goes here, never anything about a person
/// (the cart is never cached).
abstract class JsonCache {
  Future<Map<String, dynamic>?> read(String key);

  Future<void> write(String key, Map<String, dynamic> json);
}

class PrefsJsonCache implements JsonCache {
  PrefsJsonCache(this._prefs);

  final SharedPreferences _prefs;

  @override
  Future<Map<String, dynamic>?> read(String key) async {
    try {
      final raw = _prefs.getString(key);
      if (raw == null) return null;
      final json = jsonDecode(raw);
      return json is Map<String, dynamic> ? json : null;
    } on Object {
      return null;
    }
  }

  @override
  Future<void> write(String key, Map<String, dynamic> json) async {
    try {
      await _prefs.setString(key, jsonEncode(json));
    } on Object {
      // A cache that cannot be written only costs the offline view.
    }
  }
}

class MemoryJsonCache implements JsonCache {
  final Map<String, String> _store = {};

  Set<String> get keys => _store.keys.toSet();

  @override
  Future<Map<String, dynamic>?> read(String key) async {
    final raw = _store[key];
    return raw == null ? null : jsonDecode(raw) as Map<String, dynamic>;
  }

  @override
  Future<void> write(String key, Map<String, dynamic> json) async {
    _store[key] = jsonEncode(json);
  }
}
