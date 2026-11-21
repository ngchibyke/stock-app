import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../api/api_client.dart';
import '../api/api_error.dart';
import '../api/providers.dart';

const _cacheKey = 'app_config.v1';

String? _text(Object? value) {
  if (value is! String) return null;
  final trimmed = value.trim();
  return trimmed.isEmpty ? null : trimmed;
}

// Links come from the server and are opened by the app, so only absolute
// https links are accepted; anything else is treated as absent.
String? _httpsUrl(Object? value) {
  final text = _text(value);
  if (text == null) return null;
  final uri = Uri.tryParse(text);
  if (uri == null || uri.scheme != 'https' || uri.host.isEmpty) return null;
  return text;
}

/// The public app configuration answered by `GET /app/config`.
///
/// Parsed by hand rather than generated: every value may be null or of an
/// unexpected type, and none of that may crash the app or lock anyone out.
class AppConfig {
  const AppConfig({
    this.minVersionAndroid,
    this.minVersionIos,
    this.termsUrl,
    this.privacyUrl,
    this.supportEmail,
    this.maintenanceActive = false,
    this.maintenanceMessage,
  });

  factory AppConfig.fromJson(Map<String, dynamic> json) {
    final min = json['min_version'];
    final maintenance = json['maintenance'];
    return AppConfig(
      minVersionAndroid: min is Map ? _text(min['android']) : null,
      minVersionIos: min is Map ? _text(min['ios']) : null,
      termsUrl: _httpsUrl(json['terms_url']),
      privacyUrl: _httpsUrl(json['privacy_url']),
      supportEmail: _text(json['support_email']),
      maintenanceActive: maintenance is Map && maintenance['active'] == true,
      maintenanceMessage: maintenance is Map
          ? _text(maintenance['message'])
          : null,
    );
  }

  /// Nothing known: no minimum version, no maintenance, no links.
  static const empty = AppConfig();

  final String? minVersionAndroid;
  final String? minVersionIos;
  final String? termsUrl;
  final String? privacyUrl;
  final String? supportEmail;
  final bool maintenanceActive;
  final String? maintenanceMessage;

  /// This config as a fallback: maintenance is a live, transient server state,
  /// so it is never carried through a failed fetch. A stored minimum version
  /// stands (versions only go up).
  AppConfig withoutMaintenance() => AppConfig(
    minVersionAndroid: minVersionAndroid,
    minVersionIos: minVersionIos,
    termsUrl: termsUrl,
    privacyUrl: privacyUrl,
    supportEmail: supportEmail,
  );

  /// The same shape the server sends, so a stored copy reads back through
  /// [AppConfig.fromJson].
  Map<String, Object?> toJson() => {
    'min_version': {'android': minVersionAndroid, 'ios': minVersionIos},
    'terms_url': termsUrl,
    'privacy_url': privacyUrl,
    'support_email': supportEmail,
    'maintenance': {'active': maintenanceActive, 'message': maintenanceMessage},
  };
}

/// Fetches the app configuration and remembers the last good answer.
///
/// The stored copy holds nothing personal (versions, public links, a
/// maintenance notice), so plain `shared_preferences` is appropriate.
class AppConfigRepository {
  AppConfigRepository(this._api, this._prefs);

  final ApiClient _api;
  final SharedPreferences _prefs;

  /// The last answer the server gave, or null if there has been none (or the
  /// stored copy is unreadable).
  AppConfig? get lastGood {
    final raw = _prefs.getString(_cacheKey);
    if (raw == null) return null;
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map<String, dynamic>) return AppConfig.fromJson(decoded);
    } on FormatException {
      // fall through: an unreadable copy is the same as none
    }
    return null;
  }

  /// Asks the server; on success the answer replaces the stored one. On any
  /// failure (offline, timeout, 5xx, a malformed body) returns the last good
  /// answer, or [AppConfig.empty]. A failure never blocks anyone.
  Future<AppConfig> load() async {
    try {
      final body = await _api.get('/app/config', auth: false);
      if (body is Map<String, dynamic>) {
        final config = AppConfig.fromJson(body);
        await _store(config);
        return config;
      }
    } on ApiError {
      // fall through to the last good answer
    }
    return lastGood?.withoutMaintenance() ?? AppConfig.empty;
  }

  Future<void> _store(AppConfig config) async {
    try {
      await _prefs.setString(_cacheKey, jsonEncode(config.toJson()));
    } on Exception {
      // Not being able to remember the answer must not lose the answer.
    }
  }
}

final appConfigRepositoryProvider = FutureProvider<AppConfigRepository>((
  ref,
) async {
  final prefs = await SharedPreferences.getInstance();
  return AppConfigRepository(ref.watch(apiClientProvider), prefs);
});

/// The app configuration. Fetched once at launch; [AppConfigNotifier.refresh]
/// fetches again. Never ends in an error: a failure keeps the last good
/// answer, or falls back to [AppConfig.empty].
class AppConfigNotifier extends AsyncNotifier<AppConfig> {
  @override
  Future<AppConfig> build() => _fetch(AppConfig.empty);

  Future<AppConfig> _fetch(AppConfig fallback) async {
    try {
      final repository = await ref.read(appConfigRepositoryProvider.future);
      return await repository.load();
    } on Exception {
      // Could not even open local storage: fail open.
      return fallback;
    }
  }

  /// Fetches again, keeping the current value in place meanwhile.
  Future<void> refresh() async {
    final next = await _fetch(
      state.value?.withoutMaintenance() ?? AppConfig.empty,
    );
    if (ref.mounted) state = AsyncData(next);
  }
}

final appConfigProvider = AsyncNotifierProvider<AppConfigNotifier, AppConfig>(
  AppConfigNotifier.new,
);
