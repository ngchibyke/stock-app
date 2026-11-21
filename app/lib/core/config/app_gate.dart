import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';

import 'app_config.dart';

/// What the app may do right now.
enum Gate { open, updateRequired, maintenance }

final _versionPattern = RegExp(r'^\d+\.\d+\.\d+$');

/// Parses a plain `MAJOR.MINOR.PATCH`; null for anything else.
List<int>? _parseVersion(String? value) {
  if (value == null || !_versionPattern.hasMatch(value)) return null;
  final parts = value.split('.').map(int.tryParse).toList();
  if (parts.any((p) => p == null)) return null;
  return parts.cast<int>();
}

/// Compares two versions numerically, part by part: negative when [a] is
/// older than [b], zero when equal, positive when newer. Non-numeric or
/// missing parts count as 0.
int compareVersions(String a, String b) {
  List<int> parts(String v) =>
      v.split('.').map((p) => int.tryParse(p.trim()) ?? 0).toList();
  final left = parts(a);
  final right = parts(b);
  final length = left.length > right.length ? left.length : right.length;
  for (var i = 0; i < length; i++) {
    final l = i < left.length ? left[i] : 0;
    final r = i < right.length ? right[i] : 0;
    if (l != r) return l < r ? -1 : 1;
  }
  return 0;
}

/// Decides whether the app is usable. Maintenance wins over everything. A
/// missing or malformed minimum, a malformed running version or an unknown
/// platform all mean [Gate.open]: when in doubt, nobody is locked out.
Gate evaluateGate(
  AppConfig config, {
  required String platform,
  required String version,
}) {
  if (config.maintenanceActive) return Gate.maintenance;
  final minimum = switch (platform) {
    'android' => config.minVersionAndroid,
    'ios' => config.minVersionIos,
    _ => null,
  };
  final minimumParts = _parseVersion(minimum);
  if (minimumParts == null || _parseVersion(version) == null) return Gate.open;
  return compareVersions(version, minimum!) < 0
      ? Gate.updateRequired
      : Gate.open;
}

/// The running app's version (`MAJOR.MINOR.PATCH`, build number excluded).
/// Tests override this.
final packageVersionProvider = FutureProvider<String>((ref) async {
  final info = await PackageInfo.fromPlatform();
  return info.version;
});

/// `'android'` or `'ios'`. Tests override this.
final appPlatformProvider = Provider<String>(
  (ref) => defaultTargetPlatform == TargetPlatform.iOS ? 'ios' : 'android',
);

/// The current gate. Open while the config or the version is still loading,
/// and if either could not be obtained.
final gateProvider = Provider<Gate>((ref) {
  final config = ref.watch(appConfigProvider).value;
  final version = ref.watch(packageVersionProvider).value;
  if (config == null || version == null) return Gate.open;
  return evaluateGate(
    config,
    platform: ref.watch(appPlatformProvider),
    version: version,
  );
});

/// Calls [onForeground] each time the app returns to the foreground.
class AppForegroundRefresher with WidgetsBindingObserver {
  AppForegroundRefresher(this.onForeground);

  final VoidCallback onForeground;

  void attach() => WidgetsBinding.instance.addObserver(this);
  void detach() => WidgetsBinding.instance.removeObserver(this);

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) onForeground();
  }
}

/// Refetches the app config whenever the app returns to the foreground. Read
/// it once near the root (the router does this) to switch it on.
final appForegroundRefresherProvider = Provider<AppForegroundRefresher>((ref) {
  final refresher = AppForegroundRefresher(
    () => ref.read(appConfigProvider.notifier).refresh(),
  )..attach();
  ref.onDispose(refresher.detach);
  return refresher;
});
